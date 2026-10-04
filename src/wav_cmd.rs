//! CLI-only: the cassette-WAV forms of `get`, `put` and `convert`.
//!
//! Glue only: the codec is [`crate::wav`], every conversion [`crate::transfer`] — a tape
//! file is turned into the same serial image (header + payload) that `get` receives and
//! `put` sends, so the rules are the ones serial and disk transfers use.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::detokenize::LineEnding;
use crate::filename;
use crate::header;
use crate::registry::Device;
use crate::transfer::{self, Endpoint, Format, GetSpec, PutKind, PutSpec};
use crate::verbosity::narrate;
use crate::wav::{self, EncodeOptions, Leader, TapeFile};

/// `--name`, `--sample-rate`, `--leader`: how a tape is written.
#[derive(Clone, Debug)]
pub struct TapeOptions {
    /// File name on the tape (default: the header's name, else the input file's stem).
    pub name: Option<String>,
    pub sample_rate: u32,
    pub leader: Leader,
}

pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// `--leader`: `rom`, `default`, or a length in seconds.
pub fn parse_leader(s: &str) -> Result<Leader, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "rom" => Ok(Leader::Rom),
        "default" => Ok(Leader::Default),
        v => match v.trim_end_matches('s').parse::<f32>() {
            Ok(x) if x > 0.0 && x <= 600.0 => Ok(Leader::Seconds(x)),
            _ => Err(format!("{s:?}: give seconds (e.g. 5 or 0.5, at most 600), `default` or `rom`")),
        },
    }
}

/// A WAV file named on the command line, optionally with `:NAME` picking one file on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WavSource {
    pub path: PathBuf,
    pub name: Option<String>,
}

/// `<file>` or `<file>:<NAME>` where `<file>` exists and is a RIFF/WAVE file (by
/// content, not by extension). `None` for anything else.
pub fn wav_source(arg: &str) -> Option<WavSource> {
    if is_wav_file(Path::new(arg)) {
        return Some(WavSource { path: arg.into(), name: None });
    }
    let (path, name) = arg.rsplit_once(':')?;
    (!name.is_empty() && is_wav_file(Path::new(path)))
        .then(|| WavSource { path: path.into(), name: Some(name.to_string()) })
}

fn is_wav_file(p: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    p.is_file() && std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).is_ok() && wav::is_wav(&head)
}

/// One-line description of a tape file.
pub fn describe(f: &TapeFile) -> String {
    let name = if f.name.is_empty() { "(no name)".to_string() } else { format!("\"{}\"", f.name) };
    format!("{name} ({}, {} bytes)", f.kind.describe(), f.payload.len())
}

/// Decode a WAV source: every safely decoded file, or the one `:NAME` picks. Files that
/// failed their checks are reported on stderr.
pub fn decode_source(src: &WavSource, verbose: bool) -> Result<Vec<TapeFile>> {
    let data = std::fs::read(&src.path).with_context(|| format!("cannot read {}", src.path.display()))?;
    let r = wav::decode(&data).with_context(|| src.path.display().to_string())?;
    narrate(
        verbose,
        format!(
            "{}: {} Hz {}-bit, {:.1} s, {} file(s) decoded",
            src.path.display(),
            r.sample_rate,
            r.bits,
            r.duration,
            r.files.len()
        ),
    );
    for i in &r.issues {
        eprintln!("WARNING: {}: {}", src.path.display(), wav::issue_text(i));
    }
    let mut files = r.files;
    if files.is_empty() {
        bail!(
            "{}: {}",
            src.path.display(),
            if r.issues.is_empty() {
                wav::TapeError::NoSignal.to_string()
            } else {
                "no file could be decoded safely".into()
            }
        );
    }
    if let Some(want) = &src.name {
        let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        files.retain(|f| f.name.eq_ignore_ascii_case(want));
        if files.is_empty() {
            bail!("{}: no file {want:?} on the tape (it holds: {})", src.path.display(), names.join(", "));
        }
    }
    for f in &files {
        narrate(
            verbose,
            format!(
                "  {} at {:.1} s, {}, speed {:+.1} %",
                describe(f),
                f.start_time,
                f.format.describe(),
                f.speed_percent()
            ),
        );
    }
    Ok(files)
}

