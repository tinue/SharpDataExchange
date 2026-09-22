//! `sde` — SharpDataExchange: PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer plus
//! `get`/`put` serial transfer.
//!
//! `sde convert [options] <infile> [<outfile>]` tokenizes/de-tokenizes a BASIC listing.
//! Direction is chosen from file content, not the name. With no `<infile>` and data on
//! stdin, reads stdin and writes stdout.
//!
//! `sde get`/`sde put` transfer BASIC or machine-language data to/from a real Pocket
//! Computer over serial, or to/from a Calc-U-1600 floppy image; `sde dir`/`sde del` list
//! and delete files on such an image.

use std::io::{Read, Write};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};

use sharpdx::config::Config;
use sharpdx::disk_cmd::{self, DiskGetOptions, DiskPutOptions};
use sharpdx::get_cmd::{self, GetOptions};
use sharpdx::pocket_device::PocketDevice;
use sharpdx::put_cmd::{self, PutOptions};
use sharpdx::registry::Device;
use sharpdx::transfer::Format;
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

    /// Receive from the Pocket Computer over serial, or copy a file off a disk image
    /// (`sde get <image>.floppy.yaml:<A|B>:<NAME.EXT|pattern> [output]`), and write it to
    /// a file.
    Get {
        /// Serial: `[output_file]` (derived from the header, or `unnamed`, if omitted).
        /// Disk image: `<image>:<side>:<name> [output file or directory]`.
        #[arg(num_args = 0..=2)]
        args: Vec<String>,
        /// Target device: determines baud rate (default pc1500). Disk images are PC-1600.
        #[arg(short, long, value_enum)]
        device: Option<DeviceArg>,
        /// Serial port name (auto-detected if omitted).
        #[arg(short, long)]
        port: Option<String>,
        /// Output format (serial default: ascii; disk default: per content). `ascii` on
        /// machine-language content is an error.
        #[arg(short = 'f', long, value_enum)]
        format: Option<FormatArg>,
        /// Omit the serial header from the saved binary file (`--format binary` only).
        #[arg(long)]
        skip_header: bool,
        /// Line ending for a de-tokenized listing or text file. `auto` = CRLF on
        /// Windows, LF elsewhere.
        #[arg(long, value_enum, default_value_t = EolArg::Auto)]
        eol: EolArg,
        /// Dump the received bytes verbatim, with no header/content detection at all
        /// (serial: requires an output file; disk: the file exactly as stored).
        #[arg(long)]
        raw: bool,
        /// Perform the real receive, but don't write the output file — report what
        /// would have been written and where.
        #[arg(long)]
        dry_run: bool,
        /// Enable RTS/CTS hardware flow control (PC-1600 / pc1600emul only): RTS on
        /// `get`, CTS on `put`. Off by default; with it, a real PC-1600 is sent to
        /// unpaced. Requires SNDSTAT/RCVSTAT 24 on the PC-1600 (default is 28).
        #[arg(long)]
        flowcontrol: bool,
        /// Narrate every non-obvious decision made along the way.
        #[arg(short, long, conflicts_with = "quiet")]
        verbose: bool,
        /// Force narration off, overriding a default-verbose config setting.
        #[arg(short, long, conflicts_with = "verbose")]
        quiet: bool,
    },

    /// Read a file and send it to the Pocket Computer over serial, or store files on a
    /// disk image (`sde put <file>... <image>.floppy.yaml:<A|B>:[NAME.EXT]`).
    Put {
        /// Serial: the input file. Disk image: input file(s), then the target
        /// `<image>:<side>:` (optionally with a file name for a single input).
        #[arg(required = true)]
        inputs: Vec<String>,
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
        /// Send (or store) the file exactly as read: no header, no conversion.
        #[arg(long)]
        raw: bool,
        /// Disk image: replace an existing file of the same name (even write-protected).
        #[arg(long)]
        force: bool,
        /// Report what would be sent, without opening the serial port or changing the
        /// disk image.
        #[arg(long)]
        dry_run: bool,
        /// Enable RTS/CTS hardware flow control (PC-1600 / pc1600emul only): RTS on
        /// `get`, CTS on `put`. Off by default; with it, a real PC-1600 is sent to
        /// unpaced. Requires SNDSTAT/RCVSTAT 24 on the PC-1600 (default is 28).
        #[arg(long)]
        flowcontrol: bool,
        /// Narrate every non-obvious decision made along the way.
        #[arg(short, long, conflicts_with = "quiet")]
        verbose: bool,
        /// Force narration off, overriding a default-verbose config setting.
        #[arg(short, long, conflicts_with = "verbose")]
        quiet: bool,
    },

    /// List the files on a disk image: `sde dir <image>.floppy.yaml[:A|:B[:pattern]]`.
    Dir {
        /// `<image>`, `<image>:<side>` or `<image>:<side>:<pattern>`.
        target: String,
    },

    /// Delete files from a disk image: `sde del <image>.floppy.yaml:<A|B>:<NAME.EXT|pattern>...`
    Del {
        /// One or more `<image>:<side>:<name or pattern>`.
        #[arg(required = true)]
        targets: Vec<String>,
        /// Delete write-protected files too.
        #[arg(long)]
        force: bool,
        /// Report what would be deleted without changing the image.
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

impl From<FormatArg> for Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Ascii => Format::Ascii,
            FormatArg::Binary => Format::Binary,
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

        Command::Get { args, device, port, format, skip_header, eol, raw, dry_run, flowcontrol, verbose, quiet } => {
            let config = Config::load()?;
            let verbosity = verbosity::resolve(flag(verbose, quiet), &config);
            if let Some(first) = args.first() {
                if let Some(addr) = disk_cmd::parse_image_addr(first)? {
                    check_disk_options(device, port.as_deref(), flowcontrol)?;
                    let opts = DiskGetOptions {
                        format: format.map(Into::into),
                        skip_header,
                        raw,
                        eol: eol.into(),
                        dry_run,
                        verbose: verbosity,
                    };
                    println!("{}", disk_cmd::run_get_disk(first, &addr, args.get(1).map(String::as_str), &opts)?);
                    return Ok(());
                }
            }
            if args.len() > 1 {
                bail!("serial get takes at most one output file (for a disk image use <image>.floppy.yaml:<side>:<name>)");
            }
            let opts = GetOptions {
                device: device.unwrap_or(DeviceArg::Pc1500).into(),
                port,
                format: format.unwrap_or(FormatArg::Ascii).into(),
                skip_header,
                eol: eol.into(),
                raw,
                dry_run,
                verbose: verbosity,
                flow_control: flowcontrol,
                output_file: args.into_iter().next(),
            };
            let msg = get_cmd::run_get(&opts, &config)?;
            println!("{msg}");
            Ok(())
        }

        Command::Put { inputs, device, port, format, start_address, run_address, raw, force, dry_run, flowcontrol, verbose, quiet } => {
            let config = Config::load()?;
            let verbosity = verbosity::resolve(flag(verbose, quiet), &config);
            let last = inputs.last().expect("clap requires at least one input");
            if let Some(addr) = disk_cmd::parse_image_addr(last)? {
                if inputs.len() < 2 {
                    bail!("put needs the file(s) to store before the disk image target");
                }
                check_disk_options(device, port.as_deref(), flowcontrol)?;
                let opts = DiskPutOptions {
                    format: format.map(Into::into),
                    start_address,
                    run_address,
                    raw,
                    force,
                    dry_run,
                    verbose: verbosity,
                };
                let files = &inputs[..inputs.len() - 1];
                println!("{}", disk_cmd::run_put_disk(files, last, &addr, &opts)?);
                return Ok(());
            }
            if inputs.len() > 1 {
                bail!("serial put sends one file (to store several, end with a disk image target <image>.floppy.yaml:<side>:)");
            }
            if force {
                bail!("--force only applies to disk images");
            }
            let opts = PutOptions {
                device: device.map(Into::into),
                port,
                format: format.map(Into::into),
                start_address,
                run_address,
                raw,
                dry_run,
                verbose: verbosity,
                flow_control: flowcontrol,
                input_file: inputs.into_iter().next().expect("one input"),
            };
            let msg = put_cmd::run_put(&opts, &config)?;
            println!("{msg}");
            Ok(())
        }

        Command::Dir { target } => {
            println!("{}", disk_cmd::run_dir(&target)?);
            Ok(())
        }

        Command::Del { targets, force, dry_run, verbose, quiet } => {
            let config = Config::load()?;
            let verbosity = verbosity::resolve(flag(verbose, quiet), &config);
            println!("{}", disk_cmd::run_del(&targets, force, dry_run, verbosity)?);
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

/// Disk images are PC-1600 media and involve no serial port.
fn check_disk_options(device: Option<DeviceArg>, port: Option<&str>, flowcontrol: bool) -> Result<()> {
    if matches!(device, Some(DeviceArg::Pc1500 | DeviceArg::Pc1500a)) {
        bail!("disk images are PC-1600 media; --device pc1500/pc1500a does not apply");
    }
    if port.is_some() || flowcontrol {
        bail!("--port and --flowcontrol do not apply to disk images");
    }
    Ok(())
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
