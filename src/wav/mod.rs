//! Cassette-tape WAV files: PC-1500 + CE-150 and PC-1600 + CE-1600P (MODE 0).
//!
//! Pure buffer-in / buffer-out, no I/O, no dependencies — shared by the CLI and the C ABI.
//!
//! * [`decode`] finds every file on a tape (appended `CSAVE`s included). It is tolerant
//!   of speed error, wow/flutter, low level, DC offset, polarity and sample format, but
//!   *safe*: a file is only returned when every header and block checksum matches.
//! * [`encode`] writes a clean tape (phase-continuous tones, ROM framing) with a short
//!   leader by default ([`Leader`]).
//! * [`to_image`] / [`from_image`] convert between a tape file and the CE-158 / PC-1600
//!   *serial image* (header + payload) the rest of the crate already handles, so
//!   `info`, de-tokenizing, `put` and disk images work on tape files unchanged.
//!
//! Formats, as verified against ROM-made recordings (Calc-U-1600 `CSAVE`) and the
//! CE-150 / CE-1600P ROMs, are documented in `Sharp1500-1600-Ref`
//! `Shared/Data-Formats/WAV-Cassette-Format-1500-1600.md` and in [`pc1500`] / [`pc1600`].

mod demod;
mod encode;
pub mod pc1500;
pub mod pc1600;
mod riff;

use std::fmt;

use crate::header::{self, BuildHeader, FileType};
use crate::registry::Device;

pub use riff::is_wav;

/// Which machine / interface wrote (or will read) the tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeFormat {
    /// PC-1500 / PC-1500A with CE-150 (also what a PC-1600 in MODE 1 reads).
    Pc1500Ce150,
    /// PC-1600 with CE-1600P, MODE 0 (native).
    Pc1600Ce1600p,
}

impl TapeFormat {
    pub fn device(self) -> Device {
        match self {
            TapeFormat::Pc1500Ce150 => Device::Pc1500,
            TapeFormat::Pc1600Ce1600p => Device::Pc1600,
        }
    }

    pub fn for_device(d: Device) -> Self {
        match d {
            Device::Pc1500 => TapeFormat::Pc1500Ce150,
            Device::Pc1600 => TapeFormat::Pc1600Ce1600p,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            TapeFormat::Pc1500Ce150 => "PC-1500 (CE-150)",
            TapeFormat::Pc1600Ce1600p => "PC-1600 (CE-1600P, MODE 0)",
        }
    }
}

/// What a tape file holds, from its header type byte(s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeKind {
    Basic,
    Machine,
    Reserve,
    /// PC-1500 `DEF` (sub-type 03) — recognised, not converted.
    DefKeys,
    /// PC-1500 `DAT` / PC-1600 data file — recognised, not converted.
    Data,
    /// PC-1600 `CSAVE ,A` ASCII program — recognised, not converted.
    Ascii,
}

impl TapeKind {
    pub fn describe(self) -> &'static str {
        match self {
            TapeKind::Basic => "BASIC program",
            TapeKind::Machine => "machine code",
            TapeKind::Reserve => "reserve area",
            TapeKind::DefKeys => "DEF key file",
            TapeKind::Data => "data file",
            TapeKind::Ascii => "ASCII program",
        }
    }
}

/// One file decoded from (or to be written to) a tape.
#[derive(Clone, Debug, PartialEq)]
pub struct TapeFile {
    pub format: TapeFormat,
    pub kind: TapeKind,
    /// Name as stored on tape (trailing NULs / spaces removed).
    pub name: String,
    /// Load address (PC-1600: 24-bit, bank in bits 16-23).
    pub load: u32,
    /// Entry address as stored (`xFFFF` = no auto-start).
    pub entry: u32,
    /// The payload proper: no tape header, no checksums, and for PC-1500 BASIC no
    /// trailing `FF` end mark — i.e. exactly the CE-158 / PC-1600 serial payload.
    pub payload: Vec<u8>,
    /// The tape header bytes (40 for PC-1500, 48 for PC-1600) as read or written.
    pub header: Vec<u8>,
    /// Where the file starts / ends on the tape, seconds.
    pub start_time: f64,
    pub end_time: f64,
    /// Measured tape speed relative to nominal (1.0 = exact; 1.05 = 5 % fast).
    pub speed: f64,
}

