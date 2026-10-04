//! PC-1500 / PC-1500A + CE-150 tape format.
//!
//! * Tones: "1" = 8 cycles of 2500 Hz, "0" = 4 cycles of 1250 Hz (3.2 ms per bit,
//!   nominal; the ROM's LH5811 timing gives ~2667 / 1263 Hz).
//! * Nibble frame: start "0", 4 data bits LSB first, stop "1"s (the ROM writes 6;
//!   a reader accepts any number). A byte is its low nibble, then its high nibble.
//! * File: leader of "1"s, ident nibble `A`, 40-byte header, 2-byte checksum, pause,
//!   body in 80-byte blocks each followed by a 2-byte checksum (the last, partial block
//!   too), pause, `55` end byte. Checksums are the 16-bit byte sum, big-endian.
//! * Header: `10`..`17`, sub-type (`00` ML, `01` BASIC, `02` RSV, `03` DEF, `04` DAT),
//!   16-byte name (NUL-padded), 9 zero bytes, start, length − 1, entry (all BE).
//! * A BASIC body ends with the `FF` end mark, which the length counts (verified on
//!   real-world and ROM-made recordings); machine code has none.

use super::encode::Synth;
use super::{find_steady, Leader, TapeError, TapeFile, TapeFormat, TapeIssue, TapeKind, Tracker};

const F1: f64 = 2500.0;
const F0: f64 = 1250.0;
const BIT_SECS: f64 = 8.0 / F1;
const BLOCK: usize = 80;
const HEADER_LEN: usize = 40;
/// Stop bits the ROM writes after every nibble.
const STOP_BITS: usize = 6;
/// "1" bits of the ROM's pauses after the header and around the end byte.
const PAUSE_BITS: usize = 78;
/// BASIC programs are stored at the start of a stock PC-1500's program area; `CLOAD`
/// relocates, so this is informational (it's what the ROM's own `CSAVE` writes).
const BASIC_START: u32 = 0x40C5;
const DEFAULT_LEADER_SECS: f64 = 2.0;
const ROM_LEADER_BITS: usize = 2540;

/// A demodulated bit with the time it starts.
#[derive(Clone, Copy)]
struct Bit {
    v: u8,
    t: f64,
}

pub(super) fn decode(edges: &[f64]) -> (Vec<TapeFile>, Vec<TapeIssue>) {
    let d: Vec<f64> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    let (mut files, mut issues) = (Vec::new(), Vec::new());
    let mut pos = 0;
    // Each pass: find a leader, demodulate up to the next silence, parse what's there.
    while let Some((start, short)) = find_steady(&d, pos, 256) {
        let (bits, end) = demodulate(&d, edges, start, short);
        parse_stream(&bits, nominal_speed(short), &mut files, &mut issues);
        pos = end.max(start + 1);
    }
    (files, issues)
}

/// Nominal "1" half-period over the measured one.
fn nominal_speed(short: f64) -> f64 {
    (0.5 / F1) / short
}

/// Half-periods → bits, by run length: a run of n long halves is n/8 "0" bits, a run of
/// n short halves n/16 "1" bits — independent of speed, and a glitch inside a run
/// rounds away. Stops at a gap (silence or dropout).
fn demodulate(d: &[f64], edges: &[f64], start: usize, short: f64) -> (Vec<Bit>, usize) {
    let mut tr = Tracker::new(short, 2.0);
    let mut bits = Vec::new();
    let mut i = start;
    let mut run_long = false;
    let mut run_len = 0usize;
    let mut run_t = edges[start];
    let flush = |bits: &mut Vec<Bit>, long: bool, n: usize, t: f64| {
        let (per, v) = if long { (8.0, 0) } else { (16.0, 1) };
        let count = (n as f64 / per).round() as usize;
        let dur = BIT_SECS; // only used for error positions
        for k in 0..count {
            bits.push(Bit { v, t: t + k as f64 * dur });
        }
    };
    while i < d.len() {
        if d[i] > tr.long * 4.0 {
            break; // silence / dropout ends the stream
        }
        let long = tr.classify(d[i]);
        if run_len > 0 && long != run_long {
            flush(&mut bits, run_long, run_len, run_t);
            run_len = 0;
        }
        if run_len == 0 {
            run_long = long;
            run_t = edges[i];
        }
        run_len += 1;
        i += 1;
    }
    if run_len > 0 {
        flush(&mut bits, run_long, run_len, run_t);
    }
    (bits, i + 1)
}

