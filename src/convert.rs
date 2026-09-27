//! Pure, content-driven conversion — the shared core behind the CLI and the C ABI.
//! No filesystem access. Mirrors `SharpDataExchange.runConvert` / `encodeAsciiBasic`.

use anyhow::{bail, Result};

use crate::detect::{self, Content};
use crate::detokenize::LineEnding;
use crate::registry::{Device, Registry};
use crate::scanner::SegmentMarker;
use crate::{abbrev, detokenize, header, scanner, text};

/// Result of a conversion.
#[derive(Debug)]
pub struct ConvertOutcome {
    /// The output bytes: a header + tokenized payload when tokenizing (unless
    /// `with_header` was false), or a UTF-8 ASCII listing when de-tokenizing.
    pub bytes: Vec<u8>,
    /// What the input was detected as.
    pub content: Content,
    /// The device actually used (taken from the input header when de-tokenizing).
    pub device: Device,
}

/// Convert `input`, choosing direction from its detected content.
///
/// * ASCII BASIC in  -> tokenized payload out (prefixed with the CE-158 / PC-1600
///   serial header when `with_header`). `device` selects the keyword table + header;
///   `name` supplies the CE-158 filename.
/// * Tokenized BASIC in (with a CE-158 / PC-1600 header) -> ASCII listing out. The
///   device is taken from the header; `device` and `name` are ignored.
///
/// A de-tokenized listing is terminated with the host-default line ending (`\r\n` on
/// Windows, `\n` elsewhere); use [`convert_with`] to override it. `CR` / `CRLF` input to
/// a tokenize is always accepted regardless of platform.
///
/// A `#SEGMENT` marker line tokenizes to its wire form ([`SegmentMarker::Wire`]) --
/// what an actual `SAVE "COM1:"` transmits. Use [`convert_with`] for
/// [`SegmentMarker::Memory`] when building bytes to poke directly into RAM instead.
pub fn convert(
    input: &[u8],
    device: Device,
    name: Option<&str>,
    with_header: bool,
) -> Result<ConvertOutcome> {
    convert_with(input, device, name, with_header, LineEnding::Platform, SegmentMarker::Wire)
}

/// Tokenize `input` as an ASCII BASIC listing without first detecting its content --
/// for a caller that knows better than [`crate::detect`] (e.g. `put -f binary` on input
/// that looked like plain text). A line that is not valid BASIC is an error.
pub fn tokenize_listing(
    input: &[u8],
    device: Device,
    name: Option<&str>,
    with_header: bool,
    segment_marker: SegmentMarker,
) -> Result<Vec<u8>> {
    let listing = text::decode_bas_listing(input);
    text::require_ascii_for_pc1500(&listing, device)?;
    let reg = Registry::for_device(device);
    let expanded = expand_all(&listing, reg);
    let payload = scanner::tokenize(&expanded, reg, segment_marker)?;
    Ok(if with_header {
        let mut out = header::build(device, name, payload.len());
        out.extend_from_slice(&payload);
        out
    } else {
        payload
    })
}

/// As [`convert`], but with an explicit [`LineEnding`] for a de-tokenized listing and
/// an explicit [`SegmentMarker`] style for a `#SEGMENT` line when tokenizing (ignored
/// when de-tokenizing). When tokenizing, `eol` is unused (`CR` / `CRLF` input is
/// always accepted).
pub fn convert_with(
    input: &[u8],
    device: Device,
    name: Option<&str>,
    with_header: bool,
    eol: LineEnding,
    segment_marker: SegmentMarker,
) -> Result<ConvertOutcome> {
    let content = detect::detect(input);
    match content {
        Content::AsciiBasic => {
            let bytes = tokenize_listing(input, device, name, with_header, segment_marker)?;
            Ok(ConvertOutcome { bytes, content, device })
        }
        Content::Ce158Basic | Content::Pc1600Basic => {
            let h = header::find(input).expect("detect() guarantees a header here");
            let payload = &input[h.payload_start()..];
            let reg = Registry::for_device(h.device);
            let listing = detokenize::detokenize_to_text(payload, reg, eol)?;
            Ok(ConvertOutcome { bytes: listing.into_bytes(), content, device: h.device })
        }
        Content::Ce158Machine
        | Content::Pc1600Machine
        | Content::Ce158Reserve
        | Content::Ce158Variables
        | Content::Text
        | Content::Unknown => bail!(
            "convert only handles BASIC; got {}. A tokenized file must include a CE-158 or PC-1600 header.",
            content.describe()
        ),
    }
}