impl TapeFile {
    /// How far the recording ran fast (+) or slow (−) of nominal, in percent.
    pub fn speed_percent(&self) -> f64 {
        (self.speed - 1.0) * 100.0
    }

    pub fn autorun(&self) -> bool {
        self.kind == TapeKind::Machine && crate::transfer::is_autorun(self.entry)
    }
}

/// A file that was found but could not be decoded safely (or isn't supported).
#[derive(Clone, Debug, PartialEq)]
pub struct TapeIssue {
    pub format: TapeFormat,
    /// Seconds into the tape.
    pub time: f64,
    /// Name / kind, when the header itself was readable.
    pub name: Option<String>,
    pub kind: Option<TapeKind>,
    pub message: String,
}

/// Everything found on a tape.
#[derive(Clone, Debug)]
pub struct DecodeReport {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub float: bool,
    pub duration: f64,
    pub level_dbfs: f64,
    pub files: Vec<TapeFile>,
    pub issues: Vec<TapeIssue>,
}

impl DecodeReport {
    /// The tape format, if anything on the tape was recognised.
    pub fn format(&self) -> Option<TapeFormat> {
        self.files.first().map(|f| f.format).or_else(|| self.issues.first().map(|i| i.format))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TapeError {
    /// Not a RIFF/WAVE file at all.
    NotWav(String),
    /// A WAV, but in a form or with content this crate doesn't handle.
    Unsupported(String),
    /// A WAV with no recognisable PC-1500 / PC-1600 tape signal.
    NoSignal,
    /// A tape file that was found but failed its checksums / structure checks.
    Corrupt(String),
}

impl fmt::Display for TapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TapeError::NotWav(m) => write!(f, "not a WAV file: {m}"),
            TapeError::Unsupported(m) => write!(f, "unsupported: {m}"),
            TapeError::NoSignal => {
                write!(f, "no PC-1500 (CE-150) or PC-1600 (CE-1600P) tape signal found in the WAV file")
            }
            TapeError::Corrupt(m) => write!(f, "tape could not be decoded safely: {m}"),
        }
    }
}

impl std::error::Error for TapeError {}

/// Decode every file on the tape. Fails only when the buffer isn't a usable WAV; a WAV
/// with no tape signal returns an empty report (see [`decode_files`] for the strict form).
pub fn decode(wav: &[u8]) -> Result<DecodeReport, TapeError> {
    let pcm = riff::read(wav)?;
    let e = demod::edges(|| pcm.samples(), pcm.sample_rate);
    let (mut files, mut issues) = pc1500::decode(&e.times);
    let (f16, i16) = pc1600::decode(&e.times);
    files.extend(f16);
    issues.extend(i16);
    files.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));
    issues.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(DecodeReport {
        sample_rate: pcm.sample_rate,
        channels: pcm.channels,
        bits: pcm.bits,
        float: pcm.float,
        duration: pcm.len() as f64 / pcm.sample_rate as f64,
        level_dbfs: e.level_dbfs,
        files,
        issues,
    })
}

/// [`decode`], but an error unless at least one file decoded safely.
pub fn decode_files(wav: &[u8]) -> Result<Vec<TapeFile>, TapeError> {
    let r = decode(wav)?;
    if r.files.is_empty() {
        return Err(match r.issues.first() {
            Some(i) => TapeError::Corrupt(issue_text(i)),
            None => TapeError::NoSignal,
        });
    }
    Ok(r.files)
}

/// One-line description of an issue, with its position on the tape.
pub fn issue_text(i: &TapeIssue) -> String {
    let what = match (&i.name, i.kind) {
        (Some(n), Some(k)) => format!("{} \"{n}\": ", k.describe()),
        (None, Some(k)) => format!("{}: ", k.describe()),
        _ => String::new(),
    };
    format!("{what}{} (at {:.2} s)", i.message, i.time)
}

