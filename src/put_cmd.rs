//! `put` orchestration. Reads a file, resolves the effective device, builds the exact
//! byte sequence to transmit, and sends it — or, under `--dry-run`, reports what would
//! have been sent without opening the port.

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::detect::{self, Content};
use crate::header::{self, ParsedHeader};
use crate::pocket_device::PocketDevice;
use crate::sender;
use crate::serial::{self, RealTransport, Transport};
use crate::transfer::{self, PutBytes, PutKind, PutSpec};

pub use crate::transfer::Format;

pub struct PutOptions {
    pub device: Option<PocketDevice>,
    pub port: Option<String>,
    pub format: Option<Format>,
    pub start_address: Option<u32>,
    pub run_address: Option<u32>,
    pub raw: bool,
    pub dry_run: bool,
    pub verbose: bool,
    /// `--flowcontrol`: enable RTS/CTS handshaking (PC-1600 family only).
    pub flow_control: bool,
    pub input_file: String,
}

/// Resolve the effective device (§3's header-vs-`--device` precedence):
/// - No header: the explicit `--device`, defaulting to `pc1500` (matches `get`'s
///   stated default; `put` doesn't restate one).
/// - Header present, PC-1500 family: an explicit PC-1600-family `--device` is a
///   mismatch error; otherwise the explicit device (if PC-1500-family) or `pc1500`.
/// - Header present, PC-1600 family: an explicit PC-1500-family `--device` is a
///   mismatch error. An explicit `pc1600`/`pc1600emul` is kept as-is (the header can't
///   tell real hardware from the emulator). With neither given, this is genuinely
///   ambiguous — error rather than guess, consistent with §8's "abort, don't guess".
pub fn resolve_effective_device(
    explicit: Option<PocketDevice>,
    header: Option<&ParsedHeader>,
) -> Result<PocketDevice> {
    let Some(h) = header else {
        return Ok(explicit.unwrap_or(PocketDevice::Pc1500));
    };
    match h.device {
        crate::registry::Device::Pc1500 => match explicit {
            Some(d) if d.is_pc1600_family() => bail!(
                "file has a PC-1500/CE-158 header, but --device {d} is PC-1600 family"
            ),
            Some(d) => Ok(d),
            None => Ok(PocketDevice::Pc1500),
        },
        crate::registry::Device::Pc1600 => match explicit {
            Some(d) if d.is_pc1500_family() => bail!(
                "file has a PC-1600 header, but --device {d} is PC-1500 family"
            ),
            Some(d) => Ok(d),
            None => bail!(
                "file has a PC-1600 header; specify --device pc1600 or --device pc1600emul \
                 to select the transport"
            ),
        },
    }
}

/// Build the exact byte sequence to transmit (see [`transfer::build_put`]). Returns
/// `(bytes, header_len)`: `header_len` is the leading slice a paced send treats as "the
/// header" (sent at full speed, then paused); `0` for a headerless send.
pub fn build_put_bytes(
    raw: &[u8],
    header: Option<&ParsedHeader>,
    content: Content,
    opts: &PutOptions,
    device: PocketDevice,
) -> Result<(Vec<u8>, usize)> {
    let spec = PutSpec {
        source_name: &opts.input_file,
        device: device.to_registry_device(),
        format: opts.format,
        start_address: opts.start_address,
        run_address: opts.run_address,
        raw: opts.raw,
        endpoint: transfer::Endpoint::Serial,
    };
    let out = transfer::build_put(raw, header, content, &spec)?;
    Ok((out.bytes, out.header_len))
}