/// Wrap a headerless machine-language `payload` in a CE-158 (PC-1500 family) or
/// PC-1600 MACHINE header. `name` supplies the CE-158 filename (unused for PC-1600).
///
/// Addresses are 16-bit for the PC-1500 and 24-bit (bank in the top byte) for the
/// PC-1600. Without `run_addr` the file does not auto-start: `0xFFFF` on the PC-1500,
/// the load address's bank with `FFFF` on the PC-1600 (what `BSAVE` writes).
pub fn add_machine_header(
    payload: &[u8],
    device: Device,
    name: Option<&str>,
    start_addr: u32,
    run_addr: Option<u32>,
) -> Result<Vec<u8>> {
    if payload.is_empty() {
        bail!("machine-language payload is empty");
    }
    let (max, what) = match device {
        Device::Pc1500 => (0xFFFF, "16 bits (PC-1500 address)"),
        Device::Pc1600 => (0xFF_FFFF, "24 bits (bank + 16-bit address)"),
    };
    let run_addr = run_addr.unwrap_or(match device {
        Device::Pc1500 => 0xFFFF,
        Device::Pc1600 => (start_addr & 0xFF_0000) | crate::transfer::PC1600_NO_AUTORUN,
    });
    if start_addr > max {
        bail!("start address {start_addr:#X} does not fit in {what}");
    }
    if run_addr > max {
        bail!("run address {run_addr:#X} does not fit in {what}");
    }
    let max_len = match device {
        Device::Pc1500 => 0x1_0000,
        Device::Pc1600 => 0xFF_FFFF,
    };
    if payload.len() > max_len {
        bail!("machine-language payload of {} bytes is too large for a {device:?} header", payload.len());
    }
    let mut out = header::build_header(header::BuildHeader {
        device,
        file_type: header::FileType::Machine,
        name,
        payload_len: payload.len(),
        start_addr,
        run_addr,
    });
    out.extend_from_slice(payload);
    Ok(out)
}

/// A machine-language file split into its header and payload by [`strip_machine_header`].
#[derive(Debug)]
pub struct StrippedMachine<'a> {
    pub header: header::ParsedHeader,
    /// The `header.length` payload bytes.
    pub payload: &'a [u8],
    /// Bytes after the payload (e.g. capture noise); dropped from `payload`.
    pub trailing: usize,
}

/// Split `input`, which must carry a CE-158 or PC-1600 MACHINE header, into header and
/// payload. A file shorter than the header's recorded length is an error.
pub fn strip_machine_header(input: &[u8]) -> Result<StrippedMachine<'_>> {
    let Some(h) = header::find(input) else {
        bail!("no CE-158 or PC-1600 header found");
    };
    if h.file_type != header::FileType::Machine {
        bail!("header is not a machine-language header ({:?})", h.file_type);
    }
    let start = h.payload_start();
    let end = start + h.length;
    if end > input.len() {
        bail!(
            "header records {} payload bytes, but only {} follow it (truncated file?)",
            h.length,
            input.len() - start.min(input.len())
        );
    }
    Ok(StrippedMachine { payload: &input[start..end], trailing: input.len() - end, header: h })
}

