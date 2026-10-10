//! `sde` — SharpDataExchange: PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer plus
//! `get`/`put` serial transfer.
//!
//! `sde convert [options] <infile> [<outfile>]` tokenizes/de-tokenizes a BASIC listing,
//! or adds (`--start-address`) / strips a machine-language header. Direction is chosen
//! from file content, not the name. With no `<infile>` and data on
//! stdin, reads stdin and writes stdout.
//!
//! `sde info <file>` describes a file: content type, header, addresses, size.
//!
//! `sde get`/`sde put` transfer BASIC or machine-language data to/from a real Pocket
//! Computer over serial, or to/from a Calc-U-1600 floppy image; `sde dir`/`sde del` list
//! and delete files on such an image.
//!
//! Cassette WAV files (PC-1500 + CE-150, PC-1600 + CE-1600P) are read wherever a file is
//! (recognized by content), and written only with `-f wav`: `get -f wav` / `convert -f
//! wav` write a WAV file, `put -f wav` plays the tape through the audio output.

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use sharpdx::config::Config;
use sharpdx::disk_cmd::{self, DiskGetOptions, DiskPutOptions};
use sharpdx::get_cmd::{self, GetOptions};
use sharpdx::pocket_device::PocketDevice;
use sharpdx::put_cmd::{self, PutOptions};
use sharpdx::registry::Device;
use sharpdx::transfer::Format;
use sharpdx::verbosity::{self, VerbosityFlag};
use sharpdx::wav::Leader;
use sharpdx::wav_cmd::{self, TapeOptions, WavGetOptions};
use sharpdx::LineEnding;

#[derive(Parser)]
#[command(name = "sde", version, about = "SharpDataExchange — move programs between a PC and a Sharp PC-1500 / PC-1600: serial, Calc-U-1600 disks, cassette WAVs, offline conversion")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Tokenize / de-tokenize a BASIC file, add / strip a machine-language header, or
    /// unpack a cassette WAV, offline (direction detected from content); `-f wav` writes
    /// a cassette WAV.
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
        /// Only `wav`: write a cassette WAV file instead (a listing is tokenized first;
        /// `-d` picks the PC-1500 or PC-1600 tape format for input without a header).
        /// Otherwise the direction is detected from content.
        #[arg(short = 'f', long, value_enum)]
        format: Option<FormatArg>,
        #[command(flatten)]
        tape: TapeArgs,
        /// Add a machine-language header with this load address (hex, e.g. 38C5; PC-1600:
        /// 24-bit with the bank in the top byte) to a headerless input.
        #[arg(long, value_parser = parse_hex_u32)]
        start_address: Option<u32>,
        /// Auto-run address for the added header (hex). Default: no auto-start.
        #[arg(long, value_parser = parse_hex_u32, requires = "start_address")]
        run_address: Option<u32>,
        /// Explain each step (detected content, header added/stripped, output) on stderr.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Describe a file: content type, header, addresses, size; for a cassette WAV, the
    /// tape format and every file on it.
    Info {
        /// The file to describe.
        file: String,
        /// Also show how a CPU guess was reached (on stderr).
        #[arg(short, long)]
        verbose: bool,
    },

    /// Receive from the Pocket Computer over serial, copy a file off a disk image
    /// (`sde get <image>.floppy.yaml:<A|B>:<NAME.EXT|pattern> [output]`) or a cassette
    /// WAV (`sde get <tape.wav>[:NAME] [output]`), and write it to a file.
    Get {
        /// Serial: `[output_file]` (derived from the header, or `unnamed`, if omitted).
        /// Disk image: `<image>:<side>:<name> [output file or directory]`.
        /// Cassette WAV: `<tape.wav>[:NAME] [output file or directory]`.
        #[arg(num_args = 0..=2)]
        args: Vec<String>,
        /// Target device: determines baud rate (default pc1500). Disk images are PC-1600.
        #[arg(short, long, value_enum)]
        device: Option<DeviceArg>,
        /// Serial port name (auto-detected if omitted).
        #[arg(short, long)]
        port: Option<String>,
        /// Output format (serial default: ascii; disk and WAV default: per content).
        /// `ascii` on machine-language content is an error; `wav` writes a cassette WAV.
        #[arg(short = 'f', long, value_enum)]
        format: Option<FormatArg>,
        #[command(flatten)]
        tape: TapeArgs,
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
        /// Disable RTS/CTS hardware flow control (--device pc1600 only) and pace the
        /// transfer instead, as for pc1600emul. Use this if transfers hang because the
        /// cable has no working RTS/CTS.
        #[arg(long)]
        no_flowcontrol: bool,
        #[command(flatten)]
        verbosity: VerbosityArgs,
    },

    /// Read a file and send it to the Pocket Computer over serial, store files on a disk
    /// image (`sde put <file>... <image>.floppy.yaml:<A|B>:[NAME.EXT]`) or in a folder
    /// used as a PC-1600 disk, such as Calc-U-1600's host drive (`sde put <file>... <dir>`),
    /// or play it as a cassette tape through the audio output (`-f wav`). A cassette WAV
    /// input (`<tape.wav>[:NAME]`) is decoded first.
    Put {
        /// Serial: the input file. Disk image: input file(s), then the target
        /// `<image>:<side>:` (optionally with a file name for a single input). Folder:
        /// input file(s), then an existing directory.
        #[arg(required = true)]
        inputs: Vec<String>,
        /// Target device. Optional if the file already carries a recognized header.
        #[arg(short, long, value_enum)]
        device: Option<DeviceArg>,
        /// Serial port name (auto-detected if omitted).
        #[arg(short, long)]
        port: Option<String>,
        /// `binary` (default) tokenizes a BASIC listing, `ascii` sends or stores it as text;
        /// `wav` plays the file as a cassette tape through the default audio output.
        #[arg(short = 'f', long, value_enum)]
        format: Option<FormatArg>,
        #[command(flatten)]
        tape: TapeArgs,
        /// `-f wav` with a WAV input: decode it and play a freshly encoded tape instead of
        /// the recording.
        #[arg(long)]
        clean: bool,
        /// `-f wav`: start playing at once instead of waiting for Enter.
        #[arg(short, long)]
        yes: bool,
        /// Load address for a headerless machine-language input (hex, e.g. 38C5).
        #[arg(long, value_parser = parse_hex_u32)]
        start_address: Option<u32>,
        /// Auto-run address for a headerless machine-language input (hex).
        #[arg(long, value_parser = parse_hex_u32, requires = "start_address")]
        run_address: Option<u32>,
        /// Send (or store) the file exactly as read: no header, no conversion.
        #[arg(long)]
        raw: bool,
        /// Disk image or folder: replace an existing file of the same name (even
        /// write-protected).
        #[arg(long)]
        force: bool,
        /// Report what would be sent or stored, without opening the serial port or changing
        /// the disk image or folder.
        #[arg(long)]
        dry_run: bool,
        /// Disable RTS/CTS hardware flow control (--device pc1600 only) and pace the
        /// transfer instead, as for pc1600emul. Use this if transfers hang because the
        /// cable has no working RTS/CTS.
        #[arg(long)]
        no_flowcontrol: bool,
        #[command(flatten)]
        verbosity: VerbosityArgs,
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
        #[command(flatten)]
        verbosity: VerbosityArgs,
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
        PocketDevice::from(d).to_registry_device()
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
    /// A cassette tape (PC-1500: CE-150, PC-1600: CE-1600P).
    Wav,
}

