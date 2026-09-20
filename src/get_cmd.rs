//! `get` orchestration. Receives a transfer from the Pocket Computer and writes it to a
//! file — or, under `--dry-run`, still performs the real receive but skips the
//! filesystem write.

use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::detect::{self, Content};
use crate::detokenize::{self, LineEnding};
use crate::header::{self, FileType, ParsedHeader};
use crate::pocket_device::PocketDevice;
use crate::registry::Registry;
use crate::serial::{self, RealTransport};
use crate::{filename, receiver};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Ascii,
    Binary,
}

pub struct GetOptions {
    pub device: PocketDevice,
    pub port: Option<String>,
    pub format: Format,
    pub skip_header: bool,
    pub raw: bool,
    pub dry_run: bool,
    pub verbose: bool,
    /// `--flowcontrol`: enable RTS/CTS handshaking (PC-1600 family only).
    pub flow_control: bool,
    pub output_file: Option<String>,
}

/// Outcome of processing a received transfer: what to write, where, and whether a
/// header was stripped along the way. Pure/testable — no filesystem or serial I/O.
#[derive(Debug)]
pub struct Outcome {
    pub path: String,
    pub bytes: Vec<u8>,
    /// Human-readable one-line summary, printed regardless of verbosity.
    pub summary: String,
}

pub fn run_get(opts: &GetOptions, config: &Config) -> Result<String> {
    if opts.raw && opts.output_file.is_none() {
        bail!("--raw requires an explicit output file");
    }

    opts.device.check_flow_control(opts.flow_control)?;
    let port_name = serial::resolve_port(opts.device, opts.port.as_deref(), config)?;
    crate::verbosity::narrate(opts.verbose, format!("Using port {port_name}"));
    let mut transport = RealTransport::open(&port_name, opts.device, opts.flow_control)?;
    let idle_timeout = Duration::from_millis(opts.device.idle_timeout_ms());
    let raw = receiver::receive_until_done(&mut transport, idle_timeout, opts.raw)?;
    crate::verbosity::narrate(opts.verbose, format!("Received {} bytes", raw.len()));

    let outcome = if opts.raw {
        process_raw(&raw, opts)?
    } else {
        process_normal(&raw, opts)?
    };

    if opts.dry_run {
        return Ok(format!(
            "Dry run: would write {} bytes to {} ({})",
            outcome.bytes.len(),
            outcome.path,
            outcome.summary
        ));
    }

    std::fs::write(&outcome.path, &outcome.bytes)
        .with_context(|| format!("cannot write {}", outcome.path))?;
    Ok(format!("Saving to {}", outcome.path))
}

/// `--raw` mode (§4): strip a same-family header if found, leave a wrong-family one in
/// place, then report byte count + 16-bit checksum. Requires an explicit output file
/// (checked by the caller before the port is even opened).
fn process_raw(raw: &[u8], opts: &GetOptions) -> Result<Outcome> {
    let path = opts.output_file.clone().expect("checked by caller");
    let mut bytes = raw.to_vec();

    if let Some(h) = header::find(raw) {
        let same_family = family_matches(h.device, opts.device);
        if same_family {
            crate::verbosity::narrate(opts.verbose, "Stripping detected header");
            bytes = [&raw[..h.offset], &raw[h.payload_start()..]].concat();
        } else {
            crate::verbosity::narrate(
                opts.verbose,
                format!("Not stripping {} header, as device is {}", header_flavor(h.device), opts.device),
            );
        }
    }

    let checksum: u16 = bytes.iter().fold(0u16, |acc, &b| acc.wrapping_add(b as u16));
    // Wording avoids claiming anything was saved -- under --dry-run nothing is written
    // to disk; the caller's final message ("Saving to ..." / "Dry run: would write ...")
    // is what actually reports the save/no-save outcome.
    println!("Received {} bytes (target: {path})", bytes.len());
    println!("Checksum (16-bit sum): 0x{checksum:04X}");

    Ok(Outcome { path, bytes, summary: format!("checksum 0x{checksum:04X}") })
}

fn family_matches(header_device: crate::registry::Device, cli_device: PocketDevice) -> bool {
    match header_device {
        crate::registry::Device::Pc1500 => cli_device.is_pc1500_family(),
        crate::registry::Device::Pc1600 => cli_device.is_pc1600_family(),
    }
}

fn header_flavor(device: crate::registry::Device) -> &'static str {
    match device {
        crate::registry::Device::Pc1500 => "CE-158",
        crate::registry::Device::Pc1600 => "PC-1600",
    }
}

