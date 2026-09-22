//! Host-file ⇄ device-bytes conversion shared by every transfer target (serial `get`/`put`
//! today, disk images next). Pure buffer-in/buffer-out: no serial port, no filesystem, so
//! it is available without the `serial` feature and to the C ABI.
//!
//! This is the single place that decides what `--format`, `--raw`, `--skip-header` and
//! `--start-address`/`--run-address` mean, so the targets cannot drift apart.

use anyhow::{bail, Result};

use crate::detect::Content;
use crate::detokenize::{self, LineEnding};
use crate::header::{self, FileType, ParsedHeader};
use crate::registry::{Device, Registry};
use crate::scanner::SegmentMarker;
use crate::{filename, text};

/// `--format`: the host-side representation asked for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Ascii,
    Binary,
}

/// Where the bytes go. Serial `put` sends to a live Pocket Computer (PC-1500 or
/// PC-1600 family); `Disk` stores a file on a PC-1600 floppy, where the rules are
/// stricter (see [`build_put`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Serial,
    Disk,
}

/// Low 16 bits of a PC-1600 header's run address meaning "no auto-start" (`BSAVE`'s
/// default `&FFFF`). The bank byte is taken from the load address; a bank-0 `BSAVE` in
/// Calc-U-1600 writes exactly this header (`… C5 C0 00 FF FF 00 …`).
pub const PC1600_NO_AUTORUN: u32 = 0xFFFF;

/// What a `put` source is turned into before it reaches the target.
pub struct PutSpec<'a> {
    /// Host path (or name) of the source; supplies the synthesized header's filename.
    pub source_name: &'a str,
    /// Keyword table + header flavor.
    pub device: Device,
    pub format: Option<Format>,
    pub start_address: Option<u32>,
    pub run_address: Option<u32>,
    pub raw: bool,
    pub endpoint: Endpoint,
}

/// How [`build_put`] arrived at its bytes (for narration).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PutKind {
    /// Input already carried a recognized header; sent unmodified.
    AsIs,
    TokenizedBasic,
    /// A BASIC listing stored as a PC-1600 ASCII file (`SAVE "…",A` form).
    AsciiListing,
    Reserve,
    Variables,
    /// Headerless machine language wrapped in a synthesized header (`--start-address`).
    MachineWrapped,
    /// Host text converted to device text (CP437, CRLF, trailing `1A`).
    Text,
    /// `--raw`: sent unmodified without a header.
    Raw,
}

#[derive(Debug)]
pub struct PutBytes {
    pub bytes: Vec<u8>,
    /// Leading slice treated as "the header" by a paced send; `0` for headerless.
    pub header_len: usize,
    pub kind: PutKind,
}

