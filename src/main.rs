//! `sde` — SharpDataExchange: PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer plus
//! `get`/`put` serial transfer.
//!
//! `sde convert [options] <infile> [<outfile>]` mirrors the `convert` verb of the Java
//! `SharpDataExchange`. Direction is chosen from file content, not the name.
//! With no `<infile>` and data on stdin, reads stdin and writes stdout.
//!
//! `sde get`/`sde put` transfer BASIC or machine-language data to/from a real Pocket
//! Computer over serial; see `requirements-put-get.md` in the repo root.

use std::io::{Read, Write};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};

use sharpdx::config::Config;
use sharpdx::get_cmd::{self, GetOptions};
use sharpdx::pocket_device::PocketDevice;
use sharpdx::put_cmd::{self, PutOptions};
use sharpdx::registry::Device;
use sharpdx::verbosity::{self, VerbosityFlag};
use sharpdx::LineEnding;

#[derive(Parser)]
#[command(name = "sde", version, about = "SharpDataExchange — PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Tokenize / de-tokenize a BASIC file offline (direction detected from content).
    Convert {
        /// Input file. Omit to read from stdin (writes tokenized/de-tokenized bytes to stdout).
        infile: Option<String>,
        /// Output file. If omitted, written next to the input with the target extension.
        outfile: Option<String>,
        /// Keyword table + header flavor when tokenizing (ignored when de-tokenizing).
        #[arg(short, long, value_enum, default_value_t = DeviceArg::Pc1500)]
        device: DeviceArg,
        /// Line ending for a de-tokenized listing (ignored when tokenizing — CR and
        /// CRLF input are always accepted). `auto` = CRLF on Windows, LF elsewhere.
        #[arg(long, value_enum, default_value_t = EolArg::Auto)]
        eol: EolArg,
        /// Rejected: direction is always detected from content.
        #[arg(short = 'f', long, hide = true)]
        format: Option<String>,
        /// Verbose logging.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Receive from the Pocket Computer over serial and write it to a file.
    Get {
        /// Output file. If omitted, derived from the serial header (or `unnamed`).
        output_file: Option<String>,
        /// Target device: determines baud rate and flow control.
        #[arg(short, long, value_enum, default_value_t = DeviceArg::Pc1500)]
        device: DeviceArg,
        /// Serial port name (auto-detected if omitted).
        #[arg(short, long)]
        port: Option<String>,
        /// Output format. `ascii` on machine-language content is an error.
        #[arg(short = 'f', long, value_enum, default_value_t = FormatArg::Ascii)]
        format: FormatArg,
        /// Omit the serial header from the saved binary file (`--format binary` only).
        #[arg(long)]
        skip_header: bool,
        /// Dump the received bytes verbatim, with no header/content detection at all.
        /// Requires an output file.
        #[arg(long)]
        raw: bool,
        /// Perform the real receive, but don't write the output file — report what
        /// would have been written and where.
        #[arg(long)]
        dry_run: bool,
        /// Narrate every non-obvious decision made along the way.
        #[arg(short, long, conflicts_with = "quiet")]
        verbose: bool,
        /// Force narration off, overriding a default-verbose config setting.
        #[arg(short, long, conflicts_with = "verbose")]
        quiet: bool,
    },

    /// Read a file and send it to the Pocket Computer over serial.
    Put {
        /// Input file to send.
        input_file: String,
        /// Target device. Optional if the file already carries a recognized header.
        #[arg(short, long, value_enum)]
        device: Option<DeviceArg>,
        /// Serial port name (auto-detected if omitted).
        #[arg(short, long)]
        port: Option<String>,
        /// Override detected input format.
        #[arg(short = 'f', long, value_enum)]
        format: Option<FormatArg>,
        /// Load address for a headerless machine-language input (hex, e.g. 38C5).
        #[arg(long, value_parser = parse_hex_u32)]
        start_address: Option<u32>,
        /// Auto-run address for a headerless machine-language input (hex).
        #[arg(long, value_parser = parse_hex_u32, requires = "start_address")]
        run_address: Option<u32>,
        /// Send a headerless machine-language file exactly as read, even without
        /// --start-address.
        #[arg(long)]
        raw: bool,
        /// Report what would be sent, without opening the serial port.
        #[arg(long)]
        dry_run: bool,
        /// Narrate every non-obvious decision made along the way.
        #[arg(short, long, conflicts_with = "quiet")]
        verbose: bool,
        /// Force narration off, overriding a default-verbose config setting.
        #[arg(short, long, conflicts_with = "verbose")]
        quiet: bool,
    },

    /// Read or write a default in the per-user config file (`~/.sderc`).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the value of `key`, or "not set".
    Get { key: String },
    /// Set `key` to `value` and save immediately.
    Set { key: String, value: String },
}