/// Non-raw mode (§4 steps 3-6): detect content, branch on format, derive the output
/// filename.
fn process_normal(raw: &[u8], opts: &GetOptions) -> Result<Outcome> {
    let header = header::find(raw);
    let content = detect::detect_from_header(header.as_ref(), raw);

    match &header {
        Some(h) => crate::verbosity::narrate(opts.verbose, format!("Header found: {:?} ({:?})", h.file_type, h.device)),
        None => eprintln!("WARNING: No recognizable header in received data"),
    }

    let file_type = header.as_ref().map(|h| h.file_type);

    if file_type == Some(FileType::Machine) && opts.format == Format::Ascii {
        bail!("machine language cannot be converted to ASCII; use --format binary");
    }

    let bytes = match (opts.format, &header, content) {
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Basic => {
            crate::verbosity::narrate(opts.verbose, "De-tokenizing BASIC payload");
            let payload = &raw[h.payload_start()..];
            let reg = Registry::for_device(h.device);
            detokenize::detokenize_to_text(payload, reg, LineEnding::Platform)?.into_bytes()
        }
        (Format::Ascii, None, Content::AsciiBasic) => {
            crate::verbosity::narrate(opts.verbose, "Cleaning up ASCII BASIC listing");
            crate::text::decode_bas_listing(raw).into_bytes()
        }
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Reserve => {
            crate::verbosity::narrate(opts.verbose, "De-tokenizing Reserve Area payload");
            let payload = &raw[h.payload_start()..h.payload_start() + h.length];
            let reg = Registry::for_device(h.device);
            let mut layout = crate::reserve::decode_payload(payload, reg)?;
            layout.filename = h.filename.clone();
            crate::reserve::to_ascii(&layout).into_bytes()
        }
        (Format::Ascii, Some(h), _) if h.file_type == FileType::Variables => {
            crate::verbosity::narrate(opts.verbose, "De-tokenizing Variables payload");
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
            if opts.skip_header {
                match &header {
                    Some(h) => raw[h.payload_start()..].to_vec(),
                    None => {
                        eprintln!("WARNING: --skip-header given but no header was found; saving all bytes");
                        raw.to_vec()
                    }
                }
            } else {
                raw.to_vec()
            }
        }
    };

    if opts.skip_header && opts.format == Format::Binary {
        eprintln!(
            "WARNING: --skip-header omits the serial header from the saved file. \
             The file cannot be identified or reloaded without it."
        );
    }

    let ext = file_type.map(filename::ext_for).unwrap_or("bas");
    let path = resolve_output_path(opts.output_file.as_deref(), header.as_ref(), ext);

    Ok(Outcome { path, bytes, summary: content.describe().to_string() })
}

