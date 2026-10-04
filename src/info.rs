//! What a file holds, two ways:
//!
//! * [`classify`] — for programs (e.g. a program loader): a stable one-word
//!   [`FileKind`] token, [`problem`] flags, and where the payload is and goes.
//! * [`describe`] — for `sde info`: a one-line verdict plus `key: value` details.
//!
//! Both come from the same analysis, so they never disagree. Pure: no file I/O, never
//! fails (anything that doesn't parse becomes a problem flag / warning).

use crate::cpu_guess::{self, Cpu, CpuGuess};
use crate::detect::{self, Content};
use crate::header::{self, FileType, ParsedHeader};
use crate::registry::{Device, Registry};
use crate::scanner::SegmentMarker;
use crate::wav::{self, DecodeReport, TapeError, TapeFile, TapeFormat};
use crate::{convert, detokenize, reserve, text, variables};

/// What a file is, as a stable one-word token ([`FileKind::as_str`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// `basic-ascii`: an ASCII BASIC listing.
    BasicAscii,
    /// `basic-pc1500`: tokenized BASIC behind a CE-158 header.
    BasicPc1500,
    /// `basic-pc1600`: tokenized BASIC behind a PC-1600 header.
    BasicPc1600,
    /// `ml-lh5801`: machine code behind a CE-158 header.
    MlLh5801,
    /// `ml-z80`: machine code behind a PC-1600 header.
    MlZ80,
    /// `raw-lh5801`: headerless binary that looks like LH5801 code (a heuristic guess).
    RawLh5801,
    /// `raw-z80`: headerless binary that looks like Z80 code (a heuristic guess).
    RawZ80,
    /// `raw`: headerless binary, CPU not recognized.
    Raw,
    /// `reserve`: Reserve Area behind a CE-158 header.
    Reserve,
    /// `reserve-text`: Reserve Area as SDAR text.
    ReserveText,
    /// `variables`: Variables behind a CE-158 header.
    Variables,
    /// `variables-text`: Variables as SDAV text.
    VariablesText,
    /// `text`: plain text.
    Text,
    /// `empty`: zero bytes.
    Empty,
    /// `wav-pc1500`: a cassette WAV with PC-1500 (CE-150) files.
    WavPc1500,
    /// `wav-pc1600`: a cassette WAV with PC-1600 (CE-1600P) files.
    WavPc1600,
    /// `wav`: a WAV file with no decodable PC-1500 / PC-1600 tape on it.
    Wav,
}

impl FileKind {
    pub const ALL: [FileKind; 17] = [
        FileKind::BasicAscii,
        FileKind::BasicPc1500,
        FileKind::BasicPc1600,
        FileKind::MlLh5801,
        FileKind::MlZ80,
        FileKind::RawLh5801,
        FileKind::RawZ80,
        FileKind::Raw,
        FileKind::Reserve,
        FileKind::ReserveText,
        FileKind::Variables,
        FileKind::VariablesText,
        FileKind::Text,
        FileKind::Empty,
        FileKind::WavPc1500,
        FileKind::WavPc1600,
        FileKind::Wav,
    ];

    /// The token. Part of the stable API: never renamed.
    pub fn as_str(self) -> &'static str {
        self.as_cstr().to_str().expect("tokens are ASCII")
    }

    /// The token as a static NUL-terminated string (for the C ABI).
    pub fn as_cstr(self) -> &'static std::ffi::CStr {
        match self {
            FileKind::BasicAscii => c"basic-ascii",
            FileKind::BasicPc1500 => c"basic-pc1500",
            FileKind::BasicPc1600 => c"basic-pc1600",
            FileKind::MlLh5801 => c"ml-lh5801",
            FileKind::MlZ80 => c"ml-z80",
            FileKind::RawLh5801 => c"raw-lh5801",
            FileKind::RawZ80 => c"raw-z80",
            FileKind::Raw => c"raw",
            FileKind::Reserve => c"reserve",
            FileKind::ReserveText => c"reserve-text",
            FileKind::Variables => c"variables",
            FileKind::VariablesText => c"variables-text",
            FileKind::Text => c"text",
            FileKind::Empty => c"empty",
            FileKind::WavPc1500 => c"wav-pc1500",
            FileKind::WavPc1600 => c"wav-pc1600",
            FileKind::Wav => c"wav",
        }
    }
}

