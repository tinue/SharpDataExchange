//! Heuristic CPU identification for headerless machine code: LH5801 (PC-1500 family,
//! also the PC-1600's LH5803) vs Z80 (the PC-1600's SC7852). Nothing in a bare binary
//! says which CPU it is for, so this is a guess, and callers must present it as one.
//!
//! The code is decoded linearly from offset 0 under each instruction set (opcode
//! lengths and holes follow Calc-U-1600's `LH5801Disassembler.cpp` /
//! `Z80Disassembler.cpp`), collecting per-ISA [`IsaStats`]:
//!
//! * **invalid** — undocumented opcodes. The LH5801 map has many holes, so real LH5801
//!   code has almost none while Z80 code and data hit them often. The Z80 map is nearly
//!   full, so this says little the other way.
//! * **branch hits** — relative branches (LH5801 `Bxx`/`LOP`, Z80 `JR`/`DJNZ`) whose
//!   target lies inside the file on an instruction boundary of the same sweep. Needs no
//!   load address, and code decoded under the wrong ISA rarely lines up.
//! * **signatures** — instructions typical of real code (returns, calls, loads of
//!   immediates, …) as a share of all instructions.

/// A CPU [`guess_cpu`] can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cpu {
    Lh5801,
    Z80,
}

/// What a linear sweep under one instruction set found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IsaStats {
    pub instructions: usize,
    pub invalid: usize,
    pub signatures: usize,
    pub branches: usize,
    pub branch_hits: usize,
}

