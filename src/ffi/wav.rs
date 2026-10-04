//! C ABI for cassette WAV files ([`crate::wav`]).
//!
//! A tape file crosses the boundary as its *serial image* (CE-158 or PC-1600 header +
//! payload), the same bytes `sde_file_info`, `sde_detokenize` and a loader already
//! handle.

use std::ffi::c_char;

use super::info::{c_name, SDE_FILE_NAME_SIZE};
use super::{finish_bytes, guard, opt_str, set_error, slice, SDE_ERR_ARGS, SDE_OK};
use crate::wav::{self, EncodeOptions, Leader, TapeError, TapeFormat, TapeKind};

/// Not a RIFF/WAVE file (or a WAV encoding that can't be read).
pub const SDE_ERR_WAV_FORMAT: i32 = -12;
/// A WAV with no PC-1500 / PC-1600 tape signal on it.
pub const SDE_ERR_WAV_NO_SIGNAL: i32 = -13;
/// A tape file was found but failed its checksums: it could not be decoded safely.
pub const SDE_ERR_WAV_CORRUPT: i32 = -14;
/// Content this library can't put on (or take off) a tape, e.g. a PC-1500 data file.
pub const SDE_ERR_WAV_UNSUPPORTED: i32 = -15;

/// `leader_ms` for `sde_wav_encode`: the default short leader (about 2 s on the
/// PC-1500, 3 s on the PC-1600).
pub const SDE_WAV_LEADER_DEFAULT: u32 = 0;
/// `leader_ms` for `sde_wav_encode`: the ROMs' own, long leader.
pub const SDE_WAV_LEADER_ROM: u32 = 0xFFFF_FFFF;

/// Which machine / interface a tape is for.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdeTapeFormat {
    /// PC-1500 / PC-1500A with CE-150.
    Pc1500Ce150 = 0,
    /// PC-1600 with CE-1600P, MODE 0.
    Pc1600Ce1600p = 1,
}

/// What a tape file holds.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdeTapeKind {
    Basic = 0,
    Machine = 1,
    Reserve = 2,
    DefKeys = 3,
    Data = 4,
    Ascii = 5,
}

/// One file decoded from a tape.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SdeWavFile {
    pub format: SdeTapeFormat,
    pub kind: SdeTapeKind,
    /// Name on the tape, UTF-8, NUL-terminated.
    pub name: [c_char; SDE_FILE_NAME_SIZE],
    /// Machine code: load / entry address as on the tape (PC-1600: bank in bits
    /// 16-23). Else `0`.
    pub load_addr: u32,
    pub run_addr: u32,
    /// Machine code: `1` if `run_addr` is a real auto-start.
    pub autorun: i32,
    /// Payload bytes (without header).
    pub payload_len: usize,
    /// Where the file starts on the tape, milliseconds.
    pub start_ms: u32,
    /// Measured tape speed, per mille of nominal (`1000` = exact, `1050` = 5 % fast).
    pub speed_permille: u32,
}

fn code(e: &TapeError) -> i32 {
    match e {
        TapeError::NotWav(_) => SDE_ERR_WAV_FORMAT,
        TapeError::NoSignal => SDE_ERR_WAV_NO_SIGNAL,
        TapeError::Corrupt(_) => SDE_ERR_WAV_CORRUPT,
        TapeError::Unsupported(_) => SDE_ERR_WAV_UNSUPPORTED,
    }
}

fn fail(e: TapeError) -> i32 {
    set_error(&e.to_string());
    code(&e)
}

/// Count the files on the tape in `in` that decode safely. `*out_issues` (may be NULL)
/// receives the number of files found but *not* decodable (checksum errors,
/// unsupported types). Returns `SDE_ERR_WAV_NO_SIGNAL` when nothing at all is on it.
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `out_count` is writable; `out_issues` is
/// NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn sde_wav_count(
    input: *const u8,
    in_len: usize,
    out_count: *mut usize,
    out_issues: *mut usize,
) -> i32 {
    guard(|| {
        let Some(data) = slice(input, in_len) else {
            return SDE_ERR_ARGS;
        };
        if out_count.is_null() {
            return SDE_ERR_ARGS;
        }
        let r = match wav::decode(data) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        *out_count = r.files.len();
        if !out_issues.is_null() {
            *out_issues = r.issues.len();
        }
        if r.files.is_empty() && r.issues.is_empty() {
            return fail(TapeError::NoSignal);
        }
        SDE_OK
    })
}