/// [`FileSummary::problems`] bits.
pub mod problem {
    /// The payload is shorter than the header says.
    pub const TRUNCATED: u32 = 1;
    /// Bytes follow the payload.
    pub const TRAILING: u32 = 2;
    /// `00` bytes precede the header.
    pub const LEADING_NOISE: u32 = 4;
    /// Starts with header magic, but no complete header with a known type follows.
    pub const HEADER_CUT: u32 = 8;
    /// Tokenized BASIC that doesn't de-tokenize, or Reserve Area / Variables that don't
    /// decode.
    pub const BAD_PAYLOAD: u32 = 16;
    /// Problems that make the file unusable.
    pub const FATAL: u32 = TRUNCATED | HEADER_CUT | BAD_PAYLOAD;
}

/// What a program (e.g. a loader) needs to know about a file.
///
/// For a cassette WAV (`wav-*`) the fields describe the *first* file on the tape, and
/// `payload_offset` / `payload_len` refer to its decoded payload, not to the WAV bytes
/// (decode it with [`crate::wav::decode`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    pub kind: FileKind,
    /// [`problem`] bits.
    pub problems: u32,
    /// First payload byte: after the header, `0` for headerless content.
    pub payload_offset: usize,
    /// Payload bytes present (never past the end of the data, even when truncated).
    pub payload_len: usize,
    /// `ml-*`: load address (PC-1600: bank in bits 16-23). Else `0`.
    pub load_addr: u32,
    /// `ml-*`: run address. Else `0`.
    pub run_addr: u32,
    /// `ml-*`: `run_addr` is a real auto-start (its low 16 bits are not `FFFF`).
    pub autorun: bool,
    /// CE-158 header filename, or the name in SDAR / SDAV text.
    pub name: Option<String>,
}

/// Classify `data`, the whole content of a file.
pub fn classify(data: &[u8]) -> FileSummary {
    if wav::is_wav(data) {
        return wav_summary(data, &wav::decode(data));
    }
    analyze(data).summary
}

/// The human description of one file ([`describe`]).
#[derive(Debug, Default, PartialEq)]
pub struct FileInfo {
    /// One-line verdict, e.g. `LH5801 machine code, CE-158 header`.
    pub summary: String,
    /// `key: value` details, in display order.
    pub details: Vec<(String, String)>,
    /// Problems found (truncated payload, trailing bytes, …).
    pub warnings: Vec<String>,
    /// How a CPU guess was reached (for `-v`); empty when none was made.
    pub evidence: Vec<String>,
}

impl FileInfo {
    fn detail(&mut self, key: &str, value: impl Into<String>) {
        self.details.push((key.to_string(), value.into()));
    }
}

/// Everything [`classify`] and [`describe`] report, parsed once.
struct Analysis<'a> {
    summary: FileSummary,
    header: Option<ParsedHeader>,
    payload: &'a [u8],
    guess: Option<CpuGuess>,
    decoded: Decoded,
}

enum Decoded {
    Nothing,
    Lines(Vec<String>),
    Reserve(Box<reserve::ReserveLayout>),
    Variables(variables::VarFile),
    Failed(String),
}

fn analyze(data: &[u8]) -> Analysis<'_> {
    let mut a = Analysis {
        summary: FileSummary {
            kind: FileKind::Empty,
            problems: 0,
            payload_offset: 0,
            payload_len: data.len(),
            load_addr: 0,
            run_addr: 0,
            autorun: false,
            name: None,
        },
        header: header::find(data),
        payload: data,
        guess: None,
        decoded: Decoded::Nothing,
    };
    if data.is_empty() {
        return a;
    }
    let content = detect::detect_from_header(a.header.as_ref(), data);
    if let Some(h) = a.header.clone() {
        analyze_headered(&mut a, data, &h);
        return a;
    }
    let s = &mut a.summary;
    match content {
        Content::AsciiBasic => {
            s.kind = FileKind::BasicAscii;
            let listing = text::decode_bas_listing(data);
            a.decoded = Decoded::Lines(
                listing.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect(),
            );
        }
        Content::Ce158Reserve => {
            s.kind = FileKind::ReserveText;
            a.decoded = match reserve::from_ascii(&String::from_utf8_lossy(data)) {
                Ok(layout) => Decoded::Reserve(Box::new(layout)),
                Err(e) => Decoded::Failed(format!("does not parse: {e:#}")),
            };
        }
        Content::Ce158Variables => {
            s.kind = FileKind::VariablesText;
            a.decoded = match variables::from_ascii(&String::from_utf8_lossy(data)) {
                Ok(file) => Decoded::Variables(file),
                Err(e) => Decoded::Failed(format!("does not parse: {e:#}")),
            };
        }
        Content::Text => s.kind = FileKind::Text,
        _ => {
            let guess = cpu_guess::guess_cpu(data);
            s.kind = match guess.cpu {
                Some(Cpu::Lh5801) => FileKind::RawLh5801,
                Some(Cpu::Z80) => FileKind::RawZ80,
                None => FileKind::Raw,
            };
            if header_magic(data).is_some() {
                s.problems |= problem::HEADER_CUT;
            }
            a.guess = Some(guess);
        }
    }
    finish(&mut a);
    a
}