/// Read a `put` input. A WAV (`tape.wav` or `tape.wav:NAME`) becomes the serial image
/// of its one file, plus the tape name to use as the source name; anything else is read
/// unchanged.
pub fn read_put_input(arg: &str, verbose: bool) -> Result<(Vec<u8>, Option<String>)> {
    let Some(src) = wav_source(arg) else {
        return Ok((std::fs::read(arg).with_context(|| format!("cannot read {arg}"))?, None));
    };
    let files = decode_source(&src, verbose)?;
    if files.len() > 1 {
        let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        bail!("{arg} holds {} files; pick one with {}:NAME ({})", files.len(), src.path.display(), names.join(", "));
    }
    let f = &files[0];
    let image = wav::to_image(f).with_context(|| format!("{arg}: {}", describe(f)))?;
    narrate(verbose, format!("{arg}: using {} as a {} image", describe(f), f.format.device().header_name()));
    Ok((image, Some(if f.name.is_empty() { "UNNAMED".into() } else { f.name.clone() })))
}

/// What goes on a tape for `raw`: the files of a WAV, or the one file `put` would send
/// for any other input (BASIC listing, tokenized BASIC, machine code with a header, or
/// headerless machine code with `start`).
pub fn tape_files_from(
    raw: &[u8],
    source_name: &str,
    device: Device,
    start: Option<u32>,
    run: Option<u32>,
    opts: &TapeOptions,
) -> Result<Vec<TapeFile>> {
    if wav::is_wav(raw) {
        let mut files = wav::decode_files(raw)?;
        if let (Some(n), [f]) = (&opts.name, files.as_mut_slice()) {
            f.name = n.clone();
        }
        return Ok(files);
    }
    let h = header::find(raw);
    let content = crate::detect::detect_from_header(h.as_ref(), raw);
    let spec = PutSpec {
        source_name,
        device,
        format: None,
        start_address: start,
        run_address: run,
        raw: false,
        endpoint: Endpoint::Serial,
    };
    let out = transfer::build_put(raw, h.as_ref(), content, &spec)?;
    match out.kind {
        PutKind::Text | PutKind::Raw | PutKind::AsciiListing => {
            bail!("{source_name}: {} can't be put on tape", content.describe())
        }
        PutKind::Variables => bail!("{source_name}: variables can't be put on tape"),
        _ => {}
    }
    let mut f = wav::from_image(&out.bytes, opts.name.as_deref())?;
    if f.name.is_empty() {
        // The PC-1600 serial header has no name field.
        f.name = crate::filename::synth_basename(source_name);
    }
    Ok(vec![f])
}

/// Encode `files` and write them as one WAV.
pub fn write_tape(files: &[TapeFile], out: &Path, opts: &TapeOptions, dry_run: bool, verbose: bool) -> Result<String> {
    let bytes = wav::encode(files, &EncodeOptions { sample_rate: opts.sample_rate, leader: opts.leader })?;
    // 16-bit mono behind a 44-byte header.
    let secs = (bytes.len() - 44) as f64 / 2.0 / opts.sample_rate as f64;
    let what = files.iter().map(describe).collect::<Vec<_>>().join(", ");
    let fmt = files.first().map(|f| f.format.describe()).unwrap_or_default();
    for f in files {
        narrate(verbose, format!("Encoding {} as a {fmt} tape", describe(f)));
    }
    if dry_run {
        return Ok(format!("Dry run: would write {what} to {} ({fmt} tape, {secs:.1} s)", out.display()));
    }
    std::fs::write(out, &bytes).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(format!("{what} -> {} ({fmt} tape, {secs:.1} s, {} Hz)", out.display(), opts.sample_rate))
}

/// Output path for a WAV: the given one (`.wav` appended if it has no extension), else
/// `default`.
pub fn wav_output(given: Option<&str>, default: PathBuf) -> PathBuf {
    match given {
        Some(g) => PathBuf::from(crate::filename::append_ext_if_missing(g, "wav")),
        None => default,
    }
}

// ── get / convert from a WAV ──────────────────────────────────────────────

pub struct WavGetOptions {
    pub format: Option<Format>,
    pub skip_header: bool,
    pub eol: LineEnding,
    pub dry_run: bool,
    pub verbose: bool,
}