/// Decode file `index` (0-based, in tape order) of the tape in `in` to its serial image
/// (CE-158 / PC-1600 header + payload) in `*out` / `*out_len` (free with
/// `sde_buf_free`). `*out_file` (may be NULL) receives its description. With no file at
/// `index`, the error explains why (e.g. the checksum error of a damaged file).
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `out`/`out_len` are writable; `out_file`
/// is NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn sde_wav_decode(
    input: *const u8,
    in_len: usize,
    index: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
    out_file: *mut SdeWavFile,
) -> i32 {
    guard(|| {
        let Some(data) = slice(input, in_len) else {
            return SDE_ERR_ARGS;
        };
        if out.is_null() || out_len.is_null() {
            return SDE_ERR_ARGS;
        }
        let r = match wav::decode(data) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        let Some(f) = r.files.get(index) else {
            return fail(match (r.files.len(), r.issues.first()) {
                (0, Some(i)) => TapeError::Corrupt(wav::issue_text(i)),
                (0, None) => TapeError::NoSignal,
                (n, _) => {
                    set_error(&format!("file {index} requested, the tape has {n}"));
                    return SDE_ERR_ARGS;
                }
            });
        };
        let image = match wav::to_image(f) {
            Ok(i) => i,
            Err(e) => return fail(e),
        };
        if !out_file.is_null() {
            *out_file = SdeWavFile {
                format: match f.format {
                    TapeFormat::Pc1500Ce150 => SdeTapeFormat::Pc1500Ce150,
                    TapeFormat::Pc1600Ce1600p => SdeTapeFormat::Pc1600Ce1600p,
                },
                kind: match f.kind {
                    TapeKind::Basic => SdeTapeKind::Basic,
                    TapeKind::Machine => SdeTapeKind::Machine,
                    TapeKind::Reserve => SdeTapeKind::Reserve,
                    TapeKind::DefKeys => SdeTapeKind::DefKeys,
                    TapeKind::Data => SdeTapeKind::Data,
                    TapeKind::Ascii => SdeTapeKind::Ascii,
                },
                name: c_name(&f.name),
                load_addr: if f.kind == TapeKind::Machine { f.load } else { 0 },
                run_addr: if f.kind == TapeKind::Machine { f.entry } else { 0 },
                autorun: f.autorun() as i32,
                payload_len: f.payload.len(),
                start_ms: (f.start_time * 1000.0) as u32,
                speed_permille: (f.speed * 1000.0).round() as u32,
            };
        }
        finish_bytes(Ok(image), out, out_len)
    })
}

/// Encode a serial image (CE-158 or PC-1600 header + payload: BASIC, machine code, or
/// PC-1500 reserve) as a cassette WAV (16-bit mono, `sample_rate` at least 16000) in
/// `*out` / `*out_len` (free with
/// `sde_buf_free`). The tape format follows the header: CE-158 → PC-1500 / CE-150,
/// PC-1600 → CE-1600P. `name` (may be NULL) overrides the file name on the tape;
/// `leader_ms` is the lead-in length (`SDE_WAV_LEADER_DEFAULT`, `SDE_WAV_LEADER_ROM`,
/// or milliseconds).
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `name` is NULL or a NUL-terminated string;
/// `out`/`out_len` are writable.
#[no_mangle]
pub unsafe extern "C" fn sde_wav_encode(
    input: *const u8,
    in_len: usize,
    sample_rate: u32,
    leader_ms: u32,
    name: *const c_char,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guard(|| {
        let Some(data) = slice(input, in_len) else {
            return SDE_ERR_ARGS;
        };
        let leader = match leader_ms {
            SDE_WAV_LEADER_DEFAULT => Leader::Default,
            SDE_WAV_LEADER_ROM => Leader::Rom,
            ms => Leader::Seconds(ms as f32 / 1000.0),
        };
        let f = match wav::from_image(data, opt_str(name)) {
            Ok(f) => f,
            Err(e) => return fail(e),
        };
        match wav::encode(&[f], &EncodeOptions { sample_rate, leader }) {
            Ok(bytes) => finish_bytes(Ok(bytes), out, out_len),
            Err(e) => fail(e),
        }
    })
}