fn analyze_headered<'a>(a: &mut Analysis<'a>, data: &'a [u8], h: &ParsedHeader) {
    let s = &mut a.summary;
    if h.offset > 0 {
        s.problems |= problem::LEADING_NOISE;
    }
    // The payload the header describes. A Variables header's length is meaningless (see
    // `ParsedHeader::length`): its payload runs to the end.
    let start = h.payload_start().min(data.len());
    let end = if h.file_type == FileType::Variables {
        data.len()
    } else {
        let end = start + h.length;
        if end > data.len() {
            s.problems |= problem::TRUNCATED;
        } else if end < data.len() {
            s.problems |= problem::TRAILING;
        }
        end.min(data.len())
    };
    s.payload_offset = start;
    s.payload_len = end - start;
    s.name = h.filename.clone();
    a.payload = &data[start..end];
    let reg = Registry::for_device(h.device);
    match h.file_type {
        FileType::Basic => {
            s.kind = match h.device {
                Device::Pc1500 => FileKind::BasicPc1500,
                Device::Pc1600 => FileKind::BasicPc1600,
            };
            a.decoded = match detokenize::detokenize(a.payload, reg) {
                Ok(lines) => Decoded::Lines(lines),
                Err(e) => Decoded::Failed(format!("payload does not de-tokenize: {e:#}")),
            };
        }
        FileType::Machine => {
            // The header doesn't name the CPU; a CE-158 file is PC-1500 (LH5801) code, a
            // PC-1600 header is written by the Z80 side's BSAVE.
            s.kind = match h.device {
                Device::Pc1500 => FileKind::MlLh5801,
                Device::Pc1600 => FileKind::MlZ80,
            };
            s.load_addr = h.start_addr;
            s.run_addr = h.run_addr;
            s.autorun = h.run_addr & 0xFFFF != crate::transfer::PC1600_NO_AUTORUN;
            a.guess = Some(cpu_guess::guess_cpu(a.payload));
        }
        FileType::Reserve => {
            s.kind = FileKind::Reserve;
            a.decoded = match reserve::decode_payload(a.payload, reg) {
                Ok(layout) => Decoded::Reserve(Box::new(layout)),
                Err(e) => Decoded::Failed(format!("payload does not decode: {e:#}")),
            };
        }
        FileType::Variables => {
            s.kind = FileKind::Variables;
            a.decoded = match variables::decode_payload(a.payload) {
                Ok(file) => Decoded::Variables(file),
                Err(e) => Decoded::Failed(format!("payload does not decode: {e:#}")),
            };
        }
    }
    finish(a);
}

/// Problem flag and name from the decoded payload.
fn finish(a: &mut Analysis) {
    let name = match &a.decoded {
        Decoded::Failed(_) => {
            a.summary.problems |= problem::BAD_PAYLOAD;
            None
        }
        Decoded::Reserve(layout) => layout.filename.clone(),
        Decoded::Variables(file) => file.filename.clone(),
        Decoded::Nothing | Decoded::Lines(_) => None,
    };
    if a.summary.name.is_none() {
        a.summary.name = name;
    }
}