impl IsaStats {
    pub fn invalid_rate(&self) -> f64 {
        ratio(self.invalid, self.instructions)
    }
    pub fn signature_rate(&self) -> f64 {
        ratio(self.signatures, self.instructions)
    }
    pub fn branch_hit_rate(&self) -> f64 {
        ratio(self.branch_hits, self.branches)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

/// The guess plus the numbers it was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuGuess {
    /// `None`: too short, or neither instruction set fits well enough.
    pub cpu: Option<Cpu>,
    /// Whole-file sweeps.
    pub lh5801: IsaStats,
    pub z80: IsaStats,
    /// Per-window votes (see [`WINDOW`]): LH5801, Z80, undecided.
    pub votes: (usize, usize, usize),
}

/// Inputs shorter than this are never guessed.
pub const MIN_LEN: usize = 16;
/// Longer inputs are judged in windows of this size, so data tables mixed into a
/// program don't drown out its code sections.
pub const WINDOW: usize = 512;

/// Guess which CPU `code` is for. See the module docs.
pub fn guess_cpu(code: &[u8]) -> CpuGuess {
    let lh5801 = sweep(code, lh5801_decode);
    let z80 = sweep(code, z80_decode);
    if code.len() < MIN_LEN {
        return CpuGuess { cpu: None, lh5801, z80, votes: (0, 0, 0) };
    }
    let windows: Vec<&[u8]> = if code.len() < 2 * WINDOW {
        vec![code]
    } else {
        // Fold a short tail into the last window.
        let n = code.len() / WINDOW;
        (0..n).map(|i| &code[i * WINDOW..if i + 1 == n { code.len() } else { (i + 1) * WINDOW }]).collect()
    };
    let mut votes = (0, 0, 0);
    for w in &windows {
        match judge(&sweep(w, lh5801_decode), &sweep(w, z80_decode)) {
            Some(Cpu::Lh5801) => votes.0 += 1,
            Some(Cpu::Z80) => votes.1 += 1,
            None => votes.2 += 1,
        }
    }
    let (lh, z, _) = votes;
    // A third of the windows, but three agreeing windows always suffice: a window of
    // random bytes practically never votes, so the rest are data tables.
    let needed = windows.len().div_ceil(3).min(3);
    let cpu = if lh >= needed && z * 4 <= lh {
        Some(Cpu::Lh5801)
    } else if z >= needed && lh * 4 <= z {
        Some(Cpu::Z80)
    } else {
        None
    };
    CpuGuess { cpu, lh5801, z80, votes }
}

// Baselines for uniformly random bytes, measured over 4 KB samples, and calibrated
// against Z80 programs (PC-1600 `BSAVE` files), the PC-1500 ROM (LH5801) and random
// data: real LH5801 code has <= 5% invalid opcodes and >= 85% of its branches land;
// real Z80 code has 30-60% signature instructions.
const RANDOM_LH_INVALID: f64 = 0.15;
const RANDOM_LH_BRANCH_HITS: f64 = 0.74;
const RANDOM_Z80_SIGNATURES: f64 = 0.21;
/// How many standard deviations from the random baseline count as evidence: for Z80 on
/// one test, for LH5801 on each of two independent ones.
const Z80_SIGMAS: f64 = 4.0;
const LH_SIGMAS: f64 = 2.0;

/// One window's verdict.
fn judge(lh: &IsaStats, z: &IsaStats) -> Option<Cpu> {
    let lh_like = lh.instructions >= 20
        && lh.invalid_rate() <= 0.05
        && below_baseline(lh.invalid, lh.instructions, RANDOM_LH_INVALID, LH_SIGMAS)
        && lh.branch_hit_rate() >= 0.85
        && above_baseline(lh.branch_hits, lh.branches, RANDOM_LH_BRANCH_HITS, LH_SIGMAS);
    if lh_like {
        return Some(Cpu::Lh5801);
    }
    let z80_like = z.invalid_rate() <= 0.01
        && lh.invalid_rate() >= 0.06
        && z.signature_rate() >= 0.30
        && above_baseline(z.signatures, z.instructions, RANDOM_Z80_SIGNATURES, Z80_SIGMAS);
    z80_like.then_some(Cpu::Z80)
}

/// `hits` out of `n` is at least `sigmas` standard deviations above rate `p`.
fn above_baseline(hits: usize, n: usize, p: f64, sigmas: f64) -> bool {
    n > 0 && (ratio(hits, n) - p) >= sigmas * (p * (1.0 - p) / n as f64).sqrt()
}

/// `hits` out of `n` is at least `sigmas` standard deviations below rate `p`.
fn below_baseline(hits: usize, n: usize, p: f64, sigmas: f64) -> bool {
    n > 0 && (p - ratio(hits, n)) >= sigmas * (p * (1.0 - p) / n as f64).sqrt()
}

/// One decoded instruction: its length, whether it is undocumented, whether it is a
/// signature instruction, and a relative-branch target (as an offset from the start of
/// the instruction) if it is one.
struct Insn {
    len: usize,
    invalid: bool,
    signature: bool,
    branch: Option<isize>,
}

fn sweep(code: &[u8], decode: fn(&[u8]) -> Insn) -> IsaStats {
    let mut s = IsaStats::default();
    let mut starts = vec![false; code.len()];
    let mut targets = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let insn = decode(&code[pc..]);
        if pc + insn.len > code.len() {
            break; // a truncated final instruction is not counted
        }
        starts[pc] = true;
        s.instructions += 1;
        s.invalid += insn.invalid as usize;
        s.signatures += insn.signature as usize;
        if let Some(rel) = insn.branch {
            s.branches += 1;
            targets.push(pc as isize + rel);
        }
        pc += insn.len;
    }
    s.branch_hits = targets
        .iter()
        .filter(|&&t| t >= 0 && (t as usize) < code.len() && starts[t as usize])
        .count();
    s
}

fn byte(code: &[u8], i: usize) -> u8 {
    code.get(i).copied().unwrap_or(0)
}

// ---- LH5801 ----

/// LH5801 instruction lengths, `0` = undocumented. `LH_MAIN` for unprefixed opcodes,
/// `LH_FD` for the byte after an `FD` prefix (length includes the prefix).
const LH_MAIN: [u8; 256] = lh_main();
const LH_FD: [u8; 256] = lh_fd();