/// How long a lead-in tone to write before each header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leader {
    /// Short, but long enough for `CLOAD` with the remote: about 2 s (PC-1500) /
    /// 3 s (PC-1600).
    Default,
    /// The ROMs' own lengths (PC-1500 ≈ 8 s, PC-1600 10000 + 11000 cycles).
    Rom,
    /// Any length, in seconds (the PC-1600 data leader never drops below what `CLOAD`
    /// needs to find it).
    Seconds(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncodeOptions {
    pub sample_rate: u32,
    pub leader: Leader,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions { sample_rate: 48_000, leader: Leader::Default }
    }
}

/// Lowest sample rate [`encode`] writes. Below it a PC-1600 "0" (3000 Hz) gets too few
/// samples per cycle: the CE-1600P's `CLOAD` fails on 8 kHz files (Calc-U-1600, ROM
/// `CLOAD`), while 16 kHz loads.
pub const MIN_WRITE_RATE: u32 = 16_000;

/// Encode files (all of one format) to a 16-bit mono WAV.
pub fn encode(files: &[TapeFile], opts: &EncodeOptions) -> Result<Vec<u8>, TapeError> {
    if opts.sample_rate < MIN_WRITE_RATE {
        return Err(TapeError::Unsupported(format!(
            "sample rate {} Hz (a tape needs at least {MIN_WRITE_RATE} Hz to load reliably)",
            opts.sample_rate
        )));
    }
    let samples = encode_samples(files, opts)?;
    Ok(riff::write_pcm16(&samples, opts.sample_rate))
}

/// Encode to raw samples in `-1.0..=1.0` at `opts.sample_rate` (for live playback).
pub fn encode_samples(files: &[TapeFile], opts: &EncodeOptions) -> Result<Vec<f32>, TapeError> {
    if !(4000..=384_000).contains(&opts.sample_rate) {
        return Err(TapeError::Unsupported(format!("sample rate {} Hz", opts.sample_rate)));
    }
    if let Leader::Seconds(s) = opts.leader {
        if !(s.is_finite() && s > 0.0 && s <= 600.0) {
            return Err(TapeError::Unsupported(format!("leader length {s} s")));
        }
    }
    let mut synth = encode::Synth::new(opts.sample_rate);
    for f in files {
        match f.format {
            TapeFormat::Pc1500Ce150 => pc1500::encode(&mut synth, f, opts.leader)?,
            TapeFormat::Pc1600Ce1600p => pc1600::encode(&mut synth, f, opts.leader)?,
        }
    }
    Ok(synth.finish())
}

/// A WAV's audio as mono samples in `-1.0..=1.0` and its sample rate (the loudest
/// channel of a multi-channel file), e.g. to play a recording unchanged.
pub fn read_samples(wav: &[u8]) -> Result<(Vec<f32>, u32), TapeError> {
    let pcm = riff::read(wav)?;
    Ok((pcm.samples().collect(), pcm.sample_rate))
}

/// Playing time of an encoded sample buffer, seconds.
pub fn duration(samples: &[f32], rate: u32) -> f64 {
    samples.len() as f64 / rate as f64
}

/// The CE-158 / PC-1600 serial image (header + payload) for a decoded tape file — the
/// form `sde` stores binary files in and every other module understands.
pub fn to_image(f: &TapeFile) -> Result<Vec<u8>, TapeError> {
    let file_type = match (f.format, f.kind) {
        (_, TapeKind::Basic) => FileType::Basic,
        (_, TapeKind::Machine) => FileType::Machine,
        (TapeFormat::Pc1500Ce150, TapeKind::Reserve) => FileType::Reserve,
        (fmt, k) => {
            return Err(TapeError::Unsupported(format!(
                "{} on a {} tape can't be converted yet",
                k.describe(),
                fmt.describe()
            )))
        }
    };
    let mut out = header::build_header(BuildHeader {
        device: f.format.device(),
        file_type,
        name: (!f.name.is_empty()).then_some(f.name.as_str()),
        payload_len: f.payload.len(),
        start_addr: f.load,
        run_addr: f.entry,
    });
    out.extend_from_slice(&f.payload);
    Ok(out)
}