/// Describe `data`, the whole content of a file.
pub fn describe(data: &[u8]) -> FileInfo {
    if wav::is_wav(data) {
        return describe_wav(data);
    }
    let a = analyze(data);
    let s = &a.summary;
    let mut info = FileInfo::default();
    info.detail("kind", s.kind.as_str());
    info.detail("file size", format!("{} bytes", data.len()));
    if let Some(h) = &a.header {
        info.detail("header", format!("{}, {} bytes at offset {}", h.device.header_name(), h.header_len, h.offset));
    }
    if let Some(name) = &s.name {
        info.detail("name", name.clone());
    }
    let hdr = a.header.as_ref().map(|h| h.device.header_name()).unwrap_or_default();
    info.summary = match s.kind {
        FileKind::Empty => "empty file".into(),
        FileKind::BasicAscii => "ASCII BASIC listing".into(),
        FileKind::BasicPc1500 => format!("PC-1500 tokenized BASIC program, {hdr} header"),
        FileKind::BasicPc1600 => format!("PC-1600 tokenized BASIC program, {hdr} header"),
        FileKind::MlLh5801 => format!("{} machine code, {hdr} header", cpu_name(Cpu::Lh5801)),
        FileKind::MlZ80 => format!("{} machine code, {hdr} header", cpu_name(Cpu::Z80)),
        FileKind::RawLh5801 => format!("probably {} machine code, no header (heuristic)", cpu_name(Cpu::Lh5801)),
        FileKind::RawZ80 => format!("probably {} machine code, no header (heuristic)", cpu_name(Cpu::Z80)),
        FileKind::Raw => "binary data, no header; CPU not recognized".into(),
        FileKind::Reserve => format!("Reserve Area, {hdr} header"),
        FileKind::ReserveText => "Reserve Area, SDAR text".into(),
        FileKind::Variables => format!("Variables, {hdr} header"),
        FileKind::VariablesText => "Variables, SDAV text".into(),
        FileKind::Text => "Plain text".into(),
        FileKind::WavPc1500 | FileKind::WavPc1600 | FileKind::Wav => {
            unreachable!("handled by describe_wav")
        }
    };

    match s.kind {
        FileKind::BasicPc1500 | FileKind::BasicPc1600 => {
            let length = a.header.as_ref().map_or(0, |h| h.length);
            info.detail("payload", format!("{length} bytes"));
        }
        FileKind::MlLh5801 | FileKind::MlZ80 => {
            let h = a.header.as_ref().expect("ml-* kinds have a header");
            machine_details(&mut info, h);
        }
        FileKind::Raw | FileKind::RawLh5801 | FileKind::RawZ80 if data.len() < cpu_guess::MIN_LEN => {
            info.detail("note", format!("too short to guess a CPU (< {} bytes)", cpu_guess::MIN_LEN));
        }
        _ => {}
    }
    match &a.decoded {
        Decoded::Lines(lines) => line_details(&mut info, lines),
        Decoded::Reserve(layout) => {
            let assigned = layout.keys.iter().flatten().filter(|k| !k.is_empty()).count();
            info.detail("assigned keys", assigned.to_string());
        }
        Decoded::Variables(file) => info.detail("variables", file.values.len().to_string()),
        Decoded::Failed(_) | Decoded::Nothing => {}
    }
    match s.kind {
        FileKind::BasicAscii => listing_details(&mut info, data),
        FileKind::Text => text_details(&mut info, data),
        _ => {}
    }
    if let Some(guess) = &a.guess {
        let implied = match s.kind {
            FileKind::MlLh5801 => Some(Cpu::Lh5801),
            FileKind::MlZ80 => Some(Cpu::Z80),
            _ => None,
        };
        if let Some(looks) = guess.cpu.filter(|&c| implied.is_some_and(|i| i != c)) {
            info.detail("code looks like", format!("{} (heuristic)", cpu_name(looks)));
        }
        info.evidence = evidence(guess);
    }
    info.warnings = warnings(&a, data);
    info
}

fn warnings(a: &Analysis, data: &[u8]) -> Vec<String> {
    let p = a.summary.problems;
    let mut w = Vec::new();
    if let Some(h) = &a.header {
        if p & problem::LEADING_NOISE != 0 {
            w.push(format!("{} bytes of 00 noise before the header", h.offset));
        }
        if p & problem::TRUNCATED != 0 {
            w.push(format!(
                "truncated: the header records {} payload bytes, only {} follow it",
                h.length, a.summary.payload_len
            ));
        }
        if p & problem::TRAILING != 0 {
            let end = a.summary.payload_offset + a.summary.payload_len;
            w.push(format!("{} trailing bytes after the payload", data.len() - end));
        }
    }
    if p & problem::HEADER_CUT != 0 {
        let hdr = header_magic(data).unwrap_or("serial");
        w.push(format!("starts like a {hdr} header, but the header is cut short or has an unknown type byte"));
    }
    if let Decoded::Failed(msg) = &a.decoded {
        w.push(msg.clone());
    }
    w
}

fn machine_details(info: &mut FileInfo, h: &ParsedHeader) {
    let w = h.device.addr_hex_width();
    let addr = |a: u32| match h.device {
        Device::Pc1600 if a >> 16 != 0 => format!("0x{a:0w$X} (bank {})", a >> 16),
        _ => format!("0x{a:0w$X}"),
    };
    info.detail("load address", addr(h.start_addr));
    if h.length > 0 {
        info.detail("end address", addr(h.start_addr + h.length as u32 - 1));
    }
    let run = if h.run_addr & 0xFFFF == crate::transfer::PC1600_NO_AUTORUN {
        format!("none (0x{:0w$X}, no auto-start)", h.run_addr)
    } else {
        addr(h.run_addr)
    };
    info.detail("run address", run);
    info.detail("payload", format!("{} bytes", h.length));
}

/// Which header's magic `data` starts with (after `00` noise), for a file
/// [`header::find`] could not parse.
fn header_magic(data: &[u8]) -> Option<&'static str> {
    let data = &data[data.iter().position(|&b| b != 0)?..];
    if data.first() == Some(&0x01) && data.get(2..5) == Some(b"COM") {
        Some(Device::Pc1500.header_name())
    } else if data.starts_with(&[0xFF, 0x10, 0x00, 0x00]) {
        Some(Device::Pc1600.header_name())
    } else {
        None
    }
}