/// The host-file format of `-f`, `None` for `wav` (which is handled separately).
fn host_format(f: Option<FormatArg>) -> Option<Format> {
    match f? {
        FormatArg::Ascii => Some(Format::Ascii),
        FormatArg::Binary => Some(Format::Binary),
        FormatArg::Wav => None,
    }
}

/// `-v`/`-q`, resolved against the config file's default.
#[derive(Args, Clone, Copy)]
struct VerbosityArgs {
    /// Narrate every non-obvious decision made along the way.
    #[arg(short, long, conflicts_with = "quiet")]
    verbose: bool,
    /// Force narration off, overriding a default-verbose config setting.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,
}

impl VerbosityArgs {
    fn resolve(self, config: &Config) -> bool {
        let flag = if self.verbose {
            VerbosityFlag::Verbose
        } else if self.quiet {
            VerbosityFlag::Quiet
        } else {
            VerbosityFlag::Unset
        };
        verbosity::resolve(flag, config)
    }
}

/// `--name`, `--sample-rate`, `--leader`: how `-f wav` writes a tape.
#[derive(Args, Clone)]
struct TapeArgs {
    /// `-f wav`: file name on the tape (default: the header's name, else the input
    /// file's name).
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// `-f wav`: sample rate of a written WAV file, 16000 to 192000 (default 48000;
    /// playback uses the output device's own rate).
    #[arg(long, value_name = "HZ", value_parser = clap::value_parser!(u32).range(16000..=192000))]
    sample_rate: Option<u32>,
    /// `-f wav`: lead-in tone before each file, in seconds; `default` (about 2 s on the
    /// PC-1500, 3 s on the PC-1600) or `rom` (the original length, about 8 s / 3.3 s).
    #[arg(long, value_name = "SECONDS|rom", value_parser = wav_cmd::parse_leader)]
    leader: Option<Leader>,
}