/// Build the exact bytes for a `put`, per requirements §5 step 5 / §3's header-auto-add
/// rules. For [`Endpoint::Disk`] see [`build_disk_put`].
pub fn build_put(
    raw: &[u8],
    header: Option<&ParsedHeader>,
    content: Content,
    spec: &PutSpec,
) -> Result<PutBytes> {
    if spec.endpoint == Endpoint::Disk {
        return build_disk_put(raw, header, content, spec);
    }
    if let Some(h) = header {
        // Already has a recognized header -- send as-is, unmodified.
        return Ok(PutBytes { bytes: raw.to_vec(), header_len: h.offset + h.header_len, kind: PutKind::AsIs });
    }

    let forced_machine = spec.start_address.is_some();

    if !forced_machine && (content == Content::AsciiBasic || forced_basic(content, spec)) {
        // ASCII BASIC input is always tokenized here (the caller handles an explicit
        // `--format ascii` line-by-line send); `--format binary` also forces text that
        // detection did not take for BASIC to be tokenized.
        let name = filename::synth_basename(spec.source_name);
        let bytes = tokenize_forced(raw, spec.device, Some(&name), SegmentMarker::Wire, content)?;
        let header_len = header::find(&bytes).expect("tokenize_listing wraps a header").header_len;
        return Ok(PutBytes { bytes, header_len, kind: PutKind::TokenizedBasic });
    }

    if !forced_machine && content == Content::Ce158Reserve {
        let text = String::from_utf8_lossy(raw);
        let layout = crate::reserve::from_ascii(&text)?;
        let reg = Registry::for_device(spec.device);
        let payload = crate::reserve::encode_payload(&layout, reg)?;
        let name = layout.filename.clone().unwrap_or_else(|| filename::synth_basename(spec.source_name));
        return Ok(wrap(spec.device, FileType::Reserve, &name, &payload, 0, 0, PutKind::Reserve));
    }

    if !forced_machine && content == Content::Ce158Variables {
        let text = String::from_utf8_lossy(raw);
        let file = crate::variables::from_ascii(&text)?;
        let payload = crate::variables::encode_payload(&file.values)?;
        let name = file.filename.clone().unwrap_or_else(|| filename::synth_basename(spec.source_name));
        // build_header always writes the Variables wire length as 0 regardless of
        // payload_len, matching the device's own wire behavior for this type.
        return Ok(wrap(spec.device, FileType::Variables, &name, &payload, 0, 0, PutKind::Variables));
    }

    if !forced_machine && !spec.raw && content == Content::Text && spec.format != Some(Format::Binary) {
        if spec.device != Device::Pc1600 {
            bail!(
                "plain text transfer is only supported for the PC-1600 family \
                 (use --raw to send the file unmodified)"
            );
        }
        let text = decode_host_text(raw)?;
        return Ok(PutBytes { bytes: encode_text(&text)?, header_len: 0, kind: PutKind::Text });
    }

    // No header, not ASCII BASIC/Reserve/Variables/text -- a machine-language candidate.
    if let Some(start) = spec.start_address {
        let run = spec.run_address.unwrap_or(0xFFFF);
        let name = filename::synth_basename(spec.source_name);
        return Ok(wrap(spec.device, FileType::Machine, &name, raw, start, run, PutKind::MachineWrapped));
    }

    if spec.raw {
        return Ok(PutBytes { bytes: raw.to_vec(), header_len: 0, kind: PutKind::Raw });
    }

    bail!(
        "headerless machine-language input needs --start-address (or --raw to send it \
         completely unmodified)"
    )
}

/// The bytes of a file stored on a PC-1600 floppy (the device is always PC-1600):
///
/// - `--raw`: unchanged, whatever it is;
/// - PC-1600 header: unchanged (`--start-address`/`--run-address` are an error);
/// - CE-158 header, Reserve, Variables: error — PC-1500 data;
/// - `--start-address`: headerless data wrapped in a machine-language header;
/// - BASIC listing: tokenized behind a BASIC header, or with `--format ascii` stored as
///   an ASCII program file;
/// - text: CP437, CRLF, trailing `1A`;
/// - anything else: error (needs `--start-address` or `--raw`).
fn build_disk_put(
    raw: &[u8],
    header: Option<&ParsedHeader>,
    content: Content,
    spec: &PutSpec,
) -> Result<PutBytes> {
    if spec.raw {
        return Ok(PutBytes { bytes: raw.to_vec(), header_len: 0, kind: PutKind::Raw });
    }
    if let Some(h) = header {
        if h.device != Device::Pc1600 {
            bail!(
                "file has a PC-1500 (CE-158) header; the floppy is PC-1600 only. Convert a BASIC \
                 program with `sde convert` to a listing first, or use --raw to store it unchanged"
            );
        }
        if spec.start_address.is_some() || spec.run_address.is_some() {
            bail!("file already has a PC-1600 header; --start-address/--run-address cannot override it");
        }
        return Ok(PutBytes { bytes: raw.to_vec(), header_len: h.payload_start(), kind: PutKind::AsIs });
    }
    if let Some(start) = spec.start_address {
        if start > 0xFF_FFFF {
            bail!("--start-address {start:#X} does not fit in 24 bits (bank + 16-bit address)");
        }
        let run = spec.run_address.unwrap_or((start & 0xFF_0000) | PC1600_NO_AUTORUN);
        if run > 0xFF_FFFF {
            bail!("--run-address {run:#X} does not fit in 24 bits (bank + 16-bit address)");
        }
        let name = filename::synth_basename(spec.source_name);
        return Ok(wrap(Device::Pc1600, FileType::Machine, &name, raw, start, run, PutKind::MachineWrapped));
    }
    match content {
        Content::AsciiBasic if spec.format == Some(Format::Ascii) => {
            Ok(PutBytes { bytes: encode_ascii_listing(raw)?, header_len: 0, kind: PutKind::AsciiListing })
        }
        _ if content == Content::AsciiBasic || forced_basic(content, spec) => {
            // A disk file holds the program area as the ROM saves it, so `#SEGMENT`
            // markers take their in-memory form. (Not yet checked against a ROM SAVE of
            // a segmented program; plain programs re-save byte-identically.)
            let bytes = tokenize_forced(raw, Device::Pc1600, None, SegmentMarker::Memory, content)?;
            Ok(PutBytes { header_len: 16, bytes, kind: PutKind::TokenizedBasic })
        }
        Content::Text => {
            let text = decode_host_text(raw)?;
            Ok(PutBytes { bytes: encode_text(&text)?, header_len: 0, kind: PutKind::Text })
        }
        Content::Ce158Reserve | Content::Ce158Variables => {
            bail!("{} files are PC-1500 only and cannot be stored on a PC-1600 floppy", content.describe())
        }
        _ => bail!(
            "binary input without a header needs --start-address (to store it as a machine-language \
             file) or --raw (to store it unchanged)"
        ),
    }
}