fn cpu_name(cpu: Cpu) -> &'static str {
    match cpu {
        Cpu::Lh5801 => "LH5801",
        Cpu::Z80 => "Z80 (SC7852)",
    }
}

fn evidence(g: &CpuGuess) -> Vec<String> {
    let line = |name: &str, s: &cpu_guess::IsaStats| {
        format!(
            "heuristic, as {name}: {} instructions, {:.1}% invalid, {:.1}% typical, {}/{} relative branches land",
            s.instructions,
            s.invalid_rate() * 100.0,
            s.signature_rate() * 100.0,
            s.branch_hits,
            s.branches
        )
    };
    let (lh, z, none) = g.votes;
    vec![
        line("LH5801", &g.lh5801),
        line("Z80", &g.z80),
        format!("heuristic, {}-byte windows voting LH5801 / Z80 / undecided: {lh} / {z} / {none}", cpu_guess::WINDOW),
    ]
}

fn listing_details(info: &mut FileInfo, data: &[u8]) {
    eol_details(info, data);
    let fits: Vec<&str> = [(Device::Pc1500, "PC-1500"), (Device::Pc1600, "PC-1600")]
        .into_iter()
        .filter(|&(d, _)| convert::tokenize_listing(data, d, None, false, SegmentMarker::Wire).is_ok())
        .map(|(_, n)| n)
        .collect();
    info.detail(
        "tokenizes for",
        if fits.is_empty() { "neither PC-1500 nor PC-1600".into() } else { fits.join(", ") },
    );
}

/// Line count and line-number range of a listing (`NNN statement` lines).
fn line_details(info: &mut FileInfo, lines: &[String]) {
    info.detail("lines", lines.len().to_string());
    let numbers: Vec<u32> = lines
        .iter()
        .filter_map(|l| {
            let l = l.trim_start();
            l[..l.len() - l.trim_start_matches(|c: char| c.is_ascii_digit()).len()].parse().ok()
        })
        .collect();
    if let (Some(lo), Some(hi)) = (numbers.iter().min(), numbers.iter().max()) {
        info.detail("line numbers", format!("{lo}–{hi}"));
    }
}

fn text_details(info: &mut FileInfo, data: &[u8]) {
    let body = data.split(|&b| b == 0x1A).next().unwrap_or(data);
    let text = String::from_utf8_lossy(body);
    info.detail("lines", text.lines().count().to_string());
    eol_details(info, data);
}

