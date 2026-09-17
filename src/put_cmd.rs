//! `put` orchestration, per `requirements-put-get.md` §3/§5. Reads a file, resolves the
//! effective device, builds the exact byte sequence to transmit, and sends it — or,
//! under `--dry-run`, reports what would have been sent without opening the port.

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::detect::{self, Content};
use crate::detokenize::LineEnding;
use crate::header::{self, FileType, ParsedHeader};
use crate::pocket_device::PocketDevice;
use crate::serial::{self, RealTransport, Transport};
use crate::{filename, sender};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Ascii,
    Binary,
}

pub struct PutOptions {
    pub device: Option<PocketDevice>,
    pub port: Option<String>,
    pub format: Option<Format>,
    pub start_address: Option<u32>,
    pub run_address: Option<u32>,
    pub raw: bool,
    pub dry_run: bool,
    pub verbose: bool,
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

/// Build the exact byte sequence to transmit, per §5 step 5 / §3's header-auto-add
/// rules. Returns `(bytes, header_len)`: `header_len` is the leading slice a paced send
/// treats as "the header" (sent at full speed, then paused); `0` for a headerless send.
pub fn build_put_bytes(
    raw: &[u8],
    header: Option<&ParsedHeader>,
    content: Content,
    opts: &PutOptions,
    device: PocketDevice,
) -> Result<(Vec<u8>, usize)> {
    if let Some(h) = header {
        // Already has a recognized header -- send as-is, unmodified.
        return Ok((raw.to_vec(), h.offset + h.header_len));
    }

    let forced_machine = opts.start_address.is_some();

    if !forced_machine && content == Content::AsciiBasic {
        // --format is only meaningful for machine language (§5 step 3); ASCII BASIC
        // input is always tokenized regardless of what --format was given.
        let text = crate::text::decode_bas_listing(raw);
        let reg_device = device.to_registry_device();
        if reg_device == crate::registry::Device::Pc1500 {
            if let Some((line, col, ch)) = crate::text::first_non_ascii_for_pc1500(&text) {
                bail!(
                    "PC-1500 BASIC is 7-bit ASCII: line {line}, column {col} has U+{:04X} '{ch}'",
                    ch as u32
                );
            }
        }
        let name = filename::synth_basename(&opts.input_file);
        let outcome = crate::convert::convert_with(
            text.as_bytes(),
            reg_device,
            Some(&name),
            true,
            LineEnding::Platform,
        )?;
        let built_header = header::find(&outcome.bytes).expect("convert_with always wraps a header");
        return Ok((outcome.bytes, built_header.header_len));
    }

    // No header, not ASCII BASIC -- a machine-language candidate.
    if forced_machine {
        let start = opts.start_address.expect("forced_machine implies Some");
        let run = opts.run_address.unwrap_or(0xFFFF);
        let name = filename::synth_basename(&opts.input_file);
        let built = header::build_header(header::BuildHeader {
            device: device.to_registry_device(),
            file_type: FileType::Machine,
            name: Some(&name),
            payload_len: raw.len(),
            start_addr: start,
            run_addr: run,
        });
        let header_len = built.len();
        let mut bytes = built;
        bytes.extend_from_slice(raw);
        return Ok((bytes, header_len));
    }

    if opts.raw {
        return Ok((raw.to_vec(), 0));
    }

    bail!(
        "headerless machine-language input needs --start-address (or --raw to send it \
         completely unmodified)"
    )
}

fn narrate(opts: &PutOptions, msg: impl AsRef<str>) {
    if opts.verbose {
        eprintln!("{}", msg.as_ref());
    }
}

pub fn run_put(opts: &PutOptions, config: &Config) -> Result<String> {
    let raw = std::fs::read(&opts.input_file)
        .with_context(|| format!("cannot read {}", opts.input_file))?;
    if raw.is_empty() {
        bail!("{} is empty", opts.input_file);
    }

    let header = header::find(&raw);
    let content = detect::detect(&raw);

    if let Some(h) = &header {
        narrate(opts, format!("Header already present ({:?}, offset {})", h.file_type, h.offset));
    } else {
        narrate(opts, format!("No header present; detected content: {}", content.describe()));
    }

    let device = resolve_effective_device(opts.device, header.as_ref())?;
    narrate(opts, format!("Using device {device}"));

    // §5's last bullet: an explicit `--format ascii` on headerless ASCII BASIC input
    // sends it line-by-line, untokenized, instead of the normal tokenized-binary path.
    if header.is_none() && content == Content::AsciiBasic && opts.format == Some(Format::Ascii) {
        return run_put_ascii_lines(&raw, opts, config, device);
    }

    let (bytes, header_len) = build_put_bytes(&raw, header.as_ref(), content, opts, device)?;

    if header.is_none() && header_len > 0 {
        // We synthesized or added a header ourselves (either by tokenizing ASCII BASIC,
        // or by wrapping headerless machine language given --start-address).
        if let Some(h) = header::find(&bytes) {
            if h.file_type == FileType::Machine {
                let w = device.addr_hex_width();
                narrate(
                    opts,
                    format!(
                        "Adding MACHINE header: load=0x{:0w$X} run=0x{:0w$X}",
                        h.start_addr,
                        h.run_addr,
                        w = w
                    ),
                );
            } else {
                narrate(opts, "Tokenized ASCII BASIC input before sending");
            }
        }
    } else if header.is_none() && header_len == 0 {
        narrate(opts, "Sending raw bytes unmodified (no header, --raw)");
    }

    if opts.dry_run {
        return Ok(format!(
            "Dry run: would send {} bytes to {} (header_len={header_len}, device={device}); nothing transmitted",
            bytes.len(),
            opts.input_file
        ));
    }

    let port_name = serial::resolve_port(device, opts.port.as_deref(), config)?;
    narrate(opts, format!("Using port {port_name}"));
    let mut transport = RealTransport::open(&port_name, device)?;
    send(&mut transport, device, header_len, &bytes)?;

    Ok(format!("Sent {} bytes to {port_name} ({device})", bytes.len()))
}

fn send<T: Transport>(transport: &mut T, device: PocketDevice, header_len: usize, bytes: &[u8]) -> Result<()> {
    sender::send_data(transport, device, header_len, bytes)
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
    if device.to_registry_device() == crate::registry::Device::Pc1500 {
        if let Some((line, col, ch)) = crate::text::first_non_ascii_for_pc1500(&text) {
            bail!(
                "PC-1500 BASIC is 7-bit ASCII: line {line}, column {col} has U+{:04X} '{ch}'",
                ch as u32
            );
        }
    }
    let lines: Vec<String> = text.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    narrate(opts, format!("Sending {} lines as ASCII (no tokenization)", lines.len()));

    if opts.dry_run {
        return Ok(format!(
            "Dry run: would send {} ASCII lines to {} (device={device}); nothing transmitted",
            lines.len(),
            opts.input_file
        ));
    }

    let port_name = serial::resolve_port(device, opts.port.as_deref(), config)?;
    narrate(opts, format!("Using port {port_name}"));
    let mut transport = RealTransport::open(&port_name, device)?;
    sender::send_ascii_lines(&mut transport, device, &lines)?;

    Ok(format!("Sent {} ASCII lines to {port_name} ({device})", lines.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
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