#[derive(Copy, Clone, ValueEnum)]
enum DeviceArg {
    Pc1500,
    Pc1500a,
    Pc1600,
    Pc1600emul,
}

impl From<DeviceArg> for Device {
    fn from(d: DeviceArg) -> Self {
        match d {
            DeviceArg::Pc1500 | DeviceArg::Pc1500a => Device::Pc1500,
            DeviceArg::Pc1600 | DeviceArg::Pc1600emul => Device::Pc1600,
        }
    }
}

impl From<DeviceArg> for PocketDevice {
    fn from(d: DeviceArg) -> Self {
        match d {
            DeviceArg::Pc1500 => PocketDevice::Pc1500,
            DeviceArg::Pc1500a => PocketDevice::Pc1500a,
            DeviceArg::Pc1600 => PocketDevice::Pc1600,
            DeviceArg::Pc1600emul => PocketDevice::Pc1600Emul,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    Ascii,
    Binary,
}

impl From<FormatArg> for get_cmd::Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Ascii => get_cmd::Format::Ascii,
            FormatArg::Binary => get_cmd::Format::Binary,
        }
    }
}

impl From<FormatArg> for put_cmd::Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Ascii => put_cmd::Format::Ascii,
            FormatArg::Binary => put_cmd::Format::Binary,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum EolArg {
    /// CRLF on Windows, LF on macOS / Linux.
    Auto,
    /// `\n` (LF).
    Lf,
    /// `\r\n` (CRLF).
    Crlf,
    /// `\r` (CR) — the PC-1500's own line terminator.
    Cr,
}

impl From<EolArg> for LineEnding {
    fn from(e: EolArg) -> Self {
        match e {
            EolArg::Auto => LineEnding::Platform,
            EolArg::Lf => LineEnding::Lf,
            EolArg::Crlf => LineEnding::CrLf,
            EolArg::Cr => LineEnding::Cr,
        }
    }
}

/// Accepts an optional `0x`/`0X` prefix, otherwise plain hex digits.
fn parse_hex_u32(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(digits, 16).map_err(|e| format!("invalid hex value {s:?}: {e}"))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Convert { infile, outfile, device, eol, format, verbose } => {
            if format.is_some() {
                bail!("--format is not valid for convert (direction is detected from file content)");
            }
            let _ = verbose;
            match infile {
                Some(path) => {
                    let msg = sharpdx::paths::run_convert(
                        &path,
                        outfile.as_deref(),
                        device.into(),
                        eol.into(),
                    )?;
                    println!("{msg}");
                }
                None => run_stdio(device.into(), eol.into())?,
            }
            Ok(())
        }

        Command::Get { output_file, device, port, format, skip_header, raw, dry_run, verbose, quiet } => {
            let config = Config::load()?;
            let verbosity = verbosity::resolve(flag(verbose, quiet), &config);
            let opts = GetOptions {
                device: device.into(),
                port,
                format: format.into(),
                skip_header,
                raw,
                dry_run,
                verbose: verbosity,
                output_file,
            };
            let msg = get_cmd::run_get(&opts, &config)?;
            println!("{msg}");
            Ok(())
        }

        Command::Put { input_file, device, port, format, start_address, run_address, raw, dry_run, verbose, quiet } => {
            let config = Config::load()?;
            let verbosity = verbosity::resolve(flag(verbose, quiet), &config);
            let opts = PutOptions {
                device: device.map(Into::into),
                port,
                format: format.map(Into::into),
                start_address,
                run_address,
                raw,
                dry_run,
                verbose: verbosity,
                input_file,
            };
            let msg = put_cmd::run_put(&opts, &config)?;
            println!("{msg}");
            Ok(())
        }

        Command::Config { action } => match action {
            ConfigAction::Get { key } => {
                let config = Config::load()?;
                match config.get(&key) {
                    Some(v) => println!("{v}"),
                    None => println!("{key} is not set"),
                }
                Ok(())
            }
            ConfigAction::Set { key, value } => {
                let mut config = Config::load()?;
                config.set(&key, &value)?;
                println!("Set {key} = {value}");
                Ok(())
            }
        },
    }
}

fn flag(verbose: bool, quiet: bool) -> VerbosityFlag {
    if verbose {
        VerbosityFlag::Verbose
    } else if quiet {
        VerbosityFlag::Quiet
    } else {
        VerbosityFlag::Unset
    }
}

fn run_stdio(device: Device, eol: LineEnding) -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if input.is_empty() {
        bail!("no input on stdin");
    }
    let outcome = sharpdx::convert_with(&input, device, None, true, eol, sharpdx::SegmentMarker::Wire)?;
    std::io::stdout().write_all(&outcome.bytes)?;
    Ok(())
}