/// `sde get <tape.wav>[:NAME] [output]` and `sde convert <tape.wav> [output]`: each
/// file on the tape, written as `get` writes a received file. Without `output` the
/// files go to `default_dir`, named after the tape file.
pub fn run_get_wav(src: &WavSource, output: Option<&str>, default_dir: &Path, o: &WavGetOptions) -> Result<String> {
    let files = decode_source(src, o.verbose)?;
    let out_dir = match output {
        Some(p) if Path::new(p).is_dir() => Some(PathBuf::from(p)),
        Some(p) if files.len() > 1 => {
            bail!("{p}: the tape holds {} files, so the output must be an existing directory", files.len())
        }
        Some(_) => None,
        None => Some(default_dir.to_path_buf()),
    };
    let mut msgs = Vec::new();
    for f in &files {
        let image = match wav::to_image(f) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("WARNING: skipping {}: {e}", describe(f));
                continue;
            }
        };
        let spec = GetSpec { format: o.format, skip_header: o.skip_header, eol: o.eol };
        let x = transfer::extract(&image, &spec).with_context(|| describe(f))?;
        for note in &x.notes {
            eprintln!("WARNING: {}: {note}", describe(f));
        }
        let path = match &out_dir {
            Some(dir) => dir.join(host_name(f, &x)),
            None => PathBuf::from(crate::filename::append_ext_if_missing(output.expect("no dir => file"), x.ext)),
        };
        msgs.push(crate::paths::write_got_file(&describe(f), &path, &x.bytes, x.content.describe(), o.dry_run)?);
    }
    if msgs.is_empty() {
        bail!("{}: nothing on the tape could be converted", src.path.display());
    }
    Ok(msgs.join("\n"))
}

/// Host file name for a tape file: its tape name (characters a file system can't take
/// replaced by `_`), plus the extension of what is written, unless the name already
/// ends in it (`SIMPLE.BAS`). Tokenized BASIC replaces a `.BAS` the name ends in
/// (`SIMPLE.BAS` -> `SIMPLE.bbas`).
fn host_name(f: &TapeFile, x: &transfer::Extracted) -> String {
    let ext = x.ext;
    let mut stem: String =
        f.name.trim().chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    if stem.is_empty() || stem.chars().all(|c| c == '.') {
        stem = "unnamed".into();
    }
    match Path::new(&stem).extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(ext) => stem,
        Some(e) if e.eq_ignore_ascii_case(filename::BASIC_ASCII_EXT) && ext == filename::BASIC_BINARY_EXT => {
            format!("{}.{ext}", &stem[..stem.len() - e.len() - 1])
        }
        _ => format!("{stem}.{ext}"),
    }
}

// ── convert to a WAV ──────────────────────────────────────────────────────

/// `sde convert <input> [output] -f wav`.
pub fn run_convert_to_wav(
    infile: &str,
    outfile: Option<&str>,
    device: Device,
    start: Option<u32>,
    run: Option<u32>,
    opts: &TapeOptions,
    verbose: bool,
) -> Result<String> {
    let in_path = PathBuf::from(if Path::new(infile).exists() {
        infile.to_string()
    } else {
        crate::filename::append_ext_if_missing(infile, "bas")
    });
    let raw = std::fs::read(&in_path).with_context(|| format!("cannot read {}", in_path.display()))?;
    if raw.is_empty() {
        bail!("{} is empty", in_path.display());
    }
    let name = in_path.to_string_lossy().to_string();
    let files = tape_files_from(&raw, &name, device, start, run, opts)?;
    let default = if wav::is_wav(&raw) {
        let stem = in_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tape");
        in_path.with_file_name(format!("{stem}-clean.wav"))
    } else {
        in_path.with_extension("wav")
    };
    let out = wav_output(outfile, default);
    if crate::paths::same_file(&in_path, &out) {
        bail!("output {} would overwrite the input file; give a different output file", out.display());
    }
    write_tape(&files, &out, opts, false, verbose)
}

// ── put -f wav: play through the audio output ─────────────────────────────

#[cfg(feature = "audio")]
pub struct PlayOptions {
    /// The explicit `--device`, for inputs without a header.
    pub device: Option<Device>,
    pub start_address: Option<u32>,
    pub run_address: Option<u32>,
    pub tape: TapeOptions,
    /// Decode a WAV input and play a freshly encoded tape instead of the recording.
    pub clean: bool,
    /// Don't wait for Enter before playing.
    pub yes: bool,
    pub dry_run: bool,
    pub verbose: bool,
}