impl TapeArgs {
    fn options(&self) -> TapeOptions {
        TapeOptions {
            name: self.name.clone(),
            sample_rate: self.sample_rate.unwrap_or(wav_cmd::DEFAULT_SAMPLE_RATE),
            leader: self.leader.unwrap_or(Leader::Default),
        }
    }

    /// The options for `-f wav`; an error if they're given without it.
    fn for_format(&self, format: Option<FormatArg>) -> Result<Option<TapeOptions>> {
        if format == Some(FormatArg::Wav) {
            return Ok(Some(self.options()));
        }
        if self.name.is_some() || self.sample_rate.is_some() || self.leader.is_some() {
            bail!("--name, --sample-rate and --leader only apply with -f wav");
        }
        Ok(None)
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
        Command::Convert { infile, outfile, device, eol, format, tape, start_address, run_address, verbose } => {
            if matches!(format, Some(FormatArg::Ascii | FormatArg::Binary)) {
                bail!("convert takes only -f wav (otherwise the direction is detected from file content)");
            }
            let tape = tape.for_format(format)?;
            // A whole cassette WAV (no `:NAME`) converts to the files on it.
            let wav_src = match (&infile, &tape) {
                (Some(path), None) => wav_cmd::wav_source(path).filter(|s| s.name.is_none()),
                _ => None,
            };
            match (infile, tape, wav_src) {
                (Some(path), Some(t), _) => {
                    let msg = wav_cmd::run_convert_to_wav(
                        &path,
                        outfile.as_deref(),
                        device.into(),
                        start_address,
                        run_address,
                        &t,
                        verbose,
                    )?;
                    println!("{msg}");
                }
                (Some(_), None, Some(src)) => {
                    if start_address.is_some() {
                        bail!("--start-address does not apply to a cassette WAV (its files carry their addresses)");
                    }
                    let dir = src
                        .path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_default();
                    let o = WavGetOptions {
                        format: None,
                        skip_header: false,
                        eol: eol.into(),
                        dry_run: false,
                        verbose,
                    };
                    println!(
                        "{}",
                        wav_cmd::run_get_wav(&src, outfile.as_deref(), &dir, &o)?
                    );
                }
                (Some(path), None, None) => {
                    let opts = sharpdx::paths::ConvertOptions {
                        device: device.into(),
                        eol: eol.into(),
                        start_address,
                        run_address,
                        verbose,
                    };
                    let msg = sharpdx::paths::run_convert(&path, outfile.as_deref(), &opts)?;
                    println!("{msg}");
                }
                (None, Some(_), _) => bail!("-f wav needs an input file"),
                (None, None, _) if start_address.is_some() => {
                    bail!("--start-address needs an input file (stdin mode only tokenizes BASIC)")
                }
                (None, None, _) => run_stdio(device.into(), eol.into())?,
            }
            Ok(())
        }

        Command::Info { file, verbose } => {
            let data = std::fs::read(&file).with_context(|| format!("cannot read {file}"))?;
            if data.is_empty() {
                bail!("{file} is empty");
            }
            let info = sharpdx::info::describe(&data);
            println!("{}", info.summary);
            let width = info.details.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
            for (key, value) in &info.details {
                println!("  {:<width$} {value}", format!("{key}:"), width = width);
            }
            for w in &info.warnings {
                println!("  warning: {w}");
            }
            for line in &info.evidence {
                verbosity::narrate(verbose, line);
            }
            Ok(())
        }

        Command::Get { args, device, port, format, tape, skip_header, eol, raw, dry_run, no_flowcontrol, verbosity } => {
            let config = Config::load()?;
            let verbosity = verbosity.resolve(&config);
            let tape = tape.for_format(format)?;
            if tape.is_some() && (skip_header || raw) {
                bail!("--skip-header and --raw do not apply with -f wav");
            }
            if let Some(first) = args.first() {
                if let Some(addr) = disk_cmd::parse_image_addr(first)? {
                    check_disk_options(device, port.as_deref(), no_flowcontrol)?;
                    let opts = DiskGetOptions {
                        format: host_format(format),
                        tape,
                        skip_header,
                        raw,
                        eol: eol.into(),
                        dry_run,
                        verbose: verbosity,
                    };
                    println!("{}", disk_cmd::run_get_disk(first, &addr, args.get(1).map(String::as_str), &opts)?);
                    return Ok(());
                }
                // A cassette WAV as the source (by content). With -f wav the first
                // argument is the output file, never a source.
                if let Some(src) = wav_cmd::wav_source(first).filter(|_| tape.is_none()) {
                    if device.is_some() || port.is_some() || no_flowcontrol || raw {
                        bail!("--device, --port, --no-flowcontrol and --raw do not apply to a cassette WAV source");
                    }
                    let o = WavGetOptions {
                        format: host_format(format),
                        skip_header,
                        eol: eol.into(),
                        dry_run,
                        verbose: verbosity,
                    };
                    let out = args.get(1).map(String::as_str);
                    println!(
                        "{}",
                        wav_cmd::run_get_wav(&src, out, std::path::Path::new("."), &o)?
                    );
                    return Ok(());
                }
            }
            if args.len() > 1 {
                bail!("serial get takes at most one output file (for a disk image use <image>.floppy.yaml:<side>:<name>)");
            }
            let opts = GetOptions {
                device: device.unwrap_or(DeviceArg::Pc1500).into(),
                port,
                format: host_format(format).unwrap_or(Format::Ascii),
                tape,
                skip_header,
                eol: eol.into(),
                raw,
                dry_run,
                verbose: verbosity,
                no_flow_control: no_flowcontrol,
                output_file: args.into_iter().next(),
            };
            let msg = get_cmd::run_get(&opts, &config)?;
            println!("{msg}");
            Ok(())
        }

        Command::Put {
            inputs,
            device,
            port,
            format,
            tape,
            clean,
            yes,
            start_address,
            run_address,
            raw,
            force,
            dry_run,
            no_flowcontrol,
            verbosity,
        } => {
            let config = Config::load()?;
            let verbosity = verbosity.resolve(&config);
            let last = inputs.last().expect("clap requires at least one input");
            if let Some(t) = tape.for_format(format)? {
                if inputs.len() > 1 || disk_cmd::parse_image_addr(last)?.is_some() {
                    bail!(
                        "put -f wav plays one file through the audio output (no disk image target)"
                    );
                }
                if port.is_some() || no_flowcontrol || raw || force {
                    bail!("--port, --no-flowcontrol, --raw and --force do not apply with -f wav");
                }
                let o = wav_cmd::PlayOptions {
                    device: device.map(Into::into),
                    start_address,
                    run_address,
                    tape: t,
                    clean,
                    yes,
                    dry_run,
                    verbose: verbosity,
                };
                println!("{}", wav_cmd::run_play(last, &o)?);
                return Ok(());
            }
            if clean || yes {
                bail!("--clean and --yes only apply with -f wav");
            }
            if let Some(addr) = disk_cmd::parse_image_addr(last)? {
                if inputs.len() < 2 {
                    bail!("put needs the file(s) to store before the disk image target");
                }
                check_disk_options(device, port.as_deref(), no_flowcontrol)?;
                let opts = DiskPutOptions {
                    format: host_format(format),
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
            if inputs.len() > 1 && std::path::Path::new(last).is_dir() {
                check_disk_options(device, port.as_deref(), no_flowcontrol)?;
                let opts = DiskPutOptions {
                    format: host_format(format),
                    start_address,
                    run_address,
                    raw,
                    force,
                    dry_run,
                    verbose: verbosity,
                };
                let files = &inputs[..inputs.len() - 1];
                println!("{}", disk_cmd::run_put_dir(files, std::path::Path::new(last), &opts)?);
                return Ok(());
            }
            if inputs.len() > 1 {
                bail!(
                    "serial put sends one file (to store several, end with a disk image target \
                     <image>.floppy.yaml:<side>: or a directory)"
                );
            }
            if force {
                bail!("--force only applies to disk images and directories");
            }
            let opts = PutOptions {
                device: device.map(Into::into),
                port,
                format: host_format(format),
                start_address,
                run_address,
                raw,
                dry_run,
                verbose: verbosity,
                no_flow_control: no_flowcontrol,
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

        Command::Del { targets, force, dry_run, verbosity } => {
            let verbosity = verbosity.resolve(&Config::load()?);
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
fn check_disk_options(device: Option<DeviceArg>, port: Option<&str>, no_flowcontrol: bool) -> Result<()> {
    if matches!(device, Some(DeviceArg::Pc1500 | DeviceArg::Pc1500a)) {
        bail!("disk images are PC-1600 media; --device pc1500/pc1500a does not apply");
    }
    if port.is_some() || no_flowcontrol {
        bail!("--port and --no-flowcontrol do not apply to disk images");
    }
    Ok(())
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