const fn lh_main() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut k = 0;
    while k < 3 {
        let b = k << 4;
        let mut i = 0;
        while i < 16 {
            t[b + i] = 1; // 00-2F register / (Rreg) block
            i += 1;
        }
        let mut i = 0x40;
        while i < 0x48 {
            t[b + i] = 1; // inc/dec, block moves
            i += 1;
        }
        let mut i = 0x48;
        while i < 0x50 {
            t[b + i] = 2; // immediates
            i += 1;
        }
        t[b + 0x80] = 1;
        t[b + 0x82] = 1;
        t[b + 0x84] = 1;
        t[b + 0x86] = 1;
        t[b + 0x8C] = 1;
        k += 1;
    }
    // (pp) absolute forms
    let abs = [0xA1, 0xA3, 0xA5, 0xA7, 0xA9, 0xAB, 0xAD, 0xAE, 0xAF];
    let mut i = 0;
    while i < abs.len() {
        t[abs[i]] = 3;
        i += 1;
    }
    let abs_imm = [0xE9, 0xEB, 0xED, 0xEF];
    let mut i = 0;
    while i < abs_imm.len() {
        t[abs_imm[i]] = 4;
        i += 1;
    }
    // accumulator immediates and inherent forms
    let imm = [0xB1, 0xB3, 0xB5, 0xB7, 0xB9, 0xBB, 0xBD, 0xBF];
    let mut i = 0;
    while i < imm.len() {
        t[imm[i]] = 2;
        i += 1;
    }
    let one = [
        0x38, 0xA8, 0xB8, 0xE1, 0xE3, 0xF9, 0xFB, 0xD1, 0xD5, 0xD9, 0xDB, 0xD3, 0xD7, 0xDD, 0xDF,
        0xF1, 0xF5, 0xF7, 0x9A, 0x8A,
    ];
    let mut i = 0;
    while i < one.len() {
        t[one[i]] = 1;
        i += 1;
    }
    t[0xAA] = 3; // ldi s,nn
    t[0xBA] = 3; // jmp
    t[0xBE] = 3; // sjp
    // relative branches, lop, vector calls
    let two = [
        0x8E, 0x9E, 0x81, 0x91, 0x83, 0x93, 0x85, 0x95, 0x87, 0x97, 0x89, 0x99, 0x8B, 0x9B, 0x8D,
        0x9D, 0x8F, 0x9F, 0x88, 0xCD, 0xC1, 0xC3, 0xC5, 0xC7, 0xC9, 0xCB, 0xCF,
    ];
    let mut i = 0;
    while i < two.len() {
        t[two[i]] = 2;
        i += 1;
    }
    let mut op = 0xC0;
    while op <= 0xFE {
        t[op] = 1; // vej: the opcode is the vector index
        op += 2;
    }
    t
}

const fn lh_fd() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut k = 0;
    while k < 3 {
        let b = k << 4;
        let two = [0x01, 0x03, 0x05, 0x07, 0x09, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x8C, 0x0A, 0x40, 0x42, 0x88, 0xCA];
        let mut i = 0;
        while i < two.len() {
            t[b + two[i]] = 2;
            i += 1;
        }
        t[b + 0x49] = 3;
        t[b + 0x4B] = 3;
        t[b + 0x4D] = 3;
        t[b + 0x4F] = 3;
        k += 1;
    }
    let abs = [0xA1, 0xA3, 0xA5, 0xA7, 0xA9, 0xAB, 0xAD, 0xAE, 0xAF];
    let mut i = 0;
    while i < abs.len() {
        t[abs[i]] = 4;
        i += 1;
    }
    let abs_imm = [0xE9, 0xEB, 0xED, 0xEF];
    let mut i = 0;
    while i < abs_imm.len() {
        t[abs_imm[i]] = 5;
        i += 1;
    }
    let two = [
        0x18, 0x28, 0x48, 0x58, 0x5A, 0x6A, 0x4E, 0x5E, 0xC8, 0x8A, 0xEC, 0xAA, 0xD3, 0xD7, 0x81,
        0xBE, 0xC1, 0xC0, 0xCE, 0xDE, 0xCC, 0xBA, 0x8E, 0xB1, 0x4C,
    ];
    let mut i = 0;
    while i < two.len() {
        t[two[i]] = 2;
        i += 1;
    }
    t
}