fn eol_details(info: &mut FileInfo, data: &[u8]) {
    let (mut crlf, mut cr, mut lf) = (0, 0, 0);
    let mut i = 0;
    while i < data.len() {
        match (data[i], data.get(i + 1)) {
            (b'\r', Some(b'\n')) => {
                crlf += 1;
                i += 1;
            }
            (b'\r', _) => cr += 1,
            (b'\n', _) => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds: Vec<&str> =
        [(crlf, "CRLF"), (cr, "CR"), (lf, "LF")].into_iter().filter(|&(n, _)| n > 0).map(|(_, k)| k).collect();
    info.detail(
        "line endings",
        match kinds.len() {
            0 => "none".to_string(),
            1 => kinds[0].to_string(),
            _ => format!("mixed ({})", kinds.join(", ")),
        },
    );
    if data.contains(&0x1A) {
        info.detail("end-of-file mark", "1A (PC-1600 / DOS style)");
    }
}

/// Summary of a cassette WAV from its decode result.
fn wav_summary(data: &[u8], r: &Result<DecodeReport, TapeError>) -> FileSummary {
    let mut s = FileSummary {
        kind: FileKind::Wav,
        problems: 0,
        payload_offset: 0,
        payload_len: data.len(),
        load_addr: 0,
        run_addr: 0,
        autorun: false,
        name: None,
    };
    let Ok(r) = r else {
        s.problems |= problem::BAD_PAYLOAD;
        return s;
    };
    match r.format() {
        Some(TapeFormat::Pc1500Ce150) => s.kind = FileKind::WavPc1500,
        Some(TapeFormat::Pc1600Ce1600p) => s.kind = FileKind::WavPc1600,
        None => {}
    }
    match r.files.first() {
        Some(f) => {
            s.payload_len = f.payload.len();
            s.name = (!f.name.is_empty()).then(|| f.name.clone());
            if f.kind == wav::TapeKind::Machine {
                s.load_addr = f.load;
                s.run_addr = f.entry;
                s.autorun = f.autorun();
            }
            // The first file's own content problems (e.g. BASIC that doesn't
            // de-tokenize) count, as for a serial image.
            if let Ok(img) = wav::to_image(f) {
                s.problems |= analyze(&img).summary.problems & problem::FATAL;
            }
        }
        None => {
            s.payload_len = 0;
            s.problems |= problem::BAD_PAYLOAD;
        }
    }
    s
}

fn describe_wav(data: &[u8]) -> FileInfo {
    let r = wav::decode(data);
    let s = wav_summary(data, &r);
    let mut info = FileInfo::default();
    info.detail("kind", s.kind.as_str());
    info.detail("file size", format!("{} bytes", data.len()));
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            info.summary = "WAV file, not readable".into();
            info.warnings.push(e.to_string());
            return info;
        }
    };
    info.detail(
        "audio",
        format!(
            "{} Hz, {}-bit {}, {}, {:.1} s",
            r.sample_rate,
            r.bits,
            if r.float { "float" } else { "PCM" },
            match r.channels {
                1 => "mono".to_string(),
                2 => "stereo".to_string(),
                n => format!("{n} channels"),
            },
            r.duration
        ),
    );
    info.detail("level", format!("{:.1} dBFS", r.level_dbfs));
    let fmt = r.format().map(TapeFormat::describe);
    info.summary = match (fmt, r.files.as_slice()) {
        (None, _) => "WAV file, no PC-1500 / PC-1600 tape signal found".into(),
        (Some(fmt), []) => format!("{fmt} cassette WAV, no file decoded safely"),
        (Some(fmt), [f]) => format!("{fmt} cassette WAV: {}", file_line(f)),
        (Some(fmt), files) => format!("{fmt} cassette WAV, {} files", files.len()),
    };
    match r.files.as_slice() {
        [f] => tape_file_details(&mut info, f),
        files => {
            for (i, f) in files.iter().enumerate() {
                info.detail(
                    &format!("file {}", i + 1),
                    format!("{}, at {:.1} s", file_line(f), f.start_time),
                );
            }
        }
    }
    for f in &r.files {
        info.evidence.push(format!(
            "\"{}\": {:.2}-{:.2} s, tape speed {:+.1} %",
            f.name,
            f.start_time,
            f.end_time,
            (f.speed - 1.0) * 100.0
        ));
    }
    info.warnings.extend(r.issues.iter().map(wav::issue_text));
    info
}

fn file_line(f: &TapeFile) -> String {
    let name = if f.name.is_empty() {
        String::new()
    } else {
        format!(" \"{}\"", f.name)
    };
    format!("{}{name}, {} bytes", f.kind.describe(), f.payload.len())
}