/// What kind of file a floppy file is, from its content (the directory does not say).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskFileKind {
    /// Tokenized BASIC behind a PC-1600 header; `len` = payload bytes.
    Basic { len: u32 },
    /// Machine language behind a PC-1600 header.
    Machine { load: u32, run: u32, len: u32 },
    /// A BASIC program saved as ASCII (`SAVE "…",A`).
    AsciiBasic,
    /// Any other headerless ASCII file.
    Text,
    Unknown,
}

pub fn classify_disk_file(bytes: &[u8]) -> DiskFileKind {
    match header::find(bytes) {
        Some(h) if h.offset == 0 && h.device == Device::Pc1600 => match h.file_type {
            FileType::Machine => {
                DiskFileKind::Machine { load: h.start_addr, run: h.run_addr, len: h.length as u32 }
            }
            _ => DiskFileKind::Basic { len: h.length as u32 },
        },
        Some(_) => DiskFileKind::Unknown,
        None if crate::detect::detect(bytes) == Content::AsciiBasic => DiskFileKind::AsciiBasic,
        None if looks_like_device_text(bytes) => DiskFileKind::Text,
        None => DiskFileKind::Unknown,
    }
}

/// Extension for a file stored on the floppy under the host file's stem: `BAS` for
/// BASIC in either form, otherwise the host extension (upper-cased) if it is a valid
/// 1–3 character extension, else `BIN` for machine language and none for the rest.
pub fn disk_ext_for(kind: PutKind, bytes: &[u8], host_ext: Option<&str>) -> String {
    let host = host_ext
        .map(str::to_ascii_uppercase)
        .filter(|e| !e.is_empty() && e.len() <= 3 && crate::diskfs::FileName::parse(&format!("X.{e}")).is_ok());
    let is_basic = match kind {
        PutKind::TokenizedBasic | PutKind::AsciiListing => true,
        PutKind::AsIs => matches!(classify_disk_file(bytes), DiskFileKind::Basic { .. }),
        _ => false,
    };
    if is_basic {
        return "BAS".into();
    }
    match (kind, host) {
        (_, Some(e)) => e,
        (PutKind::MachineWrapped | PutKind::AsIs, None) => "BIN".into(),
        _ => String::new(),
    }
}

/// `--format binary` on input detection took for plain text: the user says it is a
/// BASIC listing after all (the BASIC test is strict on purpose).
fn forced_basic(content: Content, spec: &PutSpec) -> bool {
    content == Content::Text && spec.format == Some(Format::Binary)
}

fn tokenize_forced(
    raw: &[u8],
    device: Device,
    name: Option<&str>,
    marker: SegmentMarker,
    content: Content,
) -> Result<Vec<u8>> {
    let bytes = crate::convert::tokenize_listing(raw, device, name, true, marker).map_err(|e| {
        if content == Content::AsciiBasic {
            e
        } else {
            e.context("--format binary: the input does not tokenize as a BASIC listing")
        }
    })?;
    // The scanner drops lines without a line number, so input that is no listing at all
    // tokenizes to an empty program.
    if header::find(&bytes).is_some_and(|h| bytes.len() <= h.payload_start()) {
        bail!("--format binary: the input has no numbered BASIC lines");
    }
    Ok(bytes)
}