/// `sde put <input> -f wav`: play the input as a cassette tape through the default
/// audio output, for a CE-150 / CE-152 / CE-1600P to `CLOAD`.
#[cfg(feature = "audio")]
pub fn run_play(input: &str, o: &PlayOptions) -> Result<String> {
    use std::io::Write;

    // What to play: a WAV as recorded (unless --clean), else a fresh tape.
    let src = wav_source(input);
    let as_recorded = src.is_some() && !o.clean;
    let files = match &src {
        Some(src) => match decode_source(src, o.verbose) {
            Ok(f) => f,
            Err(e) if as_recorded => {
                eprintln!("WARNING: {e:#}; playing it anyway");
                Vec::new()
            }
            Err(e) => return Err(e),
        },
        None => {
            let raw = std::fs::read(input).with_context(|| format!("cannot read {input}"))?;
            if raw.is_empty() {
                bail!("{input} is empty");
            }
            let device = o.device.unwrap_or(Device::Pc1500);
            tape_files_from(&raw, input, device, o.start_address, o.run_address, &o.tape)?
        }
    };
    let what =
        if files.is_empty() { input.to_string() } else { files.iter().map(describe).collect::<Vec<_>>().join(", ") };
    let fmt = files.first().map(|f| f.format.describe()).unwrap_or("unknown");
    let load = match files.first().map(|f| f.kind) {
        Some(wav::TapeKind::Machine) => "CLOAD M",
        _ => "CLOAD",
    };
    let opts = |rate| EncodeOptions { sample_rate: rate, leader: o.tape.leader };

    if o.dry_run {
        let secs = if as_recorded {
            let (s, r) = wav::read_samples(&std::fs::read(&src.as_ref().expect("as_recorded => WAV").path)?)?;
            wav::duration(&s, r)
        } else {
            let s = wav::encode_samples(&files, &opts(o.tape.sample_rate))?;
            wav::duration(&s, o.tape.sample_rate)
        };
        return Ok(format!("Dry run: would play {what} ({fmt} tape, {secs:.1} s) through the default audio output"));
    }

    let out = crate::audio_out::Output::open_default()?;
    let rate = out.sample_rate();
    let samples = if as_recorded {
        let path = &src.as_ref().expect("as_recorded => WAV").path;
        let (s, r) = wav::read_samples(&std::fs::read(path)?)?;
        crate::audio_out::resample(s, r, rate)
    } else {
        wav::encode_samples(&files, &opts(rate))?
    };
    let secs = wav::duration(&samples, rate);
    println!("Playing {what}: {fmt} tape, {secs:.1} s, on {}.", out.name());
    println!("Connect the audio output to the cassette interface's EAR / input, set the volume high,");
    println!("then type {load} on the pocket computer and press ENTER.");
    if !o.yes {
        print!("Press Enter here to start playing… ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
    }
    out.play(samples, |p| {
        eprint!("\r  {:3.0} %  {:5.1} / {secs:.1} s", p * 100.0, p * secs);
        std::io::stderr().flush().ok();
    })?;
    eprintln!();
    Ok(format!("Played {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leader_values() {
        assert_eq!(parse_leader("rom"), Ok(Leader::Rom));
        assert_eq!(parse_leader("Default"), Ok(Leader::Default));
        assert_eq!(parse_leader("2.5"), Ok(Leader::Seconds(2.5)));
        assert_eq!(parse_leader("8s"), Ok(Leader::Seconds(8.0)));
        assert!(parse_leader("0").is_err());
        assert!(parse_leader("long").is_err());
    }

    use crate::wav::TapeKind;

    fn tape(name: &str, kind: TapeKind) -> TapeFile {
        TapeFile {
            format: wav::TapeFormat::Pc1500Ce150,
            kind,
            name: name.into(),
            load: 0,
            entry: 0xFFFF,
            payload: vec![0, 10, 3, 0xF1, 0x8E, 0x0D],
            header: Vec::new(),
            start_time: 0.0,
            end_time: 0.0,
            speed: 1.0,
        }
    }

    #[test]
    fn host_names() {
        let get = |f: &TapeFile, format| {
            let img = wav::to_image(f).unwrap();
            let x = transfer::extract(&img, &GetSpec { format, skip_header: false, eol: LineEnding::Lf }).unwrap();
            host_name(f, &x)
        };
        assert_eq!(get(&tape("LANDER", TapeKind::Basic), None), "LANDER.bas");
        assert_eq!(get(&tape("SIMPLE.BAS", TapeKind::Basic), None), "SIMPLE.BAS");
        assert_eq!(get(&tape("SIMPLE.BAS", TapeKind::Basic), Some(Format::Binary)), "SIMPLE.bbas");
        assert_eq!(get(&tape("LANDER", TapeKind::Basic), Some(Format::Binary)), "LANDER.bbas");
        assert_eq!(get(&tape("A/B:C", TapeKind::Machine), None), "A_B_C.bin");
        assert_eq!(get(&tape("", TapeKind::Machine), None), "unnamed.bin");
    }
}