/// A single tape file's details: tape facts, then what its payload is (the same details
/// `sde info` shows for the serial image).
fn tape_file_details(info: &mut FileInfo, f: &TapeFile) {
    info.detail("tape speed", format!("{:+.1} %", (f.speed - 1.0) * 100.0));
    if f.format == TapeFormat::Pc1600Ce1600p && f.header.len() >= 0x1D {
        let d = &f.header[0x19..0x1D];
        info.detail(
            "saved",
            format!("month {}, day {}, {:02}:{:02}", d[0], d[1], d[2], d[3]),
        );
    }
    let Ok(img) = wav::to_image(f) else { return };
    let inner = describe(&img);
    info.detail(
        "content",
        inner
            .summary
            .split(',')
            .next()
            .unwrap_or_default()
            .to_string(),
    );
    for (k, v) in inner.details {
        if !matches!(k.as_str(), "kind" | "file size" | "header") {
            info.details.push((k, v));
        }
    }
    info.warnings.extend(inner.warnings);
    info.evidence.extend(inner.evidence);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn get<'a>(info: &'a FileInfo, key: &str) -> &'a str {
        info.details.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or_else(|| {
            panic!("no {key:?} in {info:?}")
        })
    }

    #[test]
    fn tokenized_basic() {
        let ce = describe(&fixture("depreciation-tokenized-ce158header.bin"));
        assert_eq!(ce.summary, "PC-1500 tokenized BASIC program, CE-158 header");
        let pc = describe(&fixture("depreciation-tokenized-pc1600header.bin"));
        assert_eq!(pc.summary, "PC-1600 tokenized BASIC program, PC-1600 header");
        assert_eq!(get(&ce, "lines"), get(&pc, "lines"));
        assert!(ce.warnings.is_empty() && pc.warnings.is_empty(), "{ce:?} {pc:?}");
    }

    #[test]
    fn ascii_listing() {
        let i = describe(&fixture("depreciation.bas"));
        assert_eq!(i.summary, "ASCII BASIC listing");
        assert_eq!(get(&i, "tokenizes for"), "PC-1500, PC-1600");
        let tokenized = describe(&fixture("depreciation-tokenized-ce158header.bin"));
        assert_eq!(get(&i, "lines"), get(&tokenized, "lines"));
        assert_eq!(get(&i, "line numbers"), get(&tokenized, "line numbers"));
    }

    #[test]
    fn machine_code_with_header() {
        let ce = convert::add_machine_header(&[1, 2, 3], Device::Pc1500, Some("PROG"), 0x38C5, None).unwrap();
        let i = describe(&ce);
        assert_eq!(i.summary, "LH5801 machine code, CE-158 header");
        assert_eq!(get(&i, "name"), "PROG");
        assert_eq!(get(&i, "load address"), "0x38C5");
        assert_eq!(get(&i, "end address"), "0x38C7");
        assert_eq!(get(&i, "run address"), "none (0xFFFF, no auto-start)");
        assert_eq!(get(&i, "payload"), "3 bytes");

        let pc = convert::add_machine_header(&[1, 2], Device::Pc1600, None, 0x01_C000, Some(0x01_C000)).unwrap();
        let i = describe(&pc);
        assert_eq!(i.summary, "Z80 (SC7852) machine code, PC-1600 header");
        assert_eq!(get(&i, "load address"), "0x01C000 (bank 1)");
        assert_eq!(get(&i, "run address"), "0x01C000 (bank 1)");
        assert!(i.warnings.is_empty());
    }

    #[test]
    fn truncated_and_trailing_payloads_warn() {
        let ce = convert::add_machine_header(&[1, 2, 3], Device::Pc1500, None, 0x4000, None).unwrap();
        let short = describe(&ce[..ce.len() - 1]);
        assert!(short.warnings[0].starts_with("truncated"), "{short:?}");
        let mut long = ce.clone();
        long.extend([0, 0]);
        assert_eq!(describe(&long).warnings, ["2 trailing bytes after the payload"]);
        let mut noisy = vec![0, 0];
        noisy.extend(&ce);
        assert_eq!(describe(&noisy).warnings, ["2 bytes of 00 noise before the header"]);
        let cut = describe(&ce[..20]);
        assert_eq!(cut.warnings.len(), 1);
        assert!(cut.warnings[0].starts_with("starts like a CE-158 header"), "{cut:?}");
    }

    #[test]
    fn text_and_unknown() {
        let t = describe(b"hello\r\nworld\r\n\x1A");
        assert_eq!(t.summary, "Plain text");
        assert_eq!(get(&t, "line endings"), "CRLF");
        assert_eq!(get(&t, "lines"), "2");
        let b = describe(&[0x00, 0x01, 0x02]);
        assert_eq!(b.summary, "binary data, no header; CPU not recognized");
        assert!(get(&b, "note").starts_with("too short"));
    }

    #[test]
    fn marker_text_formats() {
        let vars = variables::VarFile { filename: Some("VARS".into()), values: Vec::new() };
        let v = describe(variables::to_ascii(&vars).as_bytes());
        assert_eq!(v.summary, "Variables, SDAV text", "{v:?}");
        assert_eq!((get(&v, "name"), get(&v, "variables")), ("VARS", "0"));

        let mut layout = reserve::ReserveLayout {
            filename: Some("KEYS".into()),
            labels: Default::default(),
            keys: Default::default(),
        };
        layout.keys[0][0] = "PRINT".into();
        let r = describe(reserve::to_ascii(&layout).as_bytes());
        assert_eq!(r.summary, "Reserve Area, SDAR text", "{r:?}");
        assert_eq!((get(&r, "name"), get(&r, "assigned keys")), ("KEYS", "1"));
    }

    #[test]
    fn tokens_are_unique_single_words() {
        let tokens: Vec<&str> = FileKind::ALL.iter().map(|k| k.as_str()).collect();
        let unique: std::collections::HashSet<_> = tokens.iter().collect();
        assert_eq!(unique.len(), tokens.len());
        assert!(tokens.iter().all(|t| !t.is_empty()
            && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')));
    }

    #[test]
    fn classify_every_kind() {
        use crate::cpu_guess::samples::*;
        let kind = |data: &[u8]| classify(data).kind;
        assert_eq!(kind(b""), FileKind::Empty);
        assert_eq!(kind(&fixture("depreciation.bas")), FileKind::BasicAscii);
        assert_eq!(kind(&fixture("depreciation-tokenized-ce158header.bin")), FileKind::BasicPc1500);
        assert_eq!(kind(&fixture("depreciation-tokenized-pc1600header.bin")), FileKind::BasicPc1600);
        let ce = convert::add_machine_header(&[1, 2, 3], Device::Pc1500, None, 0x4000, None).unwrap();
        assert_eq!(kind(&ce), FileKind::MlLh5801);
        let pc = convert::add_machine_header(&[1, 2, 3], Device::Pc1600, None, 0x4000, None).unwrap();
        assert_eq!(kind(&pc), FileKind::MlZ80);
        assert_eq!(kind(&LH5801_BLOCK.repeat(8)), FileKind::RawLh5801);
        assert_eq!(kind(&Z80_BLOCK.repeat(8)), FileKind::RawZ80);
        assert_eq!(kind(&pseudo_random(1024, 3)), FileKind::Raw);
        let layout = reserve::ReserveLayout { filename: None, labels: Default::default(), keys: Default::default() };
        let sdar = reserve::to_ascii(&layout);
        assert_eq!(kind(sdar.as_bytes()), FileKind::ReserveText);
        let payload = reserve::encode_payload(&layout, Registry::for_device(Device::Pc1500)).unwrap();
        let mut bin = header::build_header(header::BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Reserve,
            name: Some("KEYS"),
            payload_len: payload.len(),
            start_addr: 0,
            run_addr: 0,
        });
        bin.extend(payload);
        assert_eq!(kind(&bin), FileKind::Reserve);
        let vars = variables::VarFile { filename: None, values: Vec::new() };
        assert_eq!(kind(variables::to_ascii(&vars).as_bytes()), FileKind::VariablesText);
        let mut bin = header::build_header(header::BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Variables,
            name: Some("V"),
            payload_len: 0,
            start_addr: 0,
            run_addr: 0,
        });
        bin.extend(variables::encode_payload(&vars.values).unwrap());
        assert_eq!(kind(&bin), FileKind::Variables);
        assert_eq!(kind(b"hello world\n"), FileKind::Text);
    }

    #[test]
    fn classify_loader_fields() {
        let ce = convert::add_machine_header(&[1, 2, 3], Device::Pc1500, Some("PROG"), 0x38C5, Some(0x38C5)).unwrap();
        let s = classify(&ce);
        assert_eq!(
            (s.problems, s.payload_offset, s.payload_len, s.load_addr, s.run_addr, s.autorun),
            (0, 27, 3, 0x38C5, 0x38C5, true)
        );
        assert_eq!(s.name.as_deref(), Some("PROG"));

        let pc = convert::add_machine_header(&[1, 2], Device::Pc1600, None, 0x01_C000, None).unwrap();
        let s = classify(&pc);
        assert_eq!((s.payload_offset, s.payload_len, s.load_addr, s.run_addr, s.autorun), (16, 2, 0x01_C000, 0x01_FFFF, false));
        assert_eq!(s.name, None);

        let raw = classify(&[0x42; 40]);
        assert_eq!((raw.payload_offset, raw.payload_len, raw.load_addr), (0, 40, 0));
    }

    #[test]
    fn classify_problem_flags() {
        let ce = convert::add_machine_header(&[1, 2, 3], Device::Pc1500, None, 0x4000, None).unwrap();
        let cut = classify(&ce[..ce.len() - 1]);
        assert_eq!((cut.problems, cut.payload_len), (problem::TRUNCATED, 2));
        let mut long = ce.clone();
        long.push(0);
        assert_eq!(classify(&long).problems, problem::TRAILING);
        let mut noisy = vec![0, 0];
        noisy.extend(&ce);
        assert_eq!(classify(&noisy).problems, problem::LEADING_NOISE);
        let head = classify(&ce[..20]);
        assert_eq!((head.kind, head.problems), (FileKind::Raw, problem::HEADER_CUT));
        // Tokenized BASIC whose payload is garbage.
        let mut bad = header::build(Device::Pc1500, Some("X"), 4);
        bad.extend([0xFF, 0xFF, 0xFF, 0x00]);
        let bad = classify(&bad);
        assert_eq!(bad.kind, FileKind::BasicPc1500);
        assert_ne!(bad.problems & problem::BAD_PAYLOAD, 0, "{bad:?}");
    }

    #[test]
    fn describe_agrees_with_classify() {
        use crate::cpu_guess::samples::*;
        let samples: Vec<Vec<u8>> = vec![
            fixture("depreciation.bas"),
            fixture("depreciation-tokenized-ce158header.bin"),
            fixture("depreciation-tokenized-pc1600header.bin"),
            LH5801_BLOCK.repeat(8),
            Z80_BLOCK.repeat(8),
            pseudo_random(300, 9),
            b"plain\n".to_vec(),
            Vec::new(),
        ];
        for data in &samples {
            let (c, d) = (classify(data), describe(data));
            assert_eq!(get(&d, "kind"), c.kind.as_str());
            assert_eq!(d.warnings.is_empty(), c.problems == 0, "{d:?} {c:?}");
        }
    }
}