fn wrap(
    device: Device,
    file_type: FileType,
    name: &str,
    payload: &[u8],
    start_addr: u32,
    run_addr: u32,
    kind: PutKind,
) -> PutBytes {
    let mut bytes = header::build_header(header::BuildHeader {
        device,
        file_type,
        name: Some(name),
        payload_len: payload.len(),
        start_addr,
        run_addr,
    });
    let header_len = bytes.len();
    bytes.extend_from_slice(payload);
    PutBytes { bytes, header_len, kind }
}

/// How a received / stored byte sequence is turned into a host file.
pub struct GetSpec {
    /// `None` = the natural representation per content type.
    pub format: Option<Format>,
    pub skip_header: bool,
    pub eol: LineEnding,
}

#[derive(Debug)]
pub struct Extracted {
    pub bytes: Vec<u8>,
    pub content: Content,
    /// Host extension for this content (without the dot).
    pub ext: &'static str,
    pub header: Option<ParsedHeader>,
    /// Warnings for the caller to print (stderr).
    pub notes: Vec<String>,
}

/// Convert device bytes into host-file bytes per `spec` (requirements §4 steps 3-6).
pub fn extract(raw: &[u8], spec: &GetSpec) -> Result<Extracted> {
    let header = header::find(raw);
    let mut content = crate::detect::detect_from_header(header.as_ref(), raw);
    let file_type = header.as_ref().map(|h| h.file_type);
    let mut notes = Vec::new();

    let format = spec.format.unwrap_or(match file_type {
        Some(FileType::Machine) => Format::Binary,
        Some(_) => Format::Ascii,
        None if content == Content::Unknown && !looks_like_device_text(raw) => Format::Binary,
        None => Format::Ascii,
    });

    if file_type == Some(FileType::Machine) && format == Format::Ascii {
        bail!("machine language cannot be converted to ASCII; use --format binary");
    }

    let bytes = match (format, &header, content) {
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Basic => {
            let payload = &raw[h.payload_start()..];
            let reg = Registry::for_device(h.device);
            detokenize::detokenize_to_text(payload, reg, spec.eol)?.into_bytes()
        }
        // Device bytes are CP437 (never UTF-8), with CR or CRLF line ends and maybe `1A`.
        (Format::Ascii, None, Content::AsciiBasic) => decode_device_text(raw, spec.eol).into_bytes(),
        (Format::Ascii, None, _) if looks_like_device_text(raw) => {
            content = Content::Text;
            decode_device_text(raw, spec.eol).into_bytes()
        }
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Reserve => {
            let payload = &raw[h.payload_start()..h.payload_start() + h.length];
            let reg = Registry::for_device(h.device);
            let mut layout = crate::reserve::decode_payload(payload, reg)?;
            layout.filename = h.filename.clone();
            crate::reserve::to_ascii(&layout).into_bytes()
        }
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Variables => {
            // The header's length field is not meaningful for Variables (see
            // `ParsedHeader::length`); parse to end of the received buffer instead.
            let payload = &raw[h.payload_start()..];
            let mut file = crate::variables::decode_payload(payload)?;
            file.filename = h.filename.clone();
            crate::variables::to_ascii(&file).into_bytes()
        }
        (Format::Ascii, _, _) => {
            bail!("cannot produce an ASCII listing from this content ({})", content.describe());
        }
        (Format::Binary, _, _) => {
            if spec.skip_header {
                match &header {
                    Some(h) => raw[h.payload_start()..].to_vec(),
                    None => {
                        notes.push("--skip-header given but no header was found; saving all bytes".into());
                        raw.to_vec()
                    }
                }
            } else {
                raw.to_vec()
            }
        }
    };

    if spec.skip_header && format == Format::Binary && header.is_some() {
        notes.push(
            "--skip-header omits the header from the saved file. \
             The file cannot be identified or reloaded without it."
                .into(),
        );
    }

    if header.is_none() && content == Content::Unknown && spec.format.is_none() {
        notes.push("unrecognized content, saved unchanged".into());
    }

    let ext = match file_type {
        Some(t) => filename::ext_for(t),
        None if content == Content::Text => "txt",
        None if content == Content::Unknown => "bin",
        None => "bas",
    };
    Ok(Extracted { bytes, content, ext, header, notes })
}

/// Host text bytes (UTF-8, optional BOM) as a `String`. Errors on invalid UTF-8.
pub fn decode_host_text(raw: &[u8]) -> Result<String> {
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    match std::str::from_utf8(raw) {
        Ok(s) => Ok(s.to_string()),
        Err(e) => bail!("text input is not valid UTF-8 (byte offset {})", e.valid_up_to()),
    }
}