pub fn run_put(opts: &PutOptions, config: &Config) -> Result<String> {
    let raw = std::fs::read(&opts.input_file)
        .with_context(|| format!("cannot read {}", opts.input_file))?;
    if raw.is_empty() {
        bail!("{} is empty", opts.input_file);
    }

    let header = header::find(&raw);
    let content = detect::detect_from_header(header.as_ref(), &raw);

    if let Some(h) = &header {
        crate::verbosity::narrate(opts.verbose, format!("Header already present ({:?}, offset {})", h.file_type, h.offset));
    } else {
        crate::verbosity::narrate(opts.verbose, format!("No header present; detected content: {}", content.describe()));
    }

    let device = resolve_effective_device(opts.device, header.as_ref())?;
    device.check_flow_control(opts.flow_control)?;
    crate::verbosity::narrate(opts.verbose, format!("Using device {device}"));

    // §5's last bullet: an explicit `--format ascii` on headerless ASCII BASIC input
    // sends it line-by-line, untokenized, instead of the normal tokenized-binary path.
    // Reserve Area and Variables input has no equivalent "send as literal text" mode
    // (the device has no matching load command for either), so they always go through
    // `build_put_bytes`'s tokenizing path below regardless of `--format`.
    if header.is_none() && content == Content::AsciiBasic && opts.format == Some(Format::Ascii) {
        return run_put_ascii_lines(&raw, opts, config, device);
    }

    let spec = PutSpec {
        source_name: &opts.input_file,
        device: device.to_registry_device(),
        format: opts.format,
        start_address: opts.start_address,
        run_address: opts.run_address,
        raw: opts.raw,
        endpoint: transfer::Endpoint::Serial,
    };
    let PutBytes { bytes, header_len, kind } = transfer::build_put(&raw, header.as_ref(), content, &spec)?;

    match kind {
        PutKind::AsIs => {}
        PutKind::MachineWrapped => {
            if let Some(h) = header::find(&bytes) {
                let w = device.addr_hex_width();
                crate::verbosity::narrate(
                    opts.verbose,
                    format!("Adding MACHINE header: load=0x{:0w$X} run=0x{:0w$X}", h.start_addr, h.run_addr, w = w),
                );
            }
        }
        PutKind::Reserve => crate::verbosity::narrate(opts.verbose, "Tokenized Reserve Area input before sending"),
        PutKind::Variables => crate::verbosity::narrate(opts.verbose, "Tokenized Variables input before sending"),
        PutKind::TokenizedBasic => crate::verbosity::narrate(opts.verbose, "Tokenized ASCII BASIC input before sending"),
        PutKind::Text | PutKind::AsciiListing => crate::verbosity::narrate(opts.verbose, "Converted text to CP437 with CRLF line endings"),
        PutKind::Raw => crate::verbosity::narrate(opts.verbose, "Sending raw bytes unmodified (no header, --raw)"),
    }

    if opts.dry_run {
        return Ok(format!(
            "Dry run: would send {} bytes to {} (header_len={header_len}, device={device}); nothing transmitted",
            bytes.len(),
            opts.input_file
        ));
    }

    let port_name = serial::resolve_port(device, opts.port.as_deref(), config)?;
    crate::verbosity::narrate(opts.verbose, format!("Using port {port_name}"));
    let mut transport = RealTransport::open(&port_name, device, opts.flow_control)?;
    send(&mut transport, device, header_len, &bytes, opts.flow_control)?;

    Ok(format!("Sent {} bytes to {port_name} ({device})", bytes.len()))
}

fn send<T: Transport>(
    transport: &mut T,
    device: PocketDevice,
    header_len: usize,
    bytes: &[u8],
    flow_control: bool,
) -> Result<()> {
    sender::send_data(transport, device, header_len, bytes, flow_control)
}

