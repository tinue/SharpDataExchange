//! PC-1600 + CE-1600P tape format, MODE 0 (CE-1600P ROM bank 5, 6000H–).
//!
//! * One cycle per bit: "0" ≈ 3000 Hz, "1" ≈ 1200 Hz (`CMTONE0` / `CMTONE1`). The ROM's
//!   half periods are unequal, so whole cycles are classified, not half cycles.
//! * Sync (`CMLEADER` / `CMSYNC`): a run of "0"s (ROM: 10000 before the header, 11000
//!   before the data; `CLOAD` needs 5000), 40 (data: 20) "1"s, 40 (20) "0"s, one "1".
//! * Byte (`CMWRBYTE`): start "1", then 8 bits MSB first; no stop bit.
//! * Checksum: 16-bit count of 1 bits (big-endian) after the 48-byte header, and once
//!   after the *whole* data stream (`CMWRCSUM`), each followed by one closing "1".
//! * Header (`CASHDRFILL`): `00` type (01 ML, 02 BASIC, 04 ASCII, 08 data), `01`-`10`
//!   name (NUL-padded), `11` = `0D`, `12` length, `14` load, `16` entry (LE16), `18`
//!   MODE-2 type (00 ML, 01 BASIC, 02 RESERVE), `19`-`1C` month/day/hour/minute,
//!   `1D`/`1E`/`1F` the top bytes of length/load/entry (`FFFF`+`FF` = no entry).

use super::encode::Synth;
use super::{find_steady, Leader, TapeError, TapeFile, TapeFormat, TapeIssue, TapeKind, Tracker};

const F0: f64 = 3000.0;
const F1: f64 = 1200.0;
const HEADER_LEN: usize = 48;
const DEFAULT_LEADER_SECS: f64 = 3.0;
const ROM_HEADER_LEADER: usize = 10_000;
const ROM_DATA_LEADER: usize = 11_000;
const DEFAULT_DATA_LEADER: usize = 7_000;
/// `CLOAD` counts 5000 leader cycles (`F1AAH`) before it looks for the sync mark.
const MIN_DATA_LEADER: usize = 6_000;
/// Date written into the header: 1 January, 00:00 (what an unset clock gives).
const DATE: [u8; 4] = [1, 1, 0, 0];

struct Block {
    t: f64,
    bytes: Vec<u8>,
    speed: f64,
    end: f64,
}

pub(super) fn decode(edges: &[f64]) -> (Vec<TapeFile>, Vec<TapeIssue>) {
    // Whole cycles run edge-to-edge of the same direction. Which direction starts a bit
    // depends on the recording's polarity, so try both and keep the better reading.
    let mut best: Option<(Vec<TapeFile>, Vec<TapeIssue>)> = None;
    for phase in 0..2 {
        let t: Vec<f64> = edges.iter().skip(phase).step_by(2).copied().collect();
        let r = decode_cycles(&t);
        let better = match &best {
            None => true,
            Some(b) => r.0.len() > b.0.len() || (r.0.len() == b.0.len() && r.1.len() < b.1.len()),
        };
        if better {
            best = Some(r);
        }
    }
    best.unwrap_or_default()
}

fn decode_cycles(t: &[f64]) -> (Vec<TapeFile>, Vec<TapeIssue>) {
    let c: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    let mut blocks = Vec::new();
    let mut pos = 0;
    while let Some((start, short)) = find_steady(&c, pos, 256) {
        let mut tr = Tracker::new(short, 2.5);
        let mut bits = Vec::new();
        let mut i = start;
        while i < c.len() && c[i] < tr.long * 3.0 {
            bits.push((tr.classify(c[i]) as u8, t[i]));
            i += 1;
        }
        let speed = (1.0 / F0) / short;
        read_blocks(&bits, speed, &mut blocks);
        pos = i + 1;
    }
    interpret(&blocks)
}

/// Every sync-introduced byte run in one stream of bits.
fn read_blocks(bits: &[(u8, f64)], speed: f64, out: &mut Vec<Block>) {
    let run = |k: usize, v: u8| bits[k..].iter().take_while(|b| b.0 == v).count();
    let mut k = 0;
    while k < bits.len() {
        let zeros = run(k, 0);
        if zeros < 100 {
            k += zeros.max(1);
            continue;
        }
        let a = k + zeros;
        let ones = run(a, 1);
        let b = a + ones;
        let zeros2 = if b < bits.len() { run(b, 0) } else { 0 };
        let mut p = b + zeros2;
        if !(15..=60).contains(&ones) || !(15..=60).contains(&zeros2) || p >= bits.len() {
            k = a;
            continue;
        }
        p += 1; // the sync-end "1"
        let t0 = bits[p.min(bits.len() - 1)].1;
        let mut bytes = Vec::new();
        while p + 9 <= bits.len() && bits[p].0 == 1 {
            let v = bits[p + 1..p + 9].iter().fold(0u8, |acc, b| acc << 1 | b.0);
            bytes.push(v);
            p += 9;
        }
        let end = bits[p.min(bits.len() - 1)].1;
        out.push(Block { t: t0, bytes, speed, end });
        k = p.max(a);
    }
}

fn popcount16(b: &[u8]) -> u16 {
    b.iter().map(|x| x.count_ones() as u16).fold(0u16, u16::wrapping_add)
}