struct Reader<'a> {
    bits: &'a [Bit],
    k: usize,
}

impl Reader<'_> {
    fn time(&self) -> f64 {
        self.bits.get(self.k.min(self.bits.len().saturating_sub(1))).map_or(0.0, |b| b.t)
    }

    /// Next nibble frame: skip stop "1"s (at least `min_stop`), start "0", 4 data bits.
    fn nibble(&mut self, min_stop: usize) -> Option<u8> {
        let mut ones = 0;
        while self.k < self.bits.len() && self.bits[self.k].v == 1 {
            ones += 1;
            self.k += 1;
        }
        if ones < min_stop || self.k + 5 > self.bits.len() {
            return None;
        }
        let b = &self.bits[self.k + 1..self.k + 5];
        self.k += 5;
        Some(b[0].v | b[1].v << 1 | b[2].v << 2 | b[3].v << 3)
    }

    fn byte(&mut self) -> Option<u8> {
        let lo = self.nibble(1)?;
        let hi = self.nibble(1)?;
        Some(hi << 4 | lo)
    }

    fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        (0..n).map(|_| self.byte()).collect()
    }

    fn checksum(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.byte()?, self.byte()?]))
    }
}

fn sum16(b: &[u8]) -> u16 {
    b.iter().fold(0u16, |s, &x| s.wrapping_add(x as u16))
}

fn parse_stream(bits: &[Bit], speed: f64, files: &mut Vec<TapeFile>, issues: &mut Vec<TapeIssue>) {
    let mut r = Reader { bits, k: 0 };
    loop {
        // Ident nibble `A` after a run of "1"s (leader or the gap after a previous file).
        let ident_at = loop {
            if r.k >= bits.len() {
                return;
            }
            let save = r.k;
            match r.nibble(20) {
                Some(0xA) => break save,
                Some(_) => {}
                None => r.k = save + 1,
            }
        };
        let t0 = bits[ident_at].t;
        let after_ident = r.k;
        let Some(hdr) = r.bytes(HEADER_LEN) else {
            return;
        };
        if hdr[..8] != [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17] {
            // Not a header after all (noise that happened to look like an `A`).
            r.k = after_ident;
            continue;
        }
        let kind = match hdr[8] {
            0x00 => Some(TapeKind::Machine),
            0x01 => Some(TapeKind::Basic),
            0x02 => Some(TapeKind::Reserve),
            0x03 => Some(TapeKind::DefKeys),
            0x04 => Some(TapeKind::Data),
            _ => None,
        };
        let name = trim_name(&hdr[9..25]);
        let mut issue = |r: &Reader, msg: String| {
            issues.push(TapeIssue {
                format: TapeFormat::Pc1500Ce150,
                time: r.time(),
                name: (!name.is_empty()).then(|| name.clone()),
                kind,
                message: msg,
            })
        };
        match r.checksum() {
            Some(c) if c == sum16(&hdr) => {}
            Some(_) => {
                issue(&r, "header checksum error".into());
                continue;
            }
            None => {
                issue(&r, "tape ends inside the header".into());
                return;
            }
        }
        let Some(kind) = kind else {
            issue(&r, format!("unknown file type {:02X}", hdr[8]));
            continue;
        };
        let len = u16::from_be_bytes([hdr[36], hdr[37]]) as usize + 1;
        let mut body = Vec::with_capacity(len);
        let mut bad = None;
        while body.len() < len {
            let n = BLOCK.min(len - body.len());
            let Some(block) = r.bytes(n) else {
                bad = Some(format!("tape ends after {} of {len} bytes", body.len()));
                break;
            };
            match r.checksum() {
                Some(c) if c == sum16(&block) => body.extend_from_slice(&block),
                Some(_) => {
                    bad = Some(format!("checksum error in bytes {}..{}", body.len(), body.len() + n));
                    break;
                }
                None => {
                    bad = Some(format!("tape ends after {} of {len} bytes", body.len() + n));
                    break;
                }
            }
        }
        if let Some(msg) = bad {
            let msg = if matches!(kind, TapeKind::Data | TapeKind::DefKeys) {
                format!("{msg} (this file type is not supported)")
            } else {
                msg
            };
            issue(&r, msg);
            continue;
        }
        if kind == TapeKind::Basic {
            if body.last() != Some(&0xFF) {
                issue(&r, "BASIC program without its FF end mark".into());
                continue;
            }
            body.pop();
        }
        // The `55` end byte is optional to a reader.
        let save = r.k;
        if r.byte() != Some(0x55) {
            r.k = save;
        }
        files.push(TapeFile {
            format: TapeFormat::Pc1500Ce150,
            kind,
            name,
            load: u16::from_be_bytes([hdr[34], hdr[35]]) as u32,
            entry: u16::from_be_bytes([hdr[38], hdr[39]]) as u32,
            payload: body,
            header: hdr,
            start_time: t0,
            end_time: r.time(),
            speed,
        });
    }
}