fn expand_all(listing: &str, reg: &Registry) -> String {
    let mut out = String::with_capacity(listing.len());
    for (i, line) in listing.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&abbrev::expand(line, reg));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_to_tokenized_and_back() {
        let src = b"10 \"A\":CLEAR :WAIT\n20 GOTO 10\n";
        let out = convert(src, Device::Pc1500, Some("t"), true).unwrap();
        assert_eq!(out.content, Content::AsciiBasic);
        assert_eq!(&out.bytes[0..5], &[0x01, 0x40, b'C', b'O', b'M']);

        let back =
            convert_with(&out.bytes, Device::Pc1500, None, true, LineEnding::Lf, SegmentMarker::Wire)
                .unwrap();
        assert_eq!(back.content, Content::Ce158Basic);
        assert_eq!(back.bytes, b"10 \"A\":CLEAR :WAIT\n20 GOTO 10\n");
    }

    #[test]
    fn tokenize_accepts_cr_and_crlf_input_on_any_platform() {
        let lf = convert(b"10 \"A\":WAIT\n20 GOTO 10\n", Device::Pc1500, Some("t"), true).unwrap();
        let crlf = convert(b"10 \"A\":WAIT\r\n20 GOTO 10\r\n", Device::Pc1500, Some("t"), true).unwrap();
        let cr = convert(b"10 \"A\":WAIT\r20 GOTO 10\r", Device::Pc1500, Some("t"), true).unwrap();
        assert_eq!(lf.bytes, crlf.bytes, "CRLF input must tokenize identically to LF");
        assert_eq!(lf.bytes, cr.bytes, "bare-CR input must tokenize identically to LF");
    }

    #[test]
    fn detokenize_line_ending_is_overridable() {
        let bbin = convert(b"10 GOTO 10\n20 END\n", Device::Pc1500, Some("t"), true).unwrap().bytes;
        let crlf =
            convert_with(&bbin, Device::Pc1500, None, true, LineEnding::CrLf, SegmentMarker::Wire)
                .unwrap();
        assert_eq!(crlf.bytes, b"10 GOTO 10\r\n20 END\r\n");
        let cr = convert_with(&bbin, Device::Pc1500, None, true, LineEnding::Cr, SegmentMarker::Wire)
            .unwrap();
        assert_eq!(cr.bytes, b"10 GOTO 10\r20 END\r");
    }

    #[test]
    fn unknown_rejected() {
        assert!(convert(b"just some prose here\n", Device::Pc1500, None, true).is_err());
    }

    #[test]
    fn segment_marker_style_is_threaded_through() {
        let src = b"5 \"A\"\n10 END\n#SEGMENT\n5 \"B\"\n10 END\n";
        let wire = convert_with(src, Device::Pc1600, None, false, LineEnding::Platform, SegmentMarker::Wire)
            .unwrap();
        let mem = convert_with(src, Device::Pc1600, None, false, LineEnding::Platform, SegmentMarker::Memory)
            .unwrap();
        assert_eq!(wire.bytes.len(), mem.bytes.len() + 2);
        assert!(wire.bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
        assert!(!mem.bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
    }

    #[test]
    fn pc1500_rejects_non_ascii() {
        let e = convert("10 PRINT \"\u{00dc}\"\n".as_bytes(), Device::Pc1500, None, true).unwrap_err();
        assert!(e.to_string().contains("7-bit"));
        // PC-1600 accepts it
        assert!(convert("10 PRINT \"\u{00dc}\"\n".as_bytes(), Device::Pc1600, None, true).is_ok());
    }

    #[test]
    fn machine_header_add_and_strip() {
        let code = [0xFD, 0xA8, 0xB5];
        let ce = add_machine_header(&code, Device::Pc1500, Some("PROG"), 0x38C5, None).unwrap();
        assert_eq!(ce.len(), 27 + 3);
        let s = strip_machine_header(&ce).unwrap();
        assert_eq!((s.payload, s.trailing), (&code[..], 0));
        assert_eq!((s.header.start_addr, s.header.run_addr), (0x38C5, 0xFFFF));

        // PC-1600: no-auto-start run address keeps the load address's bank.
        let mut pc = add_machine_header(&code, Device::Pc1600, None, 0x01_C000, None).unwrap();
        pc.push(0x1A);
        let s = strip_machine_header(&pc).unwrap();
        assert_eq!((s.payload, s.trailing), (&code[..], 1));
        assert_eq!((s.header.start_addr, s.header.run_addr), (0x01_C000, 0x01_FFFF));

        assert!(add_machine_header(&code, Device::Pc1500, None, 0x1_0000, None).is_err());
        assert!(add_machine_header(&code, Device::Pc1600, None, 0x100_0000, None).is_err());
        assert!(add_machine_header(&code, Device::Pc1500, None, 0x1000, Some(0x1_0000)).is_err());
        assert!(add_machine_header(&[], Device::Pc1500, None, 0x1000, None).is_err());
        assert!(strip_machine_header(&code).is_err());
        assert!(strip_machine_header(&ce[..ce.len() - 1]).is_err());
        let basic = convert(b"10 END\n", Device::Pc1500, None, true).unwrap().bytes;
        assert!(strip_machine_header(&basic).is_err());
    }
}
