//! C ABI for [`crate::info::classify`]: what a buffer holds, as a stable one-word token
//! (for e.g. a program loader) plus where its payload is and where it goes.

use std::ffi::c_char;

use super::{guard, slice, SDE_ERR_ARGS, SDE_OK};
use crate::info::{self, problem};

/// `SdeFileInfo::problems`: the payload is shorter than the header says.
pub const SDE_PROBLEM_TRUNCATED: u32 = 1;
/// Bytes follow the payload.
pub const SDE_PROBLEM_TRAILING: u32 = 2;
/// `00` bytes precede the header.
pub const SDE_PROBLEM_LEADING_NOISE: u32 = 4;
/// Starts with header magic, but no complete header with a known type follows.
pub const SDE_PROBLEM_HEADER_CUT: u32 = 8;
/// Tokenized BASIC that doesn't de-tokenize, or Reserve Area / Variables that don't decode.
pub const SDE_PROBLEM_BAD_PAYLOAD: u32 = 16;
/// The problems that make a file unusable (`sde_file_kind` reports these as `damaged`).
pub const SDE_PROBLEM_FATAL: u32 = 25;

// Literals above so cbindgen can emit them; they must match `info::problem`.
const _: () = assert!(
    SDE_PROBLEM_TRUNCATED == problem::TRUNCATED
        && SDE_PROBLEM_TRAILING == problem::TRAILING
        && SDE_PROBLEM_LEADING_NOISE == problem::LEADING_NOISE
        && SDE_PROBLEM_HEADER_CUT == problem::HEADER_CUT
        && SDE_PROBLEM_BAD_PAYLOAD == problem::BAD_PAYLOAD
        && SDE_PROBLEM_FATAL == problem::FATAL
);

/// Bytes in `SdeFileInfo::name`: 16 CP437 characters as UTF-8, plus the NUL.
pub const SDE_FILE_NAME_SIZE: usize = 49;

/// What a buffer holds (`sde_file_info`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SdeFileInfo {
    /// Static token, as `sde_file_kind` returns it but never `damaged`: check `problems`.
    pub kind: *const c_char,
    /// `SDE_PROBLEM_*` bits.
    pub problems: u32,
    /// First payload byte in the input: after the header, `0` for headerless content.
    pub payload_offset: usize,
    /// Payload bytes present in the input (never past its end, even when truncated).
    pub payload_len: usize,
    /// `ml-*`: load address (PC-1600: bank in bits 16-23). Else `0`.
    pub load_addr: u32,
    /// `ml-*`: run address. Else `0`.
    pub run_addr: u32,
    /// `ml-*`: `1` if `run_addr` is a real auto-start, else `0`.
    pub autorun: i32,
    /// CE-158 header filename, or the name in SDAR / SDAV text; UTF-8, NUL-terminated,
    /// empty if none.
    pub name: [c_char; SDE_FILE_NAME_SIZE],
}

/// Classify `in` as a one-word token: `basic-ascii`, `basic-pc1500`, `basic-pc1600`,
/// `ml-lh5801`, `ml-z80`, `raw-lh5801`, `raw-z80`, `raw`, `reserve`, `reserve-text`,
/// `variables`, `variables-text`, `text`, `empty`, `wav-pc1500`, `wav-pc1600`, `wav` —
/// or `damaged` if the file has a fatal problem (`SDE_PROBLEM_FATAL`). `raw-*` are
/// heuristic CPU guesses. For a cassette WAV (`wav-*`) the other `SdeFileInfo` fields
/// describe the first file on the tape; `sde_wav_decode` gets its serial image.
/// `*out_kind` receives a static NUL-terminated string: do not free it.
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `out_kind` is writable.
#[no_mangle]
pub unsafe extern "C" fn sde_file_kind(
    input: *const u8,
    in_len: usize,
    out_kind: *mut *const c_char,
) -> i32 {
    guard(|| {
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        if out_kind.is_null() {
            return SDE_ERR_ARGS;
        }
        let s = info::classify(data);
        *out_kind = if s.problems & problem::FATAL != 0 {
            c"damaged".as_ptr()
        } else {
            s.kind.as_cstr().as_ptr()
        };
        SDE_OK
    })
}

/// Classify `in` and fill `*out` with what a loader needs: the kind token, problem
/// flags, where the payload is, the load/run address, and the name. Nothing to free.
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `out` is a writable `SdeFileInfo`.
#[no_mangle]
pub unsafe extern "C" fn sde_file_info(input: *const u8, in_len: usize, out: *mut SdeFileInfo) -> i32 {
    guard(|| {
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        if out.is_null() {
            return SDE_ERR_ARGS;
        }
        let s = info::classify(data);
        let name = c_name(s.name.as_deref().unwrap_or(""));
        *out = SdeFileInfo {
            kind: s.kind.as_cstr().as_ptr(),
            problems: s.problems,
            payload_offset: s.payload_offset,
            payload_len: s.payload_len,
            load_addr: s.load_addr,
            run_addr: s.run_addr,
            autorun: s.autorun as i32,
            name,
        };
        SDE_OK
    })
}

/// `n` as a NUL-terminated UTF-8 name field, truncated on a character boundary.
pub(super) fn c_name<const N: usize>(n: &str) -> [c_char; N] {
    let mut name = [0 as c_char; N];
    let mut end = n.len().min(N - 1);
    while !n.is_char_boundary(end) {
        end -= 1;
    }
    for (dst, &b) in name.iter_mut().zip(&n.as_bytes()[..end]) {
        *dst = b as c_char;
    }
    name
}