fn lh5801_decode(code: &[u8]) -> Insn {
    let op = byte(code, 0);
    if op == 0xFD {
        let op2 = byte(code, 1);
        let len = LH_FD[op2 as usize] as usize;
        return if len == 0 {
            Insn { len: 2, invalid: true, signature: false, branch: None }
        } else {
            // Any valid FD form is typical LH5801 (ME1 access, psh/pop, stack moves).
            Insn { len, invalid: false, signature: true, branch: None }
        };
    }
    let len = LH_MAIN[op as usize] as usize;
    if len == 0 {
        return Insn { len: 1, invalid: true, signature: false, branch: None };
    }
    let e = byte(code, 1) as isize;
    let branch = match op {
        // forward: target = address after the 2-byte instruction + e
        0x81 | 0x83 | 0x85 | 0x87 | 0x89 | 0x8B | 0x8D | 0x8F | 0x8E => Some(2 + e),
        // backward (and lop): target = address after the instruction - e
        0x91 | 0x93 | 0x95 | 0x97 | 0x99 | 0x9B | 0x9D | 0x9F | 0x9E | 0x88 => Some(2 - e),
        _ => None,
    };
    let signature = matches!(
        op,
        0x9A | 0xBE | 0xBA | 0xCD | 0xB5 | 0x48 | 0x4A | 0x58 | 0x5A | 0x68 | 0x6A | 0xA5 | 0xAE
    );
    Insn { len, invalid: false, signature, branch }
}

// ---- Z80 ----

fn z80_decode(code: &[u8]) -> Insn {
    let mut op = byte(code, 0);
    let mut pre = 0;
    let mut index = false;
    if op == 0xDD || op == 0xFD {
        let next = byte(code, 1);
        if next == 0xDD || next == 0xFD {
            return Insn { len: 1, invalid: true, signature: false, branch: None };
        }
        pre = 1;
        index = next != 0xED;
        op = next;
    }
    if op == 0xED {
        let op2 = byte(code, pre + 1);
        let (x, y, z) = (op2 >> 6, (op2 >> 3) & 7, op2 & 7);
        let valid = match x {
            1 => !(z == 7 && y >= 6),
            2 => z <= 3 && y >= 4,
            _ => false,
        };
        let len = pre + if x == 1 && z == 3 { 4 } else { 2 };
        // ld rr,(nn) / (nn),rr and the block moves are typical Z80.
        let signature = valid && ((x == 1 && z == 3) || x == 2);
        return Insn { len, invalid: !valid, signature, branch: None };
    }
    if op == 0xCB {
        return Insn { len: pre + if index { 3 } else { 2 }, invalid: false, signature: false, branch: None };
    }
    let (x, y, z, q, p) = (op >> 6, (op >> 3) & 7, op & 7, (op >> 3) & 1, op >> 4 & 3);
    let disp = |uses_mem: bool| usize::from(index && uses_mem);
    let e = byte(code, pre + 1) as i8 as isize;
    let (len, branch, signature) = match x {
        0 => match z {
            0 if y >= 2 => (2, Some(2 + e), true), // djnz, jr, jr cc
            0 => (1, None, false),
            1 if q == 0 => (3, None, true), // ld rr,nn
            1 => (1, None, false),
            2 if y >= 4 => (3, None, y == 6 || y == 7), // ld (nn),a / ld a,(nn)
            2 => (1, None, false),
            3 => (1, None, false),
            4 | 5 => (1 + disp(y == 6), None, false),
            6 => (2 + disp(y == 6), None, y == 7), // ld r,n; ld a,n
            _ => (1, None, false),
        },
        1 => (1 + disp((y == 6 || z == 6) && !(y == 6 && z == 6)), None, false),
        2 => (1 + disp(z == 6), None, false),
        _ => match z {
            0 => (1, None, true), // ret cc
            1 if q == 0 => (1, None, true), // pop
            1 => (1, None, p == 0), // ret, exx, jp (hl), ld sp,hl
            2 => (3, None, true), // jp cc,nn
            3 => match y {
                0 => (3, None, true), // jp nn
                2 | 3 => (2, None, false), // out (n),a / in a,(n)
                _ => (1, None, y == 5), // ex de,hl
            },
            4 => (3, None, true), // call cc,nn
            5 if q == 0 => (1, None, true), // push
            5 => (3, None, true), // call nn (p != 0 are prefixes, handled above)
            6 => (2, None, false),
            _ => (1, None, false), // rst
        },
    };
    let branch = branch.map(|b| b + pre as isize);
    Insn { len: pre + len, invalid: false, signature, branch }
}