/// A tape file from a serial image (CE-158 or PC-1600 header + payload). The tape
/// format follows the header; `name` overrides the header's file name (the PC-1600
/// header has none).
pub fn from_image(raw: &[u8], name: Option<&str>) -> Result<TapeFile, TapeError> {
    let h = header::find(raw)
        .ok_or_else(|| TapeError::Unsupported("the input has no CE-158 / PC-1600 header to put on tape".into()))?;
    let kind = match h.file_type {
        FileType::Basic => TapeKind::Basic,
        FileType::Machine => TapeKind::Machine,
        FileType::Reserve => TapeKind::Reserve,
        FileType::Variables => return Err(TapeError::Unsupported("variables can't be saved to tape".into())),
    };
    let start = h.payload_start().min(raw.len());
    let end = (start + h.length).min(raw.len());
    let format = TapeFormat::for_device(h.device);
    let name = name.map(str::to_string).or(h.filename).unwrap_or_default();
    let limit = match format {
        TapeFormat::Pc1500Ce150 => 0x1_0000,
        TapeFormat::Pc1600Ce1600p => 0x100_0000,
    };
    if end - start >= limit {
        return Err(TapeError::Unsupported(format!("{} bytes is too long for a tape file", end - start)));
    }
    Ok(TapeFile {
        format,
        kind,
        name,
        load: h.start_addr,
        entry: if kind == TapeKind::Machine { h.run_addr } else { 0 },
        payload: raw[start..end].to_vec(),
        header: Vec::new(),
        start_time: 0.0,
        end_time: 0.0,
        speed: 1.0,
    })
}

/// Two-class duration tracker shared by both decoders: follows slow speed drift by
/// adapting each class centre, and classifies against their geometric mean.
#[derive(Clone, Copy, Debug)]
struct Tracker {
    short: f64,
    long: f64,
}

impl Tracker {
    fn new(short: f64, ratio: f64) -> Self {
        Tracker { short, long: short * ratio }
    }

    fn threshold(&self) -> f64 {
        (self.short * self.long).sqrt()
    }

    /// `true` = long. Outliers (glitches, gaps) don't move the centres.
    fn classify(&mut self, d: f64) -> bool {
        let long = d > self.threshold();
        let (c, w) = if long { (&mut self.long, 0.02) } else { (&mut self.short, 0.02) };
        if d > *c * 0.7 && d < *c * 1.4 {
            *c += w * (d - *c);
        }
        long
    }
}

/// Start of the first steady tone of at least `min` durations at or after `from` (a
/// leader), and its mean duration. "Steady" means within ±25 % of the running mean;
/// a few outliers (noise spikes, a dropped edge) are allowed, three in a row are not.
fn find_steady(d: &[f64], from: usize, min: usize) -> Option<(usize, f64)> {
    let mut i = from;
    while i + min <= d.len() {
        let mut mean = d[i];
        if mean <= 0.0 {
            i += 1;
            continue;
        }
        let (mut n, mut sum, mut bad, mut run_bad) = (1usize, d[i], 0usize, 0usize);
        let mut j = i + 1;
        while j < d.len() && n < min {
            if (d[j] - mean).abs() < mean * 0.25 {
                n += 1;
                sum += d[j];
                mean = sum / n as f64;
                run_bad = 0;
            } else {
                bad += 1;
                run_bad += 1;
                if run_bad >= 3 || bad * 16 > n + 16 {
                    break;
                }
            }
            j += 1;
        }
        if n >= min {
            return Some((i, mean));
        }
        i += 1.max(j - i - run_bad);
    }
    None
}

#[cfg(test)]
mod tests;