fn interpret(blocks: &[Block]) -> (Vec<TapeFile>, Vec<TapeIssue>) {
    let (mut files, mut issues) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < blocks.len() {
        let hb = &blocks[i];
        i += 1;
        let h = &hb.bytes;
        // Only something that looks like a header counts; other blocks (the data of a
        // file whose header was lost, ASCII blocks) are passed over.
        if h.len() < HEADER_LEN + 2 || h[0x11] != 0x0D || ![1, 2, 4, 8].contains(&h[0]) {
            continue;
        }
        let name = super::pc1500::trim_name(&h[1..17]);
        let kind = match (h[0], h[0x18]) {
            (0x02, 0x02) => Some(TapeKind::Reserve),
            (0x02, _) => Some(TapeKind::Basic),
            (0x01, _) => Some(TapeKind::Machine),
            (0x04, _) => Some(TapeKind::Ascii),
            (0x08, _) => Some(TapeKind::Data),
            _ => None,
        };
        let mut issue = |time: f64, msg: String| {
            issues.push(TapeIssue {
                format: TapeFormat::Pc1600Ce1600p,
                time,
                name: (!name.is_empty()).then(|| name.clone()),
                kind,
                message: msg,
            })
        };
        if popcount16(&h[..HEADER_LEN]) != u16::from_be_bytes([h[48], h[49]]) {
            issue(hb.t, "header checksum error".into());
            continue;
        }
        let kind = match kind {
            Some(k @ (TapeKind::Basic | TapeKind::Machine | TapeKind::Reserve)) => k,
            Some(k) => {
                issue(hb.t, format!("{} files are not supported", k.describe()));
                continue;
            }
            None => unreachable!(),
        };
        let len = u32::from_le_bytes([h[0x12], h[0x13], h[0x1D], 0]) as usize;
        let Some(db) = blocks.get(i) else {
            issue(hb.end, "tape ends before the data".into());
            break;
        };
        i += 1;
        let d = &db.bytes;
        if d.len() < len + 2 {
            issue(db.t, format!("data breaks off after {} of {len} bytes", d.len().saturating_sub(2).min(len)));
            continue;
        }
        if popcount16(&d[..len]) != u16::from_be_bytes([d[len], d[len + 1]]) {
            issue(db.t, "data checksum error".into());
            continue;
        }
        files.push(TapeFile {
            format: TapeFormat::Pc1600Ce1600p,
            kind,
            name,
            load: u32::from_le_bytes([h[0x14], h[0x15], h[0x1E], 0]),
            entry: u32::from_le_bytes([h[0x16], h[0x17], h[0x1F], 0]),
            payload: d[..len].to_vec(),
            header: h[..HEADER_LEN].to_vec(),
            start_time: hb.t,
            end_time: db.end,
            speed: (hb.speed + db.speed) / 2.0,
        });
    }
    (files, issues)
}

pub(super) fn encode(s: &mut Synth, f: &TapeFile, leader: Leader) -> Result<(), TapeError> {
    let (t1, t2) = match f.kind {
        TapeKind::Basic => (0x02, 0x01),
        TapeKind::Machine => (0x01, 0x00),
        TapeKind::Reserve => (0x02, 0x02),
        k => return Err(TapeError::Unsupported(format!("writing a {} to tape", k.describe()))),
    };
    let len = f.payload.len() as u32;
    if len >= 0x100_0000 {
        return Err(TapeError::Unsupported(format!("{len} bytes is too long for a PC-1600 tape")));
    }
    let (load, entry) = match f.kind {
        TapeKind::Machine if f.entry & 0xFFFF == 0xFFFF => (f.load, 0xFF_FFFF),
        TapeKind::Machine => (f.load, f.entry),
        _ => (0, 0),
    };
    let mut h = vec![0u8; HEADER_LEN];
    h[0] = t1;
    let name = crate::cp437::encode_lossy(&f.name);
    let n = name.len().min(16);
    h[1..1 + n].copy_from_slice(&name[..n]);
    h[0x11] = 0x0D;
    let (l, a, e) = (len.to_le_bytes(), load.to_le_bytes(), entry.to_le_bytes());
    h[0x12..0x14].copy_from_slice(&l[..2]);
    h[0x14..0x16].copy_from_slice(&a[..2]);
    h[0x16..0x18].copy_from_slice(&e[..2]);
    h[0x18] = t2;
    h[0x19..0x1D].copy_from_slice(&DATE);
    h[0x1D] = l[2];
    h[0x1E] = a[2];
    h[0x1F] = e[2];

    let (lead_h, lead_d) = match leader {
        Leader::Default => ((DEFAULT_LEADER_SECS * F0) as usize, DEFAULT_DATA_LEADER),
        Leader::Rom => (ROM_HEADER_LEADER, ROM_DATA_LEADER),
        Leader::Seconds(secs) => {
            let n = ((secs as f64 * F0).round() as usize).max(1);
            (n, (n * ROM_DATA_LEADER / ROM_HEADER_LEADER).max(MIN_DATA_LEADER))
        }
    };
    let mut w = Writer { s };
    w.s.silence(0.5);
    w.sync(lead_h, 40);
    w.block(&h);
    w.s.silence(0.5);
    w.sync(lead_d, 20);
    w.block(&f.payload);
    w.s.silence(0.5);
    Ok(())
}

struct Writer<'a> {
    s: &'a mut Synth,
}

impl Writer<'_> {
    fn bit(&mut self, v: u8) {
        self.s.tone(if v == 1 { F1 } else { F0 }, 1.0);
    }

    fn sync(&mut self, leader: usize, marks: usize) {
        for _ in 0..leader {
            self.bit(0);
        }
        for _ in 0..marks {
            self.bit(1);
        }
        for _ in 0..marks {
            self.bit(0);
        }
        self.bit(1);
    }

    fn byte(&mut self, b: u8) {
        self.bit(1);
        for i in (0..8).rev() {
            self.bit(b >> i & 1);
        }
    }

    fn block(&mut self, data: &[u8]) {
        for &b in data {
            self.byte(b);
        }
        let c = popcount16(data);
        self.byte((c >> 8) as u8);
        self.byte(c as u8);
        self.bit(1);
    }
}