pub(super) fn trim_name(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    crate::cp437::decode(&raw[..end]).trim_end().to_string()
}

pub(super) fn encode(s: &mut Synth, f: &TapeFile, leader: Leader) -> Result<(), TapeError> {
    let (sub, body, start, entry) = match f.kind {
        TapeKind::Basic => {
            let mut b = f.payload.clone();
            b.push(0xFF);
            (0x01, b, BASIC_START, 0)
        }
        TapeKind::Machine => (0x00, f.payload.clone(), f.load & 0xFFFF, f.entry & 0xFFFF),
        TapeKind::Reserve => (0x02, f.payload.clone(), f.load & 0xFFFF, 0),
        k => return Err(TapeError::Unsupported(format!("writing a {} to tape", k.describe()))),
    };
    if body.is_empty() {
        // The length field stores count - 1, so it can't express an empty file.
        return Err(TapeError::Unsupported("an empty file can't be written to a PC-1500 tape".into()));
    }
    if body.len() > 0x1_0000 {
        return Err(TapeError::Unsupported(format!("{} bytes is too long for a PC-1500 tape", body.len())));
    }
    let mut hdr = vec![0u8; HEADER_LEN];
    hdr[..8].copy_from_slice(&[0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17]);
    hdr[8] = sub;
    let name = crate::cp437::encode_lossy(&f.name);
    let n = name.len().min(16);
    hdr[9..9 + n].copy_from_slice(&name[..n]);
    hdr[34..36].copy_from_slice(&(start as u16).to_be_bytes());
    hdr[36..38].copy_from_slice(&((body.len() - 1) as u16).to_be_bytes());
    hdr[38..40].copy_from_slice(&(entry as u16).to_be_bytes());

    let leader_bits = match leader {
        Leader::Default => (DEFAULT_LEADER_SECS / BIT_SECS).round() as usize,
        Leader::Rom => ROM_LEADER_BITS,
        Leader::Seconds(secs) => ((secs as f64 / BIT_SECS).round() as usize).max(1),
    };
    let mut w = Writer { s };
    w.s.silence(0.5);
    for _ in 0..leader_bits {
        w.bit(1);
    }
    w.nibble(0xA);
    for &b in &hdr {
        w.byte(b);
    }
    w.word(sum16(&hdr));
    w.pause();
    for block in body.chunks(BLOCK) {
        for &b in block {
            w.byte(b);
        }
        w.word(sum16(block));
    }
    w.pause();
    w.byte(0x55);
    w.pause();
    w.s.silence(0.5);
    Ok(())
}

struct Writer<'a> {
    s: &'a mut Synth,
}

impl Writer<'_> {
    fn bit(&mut self, v: u8) {
        if v == 1 {
            self.s.tone(F1, 8.0);
        } else {
            self.s.tone(F0, 4.0);
        }
    }

    fn nibble(&mut self, n: u8) {
        self.bit(0);
        for i in 0..4 {
            self.bit(n >> i & 1);
        }
        for _ in 0..STOP_BITS {
            self.bit(1);
        }
    }

    fn byte(&mut self, b: u8) {
        self.nibble(b & 0x0F);
        self.nibble(b >> 4);
    }

    fn word(&mut self, w: u16) {
        self.byte((w >> 8) as u8);
        self.byte(w as u8);
    }

    fn pause(&mut self) {
        for _ in 0..PAUSE_BITS {
            self.bit(1);
        }
    }
}