/// Encode host text as a PC-1600 ASCII file: CP437, every line ending (LF, CR, CRLF)
/// becomes CRLF, one `1A` end-of-file byte appended (an existing trailing `1A` is not
/// doubled). A character outside the Sharp set is an error naming its position.
pub fn encode_text(text: &str) -> Result<Vec<u8>> {
    let body = text.trim_end_matches('\u{1A}');
    let mut out = Vec::with_capacity(body.len() + 16);
    let (mut line, mut col) = (1usize, 1usize);
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.extend_from_slice(b"\r\n");
                line += 1;
                col = 1;
                continue;
            }
            _ => match crate::cp437::to_byte(c) {
                Some(b) => out.push(b),
                None => bail!(
                    "line {line}, column {col}: U+{:04X} '{c}' has no equivalent in the Sharp character set",
                    c as u32
                ),
            },
        }
        col += 1;
    }
    out.push(0x1A);
    Ok(out)
}

/// Encode a BASIC listing as a PC-1600 ASCII program file (what `SAVE "…",A` writes).
pub fn encode_ascii_listing(raw: &[u8]) -> Result<Vec<u8>> {
    encode_text(&text::decode_bas_listing(raw))
}

/// True if device bytes look like an ASCII (headerless) file: up to the first `1A`,
/// non-empty and free of control bytes other than TAB/LF/CR. CP437 high bytes are fine.
pub fn looks_like_device_text(bytes: &[u8]) -> bool {
    let body = device_text_body(bytes);
    body.iter().any(|b| !b.is_ascii_whitespace())
        && body.first() != Some(&0xFF)
        && body.iter().all(|&b| b >= 0x20 || matches!(b, 0x09 | 0x0A | 0x0D))
}

/// Decode a device ASCII file to host text: CP437 → UTF-8, cut at the first `1A`, every
/// CRLF / CR / LF becomes `eol`.
pub fn decode_device_text(bytes: &[u8], eol: LineEnding) -> String {
    let text = crate::cp437::decode(device_text_body(bytes));
    apply_eol(&normalize_newlines(&text), eol)
}