/// Line-by-line ASCII send for headerless ASCII BASIC input, when `--format ascii` is
/// explicitly given (requirements §5's last bullet) — no tokenization, no header.
fn run_put_ascii_lines(
    raw: &[u8],
    opts: &PutOptions,
    config: &Config,
    device: PocketDevice,
) -> Result<String> {
    let text = crate::text::decode_bas_listing(raw);
    crate::text::require_ascii_for_pc1500(&text, device.to_registry_device())?;
    let lines: Vec<String> = text.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    crate::verbosity::narrate(opts.verbose, format!("Sending {} lines as ASCII (no tokenization)", lines.len()));

    if opts.dry_run {
        return Ok(format!(
            "Dry run: would send {} ASCII lines to {} (device={device}); nothing transmitted",
            lines.len(),
            opts.input_file
        ));
    }

    let port_name = serial::resolve_port(device, opts.port.as_deref(), config)?;
    crate::verbosity::narrate(opts.verbose, format!("Using port {port_name}"));
    let mut transport = RealTransport::open(&port_name, device, opts.flow_control)?;
    sender::send_ascii_lines(&mut transport, device, &lines, opts.flow_control)?;

    Ok(format!("Sent {} ASCII lines to {port_name} ({device})", lines.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::FileType;
    use crate::registry::Device as RegDevice;

    fn opts(input_file: &str) -> PutOptions {
        PutOptions {
            device: None,
            port: None,
            format: None,
            start_address: None,
            run_address: None,
            raw: false,
            dry_run: false,
            flow_control: false,
            verbose: false,
            input_file: input_file.to_string(),
        }
    }

    #[test]
    fn resolve_device_defaults_to_pc1500_without_header_or_flag() {
        assert_eq!(resolve_effective_device(None, None).unwrap(), PocketDevice::Pc1500);
    }

    #[test]
    fn resolve_device_keeps_explicit_choice_without_header() {
        assert_eq!(
            resolve_effective_device(Some(PocketDevice::Pc1500a), None).unwrap(),
            PocketDevice::Pc1500a
        );
    }

    #[test]
    fn resolve_device_pc1600_header_ambiguous_without_flag_errors() {
        let h = ParsedHeader {
            device: RegDevice::Pc1600,
            file_type: FileType::Basic,
            offset: 0,
            header_len: 16,
            length: 10,
            start_addr: 0,
            run_addr: 0,
            filename: None,
        };
        let err = resolve_effective_device(None, Some(&h)).unwrap_err();
        assert!(err.to_string().contains("pc1600emul"));
    }

    #[test]
    fn resolve_device_pc1600_header_keeps_explicit_emulator_choice() {
        let h = ParsedHeader {
            device: RegDevice::Pc1600,
            file_type: FileType::Basic,
            offset: 0,
            header_len: 16,
            length: 10,
            start_addr: 0,
            run_addr: 0,
            filename: None,
        };
        assert_eq!(
            resolve_effective_device(Some(PocketDevice::Pc1600Emul), Some(&h)).unwrap(),
            PocketDevice::Pc1600Emul
        );
    }

    #[test]
    fn resolve_device_family_mismatch_errors() {
        let h = ParsedHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Basic,
            offset: 0,
            header_len: 27,
            length: 10,
            start_addr: 0,
            run_addr: 0,
            filename: None,
        };
        assert!(resolve_effective_device(Some(PocketDevice::Pc1600), Some(&h)).is_err());
    }

    #[test]
    fn build_put_bytes_sends_existing_header_as_is() {
        let raw = header::build(RegDevice::Pc1500, Some("x"), 4);
        let h = header::find(&raw).unwrap();
        let (bytes, header_len) =
            build_put_bytes(&raw, Some(&h), Content::Ce158Basic, &opts("x.bbin"), PocketDevice::Pc1500)
                .unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(header_len, 27);
    }

    #[test]
    fn build_put_bytes_tokenizes_ascii_basic() {
        let raw = b"10 PRINT \"HI\"\n";
        let (bytes, header_len) =
            build_put_bytes(raw, None, Content::AsciiBasic, &opts("prog.bas"), PocketDevice::Pc1500)
                .unwrap();
        assert_eq!(header_len, 27);
        let h = header::find(&bytes).unwrap();
        assert_eq!(h.file_type, FileType::Basic);
    }

    #[test]
    fn build_put_bytes_tokenizes_reserve_ascii() {
        let raw = format!("{}1.0 pc1500\n\n[layer 1]\nlabel: MENU\nkey 1: PRINT\n", crate::reserve::MARKER);
        let (bytes, header_len) =
            build_put_bytes(raw.as_bytes(), None, Content::Ce158Reserve, &opts("prog.sdar"), PocketDevice::Pc1500)
                .unwrap();
        let h = header::find(&bytes).unwrap();
        assert_eq!(h.file_type, FileType::Reserve);
        assert_eq!(header_len, h.header_len);
        let reg = crate::registry::Registry::for_device(RegDevice::Pc1500);
        let layout = crate::reserve::decode_payload(&bytes[h.payload_start()..], reg).unwrap();
        assert_eq!(layout.labels[0], "MENU");
        assert_eq!(layout.keys[0][0], "PRINT");
    }

    #[test]
    fn build_put_bytes_tokenizes_variables_ascii() {
        let raw = format!("{}1.0 pc1500\n; Count: 1\n42\n", crate::variables::MARKER);
        let (bytes, header_len) = build_put_bytes(
            raw.as_bytes(),
            None,
            Content::Ce158Variables,
            &opts("prog.sdav"),
            PocketDevice::Pc1500,
        )
        .unwrap();
        let h = header::find(&bytes).unwrap();
        assert_eq!(h.file_type, FileType::Variables);
        assert_eq!(header_len, h.header_len);
        let file = crate::variables::decode_payload(&bytes[h.payload_start()..]).unwrap();
        assert_eq!(file.values, vec![crate::variables::VarValue::NumericScalar("42".to_string())]);
    }

    #[test]
    fn build_put_bytes_sends_existing_reserve_header_as_is() {
        let reg = crate::registry::Registry::for_device(RegDevice::Pc1500);
        let layout = crate::reserve::ReserveLayout::default();
        let payload = crate::reserve::encode_payload(&layout, reg).unwrap();
        let header_bytes = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Reserve,
            name: Some("x"),
            payload_len: payload.len(),
            start_addr: 0,
            run_addr: 0,
        });
        let mut raw = header_bytes;
        raw.extend_from_slice(&payload);
        let h = header::find(&raw).unwrap();
        let (bytes, header_len) =
            build_put_bytes(&raw, Some(&h), Content::Ce158Reserve, &opts("x.sdar"), PocketDevice::Pc1500).unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(header_len, h.header_len);
    }

    #[test]
    fn build_put_bytes_synthesizes_machine_header_with_start_address() {
        let raw = [0x01u8, 0x02, 0x03];
        let mut o = opts("prog.bin");
        o.start_address = Some(0x38C5);
        let (bytes, header_len) =
            build_put_bytes(&raw, None, Content::Unknown, &o, PocketDevice::Pc1500).unwrap();
        let h = header::find(&bytes).unwrap();
        assert_eq!(h.file_type, FileType::Machine);
        assert_eq!(h.start_addr, 0x38C5);
        assert_eq!(h.run_addr, 0xFFFF);
        assert_eq!(header_len, 27);
        assert_eq!(&bytes[header_len..], &raw);
    }

    #[test]
    fn build_put_bytes_raw_override_sends_unmodified_without_start_address() {
        let raw = [0xAAu8, 0xBB];
        let mut o = opts("prog.bin");
        o.raw = true;
        let (bytes, header_len) =
            build_put_bytes(&raw, None, Content::Unknown, &o, PocketDevice::Pc1500).unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(header_len, 0);
    }

    #[test]
    fn build_put_bytes_errors_without_header_start_address_or_raw() {
        let raw = [0xAAu8, 0xBB];
        let o = opts("prog.bin");
        let err = build_put_bytes(&raw, None, Content::Unknown, &o, PocketDevice::Pc1500).unwrap_err();
        assert!(err.to_string().contains("--start-address"));
    }

    #[test]
    fn ascii_format_flag_sends_line_by_line_dry_run() {
        let mut o = opts("prog.bas");
        o.format = Some(Format::Ascii);
        o.dry_run = true;
        let config = Config::from_entries_for_test(Default::default());
        let msg = run_put_ascii_lines(b"10 PRINT \"HI\"\n20 END\n", &o, &config, PocketDevice::Pc1500)
            .unwrap();
        assert!(msg.contains("2 ASCII lines"));
    }

    #[test]
    fn run_put_routes_to_ascii_lines_when_format_ascii_given() {
        let dir = std::env::temp_dir().join(format!("sde_put_ascii_test_{}", std::process::id()));
        std::fs::write(&dir, "10 PRINT \"HI\"\n20 END\n").unwrap();
        let mut o = opts(dir.to_str().unwrap());
        o.format = Some(Format::Ascii);
        o.dry_run = true;
        let config = Config::from_entries_for_test(Default::default());
        let msg = run_put(&o, &config).unwrap();
        std::fs::remove_file(&dir).ok();
        assert!(msg.contains("ASCII lines"), "expected ascii-lines dry-run message, got: {msg}");
    }
}