/// Hand-assembled code for tests (here and in `info`).
#[cfg(test)]
pub(crate) mod samples {
    /// ldi xh/xl/ul; loop: lda (x); sin x; lop ul,loop; cpi a,0D; bzs +1; inc a; rtn
    pub const LH5801_BLOCK: [u8; 16] =
        [0x48, 0x76, 0x4A, 0x00, 0x6A, 0x04, 0x05, 0x41, 0x88, 0x04, 0xB7, 0x0D, 0x8B, 0x01, 0xDD, 0x9A];
    /// ld hl,8000; ld b,10; loop: ld a,(hl); cp 20; jr z,+1; inc hl; djnz loop;
    /// call 1000; ret
    pub const Z80_BLOCK: [u8; 17] =
        [0x21, 0x00, 0x80, 0x06, 0x10, 0x7E, 0xFE, 0x20, 0x28, 0x01, 0x23, 0x10, 0xF8, 0xCD, 0x00, 0x10, 0xC9];

    pub fn pseudo_random(len: usize, mut seed: u32) -> Vec<u8> {
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::samples::*;

    #[test]
    fn lh5801_code() {
        let code = LH5801_BLOCK.repeat(8);
        let g = guess_cpu(&code);
        assert_eq!(g.cpu, Some(Cpu::Lh5801), "{g:?}");
        assert_eq!((g.lh5801.invalid, g.lh5801.branches, g.lh5801.branch_hits), (0, 16, 16));
    }

    #[test]
    fn z80_code() {
        let code = Z80_BLOCK.repeat(8);
        let g = guess_cpu(&code);
        assert_eq!(g.cpu, Some(Cpu::Z80), "{g:?}");
        assert_eq!((g.z80.branches, g.z80.branch_hits), (16, 16));
    }

    #[test]
    fn code_among_data_windows_still_counts() {
        // Three windows of Z80 code among nine of random data.
        let mut data = Vec::new();
        for i in 0..12 {
            if i % 4 == 0 {
                data.extend(Z80_BLOCK.repeat(WINDOW / Z80_BLOCK.len() + 1).into_iter().take(WINDOW));
            } else {
                data.extend(pseudo_random(WINDOW, i));
            }
        }
        let g = guess_cpu(&data);
        assert_eq!((g.cpu, g.votes.0, g.votes.1), (Some(Cpu::Z80), 0, 3), "{g:?}");
    }

    #[test]
    fn random_bytes_and_padding_are_not_guessed() {
        for seed in 1..20 {
            assert_eq!(guess_cpu(&pseudo_random(4096, seed)).cpu, None, "seed {seed}");
            assert_eq!(guess_cpu(&pseudo_random(128, seed)).cpu, None, "seed {seed}");
        }
        assert_eq!(guess_cpu(&[0u8; 600]).cpu, None);
        assert_eq!(guess_cpu(&[0xFFu8; 600]).cpu, None);
    }

    #[test]
    fn short_input_is_never_guessed() {
        assert_eq!(guess_cpu(&LH5801_BLOCK[..MIN_LEN - 1]).cpu, None);
    }

    #[test]
    fn random_baselines_match_the_tables() {
        // RANDOM_LH_INVALID is the share of undocumented LH5801 opcodes among uniformly
        // random instruction starts (FD-prefixed ones weighted by 1/256).
        let main_holes = LH_MAIN.iter().filter(|&&l| l == 0).count() as f64;
        let fd_holes = LH_FD.iter().filter(|&&l| l == 0).count() as f64;
        let p = (main_holes + fd_holes / 256.0) / 256.0;
        assert!((p - RANDOM_LH_INVALID).abs() < 0.03, "LH5801 hole share {p:.3}");
        let g = guess_cpu(&pseudo_random(1 << 16, 7));
        assert!((g.z80.signature_rate() - RANDOM_Z80_SIGNATURES).abs() < 0.03, "{g:?}");
        assert!((g.lh5801.branch_hit_rate() - RANDOM_LH_BRANCH_HITS).abs() < 0.05, "{g:?}");
    }
}