fn device_text_body(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|&b| b == 0x1A) {
        Some(i) => &bytes[..i],
        None => bytes,
    }
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// `s` has LF line endings; rewrite them to `eol`.
fn apply_eol(s: &str, eol: LineEnding) -> String {
    match eol.as_str() {
        "\n" => s.to_string(),
        other => s.replace('\n', other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_text_converts_charset_and_line_endings() {
        assert_eq!(encode_text("A\u{e4}\nB\r\nC\rD").unwrap(), b"A\x84\r\nB\r\nC\r\nD\x1A");
        assert_eq!(encode_text("\u{fc}").unwrap(), vec![0x81, 0x1A]);
        // An existing EOF byte is not doubled.
        assert_eq!(encode_text("X\r\n\u{1A}").unwrap(), b"X\r\n\x1A");
    }

    #[test]
    fn encode_text_rejects_unmappable() {
        let err = encode_text("ok\nsnow \u{2603}").unwrap_err().to_string();
        assert!(err.contains("line 2, column 6"), "{err}");
    }

    #[test]
    fn decode_device_text_cuts_at_eof() {
        let bytes = b"A\x84\r\nB\r\n\x1Agarbage";
        assert_eq!(decode_device_text(bytes, LineEnding::Lf), "A\u{e4}\nB\n");
        assert_eq!(decode_device_text(bytes, LineEnding::CrLf), "A\u{e4}\r\nB\r\n");
        assert!(looks_like_device_text(bytes));
        assert!(!looks_like_device_text(&[0x00, 0x0A, 0x01, 0x0D]));
        assert!(!looks_like_device_text(b"\x1A"));
    }

    #[test]
    fn text_roundtrip() {
        let host = "; Z80\n\tLD A,\u{e9}\n";
        let dev = encode_text(host).unwrap();
        assert_eq!(decode_device_text(&dev, LineEnding::Lf), host);
    }

    fn put_spec(device: Device) -> PutSpec<'static> {
        PutSpec {
            source_name: "x.asm",
            device,
            format: None,
            start_address: None,
            run_address: None,
            raw: false,
            endpoint: Endpoint::Serial,
        }
    }

    #[test]
    fn put_text_pc1600_and_pc1500() {
        let out = build_put(b"LD A,0\n", None, Content::Text, &put_spec(Device::Pc1600)).unwrap();
        assert_eq!(out.kind, PutKind::Text);
        assert_eq!(out.bytes, b"LD A,0\r\n\x1A");
        assert!(build_put(b"LD A,0\n", None, Content::Text, &put_spec(Device::Pc1500)).is_err());
        let mut raw = put_spec(Device::Pc1500);
        raw.raw = true;
        assert_eq!(build_put(b"LD\n", None, Content::Text, &raw).unwrap().kind, PutKind::Raw);
    }

    #[test]
    fn extract_device_text() {
        let spec = GetSpec { format: Some(Format::Ascii), skip_header: false, eol: LineEnding::Lf };
        let out = extract(b"ORG 0\r\n\x81\r\n\x1A", &spec).unwrap();
        assert_eq!(out.content, Content::Text);
        assert_eq!(out.ext, "txt");
        assert_eq!(out.bytes, "ORG 0\n\u{fc}\n".as_bytes());
    }

    fn disk_spec(source: &'static str) -> PutSpec<'static> {
        PutSpec { endpoint: Endpoint::Disk, source_name: source, ..put_spec(Device::Pc1600) }
    }

    fn disk_put(raw: &[u8], spec: &PutSpec) -> Result<PutBytes> {
        let h = header::find(raw);
        build_put(raw, h.as_ref(), crate::detect::detect_from_header(h.as_ref(), raw), spec)
    }

    #[test]
    fn disk_put_basic_listing() {
        let out = disk_put(b"10 PRINT \"HI\"\n", &disk_spec("hi.bas")).unwrap();
        assert_eq!(out.kind, PutKind::TokenizedBasic);
        assert_eq!(&out.bytes[..5], &[0xFF, 0x10, 0x00, 0x00, 0x21]);
        assert_eq!(classify_disk_file(&out.bytes), DiskFileKind::Basic { len: out.bytes.len() as u32 - 16 });
        assert_eq!(disk_ext_for(out.kind, &out.bytes, Some("txt")), "BAS");

        let mut ascii = disk_spec("hi.bas");
        ascii.format = Some(Format::Ascii);
        let out = disk_put(b"10 PRINT \"HI\"\n", &ascii).unwrap();
        assert_eq!(out.kind, PutKind::AsciiListing);
        assert_eq!(out.bytes, b"10 PRINT \"HI\"\r\n\x1A");
        assert_eq!(classify_disk_file(&out.bytes), DiskFileKind::AsciiBasic);
    }

    #[test]
    fn disk_put_headers_and_binaries() {
        let mut p16 = header::build(Device::Pc1600, None, 4);
        p16.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        assert_eq!(disk_put(&p16, &disk_spec("x.bbin")).unwrap().bytes, p16);

        let mut with_start = disk_spec("x.bbin");
        with_start.start_address = Some(0x1000);
        assert!(disk_put(&p16, &with_start).unwrap_err().to_string().contains("already has a PC-1600 header"));

        let mut ce = header::build(Device::Pc1500, Some("x"), 4);
        ce.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        assert!(disk_put(&ce, &disk_spec("x.bin")).unwrap_err().to_string().contains("sde convert"));
        let mut raw = disk_spec("x.bin");
        raw.raw = true;
        assert_eq!(disk_put(&ce, &raw).unwrap().kind, PutKind::Raw);

        let blob = [0xC3u8, 0x00, 0x00, 0x01];
        assert!(disk_put(&blob, &disk_spec("mc.bin")).unwrap_err().to_string().contains("--start-address"));
        let mut mc = disk_spec("mc.bin");
        mc.start_address = Some(0x01_C000);
        let out = disk_put(&blob, &mc).unwrap();
        assert_eq!(out.kind, PutKind::MachineWrapped);
        assert_eq!(
            &out.bytes[..16],
            &[0xFF, 0x10, 0x00, 0x00, 0x10, 4, 0, 0, 0x00, 0xC0, 0x01, 0xFF, 0xFF, 0x01, 0x00, 0x0F]
        );
        assert_eq!(classify_disk_file(&out.bytes), DiskFileKind::Machine { load: 0x01_C000, run: 0x01_FFFF, len: 4 });
        assert_eq!(disk_ext_for(out.kind, &out.bytes, Some("bin")), "BIN");
        assert_eq!(disk_ext_for(out.kind, &out.bytes, Some("toolong")), "BIN");
        mc.start_address = Some(0x100_0000);
        assert!(disk_put(&blob, &mc).is_err());

        let sdav = format!("{}1.0 pc1500\n; Count: 1\n42\n", crate::variables::MARKER);
        assert!(disk_put(sdav.as_bytes(), &disk_spec("v.sdav")).unwrap_err().to_string().contains("PC-1500 only"));
    }

    #[test]
    fn disk_put_text() {
        let out = disk_put("ORG 0\n; \u{e4}\n".as_bytes(), &disk_spec("p.asm")).unwrap();
        assert_eq!(out.kind, PutKind::Text);
        assert_eq!(out.bytes, b"ORG 0\r\n; \x84\r\n\x1A");
        assert_eq!(disk_ext_for(out.kind, &out.bytes, Some("asm")), "ASM");
        assert_eq!(disk_ext_for(out.kind, &out.bytes, None), "");
    }

    #[test]
    fn extract_disk_defaults() {
        let auto = |eol| GetSpec { format: None, skip_header: false, eol };
        // BASIC -> listing
        let mut p16 = header::build(Device::Pc1600, None, 4);
        p16.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        let out = extract(&p16, &auto(LineEnding::CrLf)).unwrap();
        assert_eq!(out.bytes, b"10\r\n");
        assert_eq!(out.ext, "bas");
        // BASIC, binary, header kept / skipped
        let bin = extract(&p16, &GetSpec { format: Some(Format::Binary), ..auto(LineEnding::Lf) }).unwrap();
        assert_eq!(bin.bytes, p16);
        let bare = extract(&p16, &GetSpec { format: Some(Format::Binary), skip_header: true, eol: LineEnding::Lf })
            .unwrap();
        assert_eq!(bare.bytes, &p16[16..]);
        // Machine: header kept by default; explicit ascii is an error.
        let mc = build_put(&[1, 2, 3], None, Content::Unknown, &PutSpec { start_address: Some(0x2000), ..disk_spec("m") })
            .unwrap()
            .bytes;
        assert_eq!(extract(&mc, &auto(LineEnding::Lf)).unwrap().bytes, mc);
        assert!(extract(&mc, &GetSpec { format: Some(Format::Ascii), ..auto(LineEnding::Lf) }).is_err());
        // Unknown -> unchanged, with a note.
        let blob = [0x00u8, 0x01, 0x02];
        let out = extract(&blob, &auto(LineEnding::Lf)).unwrap();
        assert_eq!(out.bytes, blob);
        assert_eq!(out.ext, "bin");
        assert_eq!(out.notes, vec!["unrecognized content, saved unchanged".to_string()]);
        // ASCII-saved BASIC listing -> listing with host line endings, 1A stripped.
        let out = extract(b"10 PRINT\r\n20 END\r\n\x1A", &auto(LineEnding::Lf)).unwrap();
        assert_eq!(out.bytes, b"10 PRINT\n20 END\n");
    }

    #[test]
    fn format_binary_forces_tokenizing_text() {
        let pun = b"                 50                     1 MARTIN\r\n\x1A";
        assert_eq!(crate::detect::detect(pun), Content::Text);
        // Default: stored as text, byte for byte.
        assert_eq!(disk_put(pun, &disk_spec("atlantis.pun")).unwrap().bytes, pun);
        // -f binary: tokenized as BASIC (line 50), on disk and over serial.
        let forced = PutSpec { format: Some(Format::Binary), ..disk_spec("atlantis.pun") };
        let out = disk_put(pun, &forced).unwrap();
        assert_eq!(out.kind, PutKind::TokenizedBasic);
        assert_eq!(&out.bytes[16..18], &[0x00, 50]);
        let serial = PutSpec { format: Some(Format::Binary), ..put_spec(Device::Pc1600) };
        assert_eq!(disk_put(pun, &serial).unwrap().kind, PutKind::TokenizedBasic);
        // Text that is no listing at all is an error naming the override.
        let err = disk_put(b"hello world\n", &forced).unwrap_err();
        assert!(format!("{err:#}").contains("--format binary"), "{err:#}");
        let pc1500 = PutSpec { format: Some(Format::Binary), ..put_spec(Device::Pc1500) };
        assert!(disk_put(b"hello world\n", &pc1500).is_err());
    }
}