/// Filename resolution per §4 step 5.
fn resolve_output_path(given: Option<&str>, header: Option<&ParsedHeader>, ext: &str) -> String {
    if let Some(name) = given {
        return filename::append_ext_if_missing(name, ext);
    }
    let derived = header.and_then(|h| h.filename.clone()).filter(|n| !n.trim().is_empty());
    match derived {
        Some(name) => filename::append_ext_if_missing(&name, ext),
        None => {
            eprintln!("WARNING: No filename provided on command line or in header, saving to unnamed.{ext}");
            format!("unnamed.{ext}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Device as RegDevice;

    fn opts() -> GetOptions {
        GetOptions {
            device: PocketDevice::Pc1500,
            port: None,
            format: Format::Ascii,
            skip_header: false,
            raw: false,
            dry_run: false,
            flow_control: false,
            verbose: false,
            output_file: None,
        }
    }

    #[test]
    fn machine_ascii_format_is_an_error() {
        let raw = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Machine,
            name: Some("x"),
            payload_len: 4,
            start_addr: 0x8000,
            run_addr: 0xFFFF,
        });
        let mut data = raw;
        data.extend_from_slice(&[1, 2, 3, 4]);
        let err = process_normal(&data, &opts()).unwrap_err();
        assert!(err.to_string().contains("--format binary"));
    }

    #[test]
    fn machine_binary_format_ok_and_keeps_header_by_default() {
        let header_bytes = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Machine,
            name: Some("x"),
            payload_len: 4,
            start_addr: 0x8000,
            run_addr: 0xFFFF,
        });
        let mut data = header_bytes.clone();
        data.extend_from_slice(&[1, 2, 3, 4]);
        let mut o = opts();
        o.format = Format::Binary;
        let outcome = process_normal(&data, &o).unwrap();
        assert_eq!(outcome.bytes, data);
        assert!(outcome.path.ends_with(".bin"));
    }

    #[test]
    fn skip_header_strips_header_in_binary_mode() {
        let header_bytes = header::build(RegDevice::Pc1500, Some("x"), 4);
        let mut data = header_bytes.clone();
        data.extend_from_slice(&[1, 2, 3, 4]);
        let mut o = opts();
        o.format = Format::Binary;
        o.skip_header = true;
        let outcome = process_normal(&data, &o).unwrap();
        assert_eq!(outcome.bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn ascii_basic_no_header_is_cleaned_up() {
        let data = b"10 PRINT \"HI\"\r\n20 END\r\n";
        let outcome = process_normal(data, &opts()).unwrap();
        assert_eq!(outcome.bytes, b"10 PRINT \"HI\"\n20 END\n");
        assert!(outcome.path.ends_with(".bas"));
    }

    #[test]
    fn output_filename_falls_back_to_unnamed_without_header_filename() {
        let path = resolve_output_path(None, None, "bin");
        assert_eq!(path, "unnamed.bin");
    }

    #[test]
    fn output_filename_uses_header_filename_when_no_cli_arg() {
        let h = ParsedHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Basic,
            offset: 0,
            header_len: 27,
            length: 4,
            start_addr: 0,
            run_addr: 0,
            filename: Some("MYPROG".to_string()),
        };
        let path = resolve_output_path(None, Some(&h), "bas");
        assert_eq!(path, "MYPROG.bas");
    }

    #[test]
    fn output_filename_cli_arg_wins_and_keeps_existing_extension() {
        let path = resolve_output_path(Some("out.txt"), None, "bas");
        assert_eq!(path, "out.txt");
    }

    #[test]
    fn raw_mode_strips_same_family_header() {
        let header_bytes = header::build(RegDevice::Pc1500, Some("x"), 2);
        let mut data = header_bytes;
        data.extend_from_slice(&[0xAA, 0xBB]);
        let mut o = opts();
        o.raw = true;
        o.output_file = Some("out.bin".to_string());
        let outcome = process_raw(&data, &o).unwrap();
        assert_eq!(outcome.bytes, vec![0xAA, 0xBB]);
    }

    #[test]
    fn reserve_ascii_get() {
        let reg = crate::registry::Registry::for_device(RegDevice::Pc1500);
        let mut layout = crate::reserve::ReserveLayout::default();
        layout.labels[0] = "MENU".to_string();
        layout.keys[0][0] = "PRINT".to_string();
        let payload = crate::reserve::encode_payload(&layout, reg).unwrap();
        let header_bytes = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Reserve,
            name: Some("RES"),
            payload_len: payload.len(),
            start_addr: 0,
            run_addr: 0,
        });
        let mut data = header_bytes;
        data.extend_from_slice(&payload);
        let outcome = process_normal(&data, &opts()).unwrap();
        let text = String::from_utf8(outcome.bytes).unwrap();
        assert!(text.starts_with(crate::reserve::MARKER));
        assert!(text.contains("; Filename: RES"));
        assert!(text.contains("key 1: PRINT"));
        assert!(outcome.path.ends_with(".sdar"));
    }

    #[test]
    fn variables_ascii_get() {
        let values = vec![crate::variables::VarValue::NumericScalar("42".to_string())];
        let payload = crate::variables::encode_payload(&values).unwrap();
        // build_header always writes wire length 0 for Variables; get_cmd must still
        // decode the full payload by reading to end of buffer.
        let header_bytes = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Variables,
            name: Some("VARS"),
            payload_len: payload.len(),
            start_addr: 0,
            run_addr: 0,
        });
        let mut data = header_bytes;
        data.extend_from_slice(&payload);
        let outcome = process_normal(&data, &opts()).unwrap();
        let text = String::from_utf8(outcome.bytes).unwrap();
        assert!(text.starts_with(crate::variables::MARKER));
        assert!(text.contains("; Count: 1"));
        assert!(text.contains("42"));
        assert!(outcome.path.ends_with(".sdav"));
    }

    #[test]
    fn reserve_binary_get_roundtrips_with_skip_header() {
        let reg = crate::registry::Registry::for_device(RegDevice::Pc1500);
        let layout = crate::reserve::ReserveLayout::default();
        let payload = crate::reserve::encode_payload(&layout, reg).unwrap();
        let header_bytes = header::build_header(header::BuildHeader {
            device: RegDevice::Pc1500,
            file_type: FileType::Reserve,
            name: Some("RES"),
            payload_len: payload.len(),
            start_addr: 0,
            run_addr: 0,
        });
        let mut data = header_bytes;
        data.extend_from_slice(&payload);
        let mut o = opts();
        o.format = Format::Binary;
        o.skip_header = true;
        let outcome = process_normal(&data, &o).unwrap();
        assert_eq!(outcome.bytes, payload.to_vec());
    }

    #[test]
    fn raw_mode_leaves_wrong_family_header_in_place() {
        let header_bytes = header::build(RegDevice::Pc1600, Some("x"), 2);
        let mut data = header_bytes.clone();
        data.extend_from_slice(&[0xAA, 0xBB]);
        let mut o = opts(); // device: Pc1500
        o.raw = true;
        o.output_file = Some("out.bin".to_string());
        let outcome = process_raw(&data, &o).unwrap();
        assert_eq!(outcome.bytes, data);
    }
}
