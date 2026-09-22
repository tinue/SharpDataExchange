//! C ABI for files on one CE-1600F floppy side ([`crate::diskfs`] + [`crate::transfer`]).
//!
//! Every call works on a caller-owned side buffer of exactly [`SDE_DISK_SIDE_SIZE`] bytes
//! (the caller owns the container, e.g. Calc-U-1600's in-memory disk image). Mutating
//! calls work on a copy and write it back only on success, so an error never leaves a
//! half-changed side.

use std::ffi::{c_char, CStr};

use super::{clear_error, finish_bytes, guard, opt_str, set_error, slice, SdeLineEnding, SDE_ERR, SDE_ERR_ARGS, SDE_OK};
use crate::diskfs::{DiskError, DosTimestamp, FileName, Pattern, Volume};
use crate::registry::Device;
use crate::transfer::{self, DiskFileKind, Endpoint, Format, GetSpec, PutSpec};

pub const SDE_ERR_NOT_FOUND: i32 = -4;
pub const SDE_ERR_EXISTS: i32 = -5;
pub const SDE_ERR_DISK_FULL: i32 = -6;
pub const SDE_ERR_NOT_FORMATTED: i32 = -7;
pub const SDE_ERR_PROTECTED: i32 = -8;
pub const SDE_ERR_BAD_NAME: i32 = -9;
pub const SDE_ERR_CORRUPT: i32 = -10;
pub const SDE_ERR_DIRECTORY_FULL: i32 = -11;

/// Bytes in one floppy side (16 tracks × 8 sectors × 512 bytes).
pub const SDE_DISK_SIDE_SIZE: usize = 65536;
/// Directory entries per side.
pub const SDE_DISK_MAX_ENTRIES: usize = 48;
/// `flags`: replace an existing file / delete write-protected files.
pub const SDE_DISK_FORCE: u32 = 1;
/// `run_addr` for `sde_disk_put` in machine mode: no auto-start.
pub const SDE_DISK_NO_RUN: u32 = 0xFFFF_FFFF;

/// What a stored file holds (from its content; the directory does not record it).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdeDiskKind {
    Unknown = 0,
    /// Tokenized BASIC behind a PC-1600 header.
    Basic = 1,
    /// Machine language behind a PC-1600 header.
    Machine = 2,
    /// A BASIC program saved as ASCII.
    AsciiBasic = 3,
    /// Any other ASCII file.
    Text = 4,
}

/// How `sde_disk_get` returns a file.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum SdeDiskGetMode {
    /// The natural host form: BASIC de-tokenized to a UTF-8 listing, ASCII files as
    /// UTF-8 text, machine code and unknown data unchanged.
    Auto = 0,
    /// Unchanged, header included (BASIC stays tokenized).
    Binary = 1,
    /// Like `Binary`, without the 16-byte header.
    Payload = 2,
    /// The stored bytes, no interpretation at all.
    Raw = 3,
}

/// How `sde_disk_put` turns the input into a stored file.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum SdeDiskPutMode {
    /// By content: a BASIC listing is tokenized, text becomes a PC-1600 ASCII file
    /// (CP437, CRLF, `1A`), a file with a PC-1600 header is stored unchanged; anything
    /// else is an error.
    Auto = 0,
    /// A BASIC listing stored as an ASCII program file (`SAVE "…",A`).
    AsciiListing = 1,
    /// Headerless machine code, wrapped in a header with `start_addr`/`run_addr`.
    Machine = 2,
    /// Stored exactly as given.
    Raw = 3,
    /// Tokenized as a BASIC listing even if detection took it for plain text (the
    /// override for a misdetect); fails if it does not tokenize.
    Tokenize = 4,
}

/// One directory entry.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SdeDirEntry {
    /// `NAME.EXT`, NUL-terminated.
    pub name: [c_char; 13],
    /// Attribute byte (bit 0 write-protected, bit 2 hidden).
    pub attr: u8,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Size in bytes as stored (a header included).
    pub size: u32,
    pub kind: SdeDiskKind,
    /// Machine code only: 24-bit load / run address (bank in bits 16-23).
    pub load_addr: u32,
    pub run_addr: u32,
}

/// A directory timestamp. The PC-1600 clock has no year.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SdeDiskTime {
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn disk_error_code(e: &anyhow::Error) -> i32 {
    match e.downcast_ref::<DiskError>() {
        Some(DiskError::NotFound(_)) => SDE_ERR_NOT_FOUND,
        Some(DiskError::Exists(_)) => SDE_ERR_EXISTS,
        Some(DiskError::DiskFull { .. }) => SDE_ERR_DISK_FULL,
        Some(DiskError::NotFormatted) => SDE_ERR_NOT_FORMATTED,
        Some(DiskError::Protected(_)) => SDE_ERR_PROTECTED,
        Some(DiskError::BadName(_)) => SDE_ERR_BAD_NAME,
        Some(DiskError::Corrupt(_)) => SDE_ERR_CORRUPT,
        Some(DiskError::DirectoryFull) => SDE_ERR_DIRECTORY_FULL,
        None => SDE_ERR,
    }
}

fn fail(e: anyhow::Error) -> i32 {
    set_error(&format!("{e:#}"));
    disk_error_code(&e)
}

unsafe fn side<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    if p.is_null() || len != SDE_DISK_SIDE_SIZE {
        return None;
    }
    Some(std::slice::from_raw_parts(p, len))
}

unsafe fn required_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

fn kind_of(k: DiskFileKind) -> (SdeDiskKind, u32, u32) {
    match k {
        DiskFileKind::Basic { .. } => (SdeDiskKind::Basic, 0, 0),
        DiskFileKind::Machine { load, run, .. } => (SdeDiskKind::Machine, load, run),
        DiskFileKind::AsciiBasic => (SdeDiskKind::AsciiBasic, 0, 0),
        DiskFileKind::Text => (SdeDiskKind::Text, 0, 0),
        DiskFileKind::Unknown => (SdeDiskKind::Unknown, 0, 0),
    }
}

/// List the files on a side into `entries` (up to `capacity`; `SDE_DISK_MAX_ENTRIES`
/// always suffices). `*out_count` receives the number of files, `*out_free_bytes` (if
/// non-NULL) the free space. An unformatted side is `SDE_ERR_NOT_FORMATTED`.
///
/// # Safety
/// `side` points to `side_len` readable bytes; `entries` to `capacity` writable
/// entries (or NULL with `capacity == 0`); `out_count` is writable.
#[no_mangle]
pub unsafe extern "C" fn sde_disk_list(
    side_buf: *const u8,
    side_len: usize,
    entries: *mut SdeDirEntry,
    capacity: usize,
    out_count: *mut usize,
    out_free_bytes: *mut u32,
) -> i32 {
    guard(|| {
        clear_error();
        let Some(bytes) = side(side_buf, side_len) else { return SDE_ERR_ARGS };
        if out_count.is_null() || (entries.is_null() && capacity > 0) {
            return SDE_ERR_ARGS;
        }
        let vol = match Volume::open_floppy_side(bytes) {
            Ok(v) => v,
            Err(e) => return fail(e.into()),
        };
        let list = vol.list();
        for (i, e) in list.iter().take(capacity).enumerate() {
            let (kind, load, run) = match vol.read(e) {
                Ok(data) => kind_of(transfer::classify_disk_file(&data)),
                Err(_) => (SdeDiskKind::Unknown, 0, 0),
            };
            let ts = e.timestamp();
            let mut name = [0 as c_char; 13];
            for (dst, &b) in name.iter_mut().zip(e.name.to_string().as_bytes().iter().take(12)) {
                *dst = b as c_char;
            }
            *entries.add(i) = SdeDirEntry {
                name,
                attr: e.attr,
                month: ts.month,
                day: ts.day,
                hour: ts.hour,
                minute: ts.minute,
                second: ts.second,
                size: e.size,
                kind,
                load_addr: load,
                run_addr: run,
            };
        }
        *out_count = list.len();
        if !out_free_bytes.is_null() {
            *out_free_bytes = vol.free_bytes() as u32;
        }
        SDE_OK
    })
}

/// Read file `name` (`NAME.EXT`, case-insensitive) from a side, converted per `mode`.
/// `line_ending` applies to listings and text in `Auto` mode. `*out`/`*out_len` receive
/// a buffer to release with `sde_buf_free`; `*out_kind` (if non-NULL) the file's kind.
///
/// # Safety
/// As `sde_disk_list`; `name` is a C string; `out`/`out_len` are writable.
#[no_mangle]
pub unsafe extern "C" fn sde_disk_get(
    side_buf: *const u8,
    side_len: usize,
    name: *const c_char,
    mode: SdeDiskGetMode,
    line_ending: SdeLineEnding,
    out: *mut *mut u8,
    out_len: *mut usize,
    out_kind: *mut SdeDiskKind,
) -> i32 {
    guard(|| {
        clear_error();
        let (Some(bytes), Some(name)) = (side(side_buf, side_len), required_str(name)) else {
            return SDE_ERR_ARGS;
        };
        let result = (|| -> anyhow::Result<Vec<u8>> {
            let vol = Volume::open_floppy_side(bytes)?;
            let n = FileName::parse(name)?;
            let e = vol.find(&n).ok_or_else(|| DiskError::NotFound(n.to_string()))?;
            let stored = vol.read(&e)?;
            if !out_kind.is_null() {
                *out_kind = kind_of(transfer::classify_disk_file(&stored)).0;
            }
            let spec = |format, skip_header| GetSpec { format, skip_header, eol: line_ending.into() };
            Ok(match mode {
                SdeDiskGetMode::Raw => stored,
                SdeDiskGetMode::Auto => transfer::extract(&stored, &spec(None, false))?.bytes,
                SdeDiskGetMode::Binary => transfer::extract(&stored, &spec(Some(Format::Binary), false))?.bytes,
                SdeDiskGetMode::Payload => transfer::extract(&stored, &spec(Some(Format::Binary), true))?.bytes,
            })
        })();
        match result {
            Ok(data) => finish_bytes(Ok(data), out, out_len),
            Err(e) => fail(e),
        }
    })
}

/// Store `input` as `name` on a side, converted per `mode` (see [`SdeDiskPutMode`]).
/// `start_addr`/`run_addr` are used in `Machine` mode only (24-bit, bank in bits 16-23;
/// `run_addr == SDE_DISK_NO_RUN` = no auto-start). `flags` may contain `SDE_DISK_FORCE`
/// to replace an existing file. `when` is the directory timestamp, NULL = now (UTC).
///
/// # Safety
/// `side_buf` points to `side_len` writable bytes; `name` is a C string; `input`/`in_len`
/// describe a readable buffer; `when` is NULL or readable.
#[no_mangle]
pub unsafe extern "C" fn sde_disk_put(
    side_buf: *mut u8,
    side_len: usize,
    name: *const c_char,
    input: *const u8,
    in_len: usize,
    mode: SdeDiskPutMode,
    start_addr: u32,
    run_addr: u32,
    flags: u32,
    when: *const SdeDiskTime,
) -> i32 {
    guard(|| {
        clear_error();
        let (Some(bytes), Some(name), Some(data)) =
            (side(side_buf, side_len), required_str(name), slice(input, in_len))
        else {
            return SDE_ERR_ARGS;
        };
        let ts = if when.is_null() {
            DosTimestamp::now_utc()
        } else {
            let w = &*when;
            DosTimestamp::new(w.month, w.day, w.hour, w.minute, w.second)
        };
        let mut copy = bytes.to_vec();
        let result = (|| -> anyhow::Result<()> {
            let mut vol = Volume::open_floppy_side(&mut copy[..])?;
            let n = FileName::parse(name)?;
            let h = crate::header::find(data);
            let content = crate::detect::detect_from_header(h.as_ref(), data);
            let machine = matches!(mode, SdeDiskPutMode::Machine);
            let spec = PutSpec {
                source_name: name,
                device: Device::Pc1600,
                format: match mode {
                    SdeDiskPutMode::AsciiListing => Some(Format::Ascii),
                    SdeDiskPutMode::Tokenize => Some(Format::Binary),
                    _ => None,
                },
                start_address: machine.then_some(start_addr),
                run_address: (machine && run_addr != SDE_DISK_NO_RUN).then_some(run_addr),
                raw: matches!(mode, SdeDiskPutMode::Raw),
                endpoint: Endpoint::Disk,
            };
            if matches!(mode, SdeDiskPutMode::AsciiListing) && content != crate::detect::Content::AsciiBasic {
                anyhow::bail!("input is not a BASIC listing");
            }
            let out = transfer::build_put(data, if machine { None } else { h.as_ref() }, content, &spec)?;
            vol.write(&n, &out.bytes, ts, flags & SDE_DISK_FORCE != 0)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                std::slice::from_raw_parts_mut(side_buf, side_len).copy_from_slice(&copy);
                SDE_OK
            }
            Err(e) => fail(e),
        }
    })
}

/// Delete `name_or_pattern` (`NAME.EXT`, or with `*`/`?` wildcards) from a side.
/// `*out_deleted` (if non-NULL) receives the number of files deleted; no match is
/// `SDE_ERR_NOT_FOUND`. `flags` may contain `SDE_DISK_FORCE` to delete write-protected
/// files.
///
/// # Safety
/// As `sde_disk_put`; `out_deleted` is NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn sde_disk_delete(
    side_buf: *mut u8,
    side_len: usize,
    name_or_pattern: *const c_char,
    flags: u32,
    out_deleted: *mut usize,
) -> i32 {
    guard(|| {
        clear_error();
        let (Some(bytes), Some(pat)) = (side(side_buf, side_len), opt_str(name_or_pattern)) else {
            return SDE_ERR_ARGS;
        };
        let mut copy = bytes.to_vec();
        let result = (|| -> anyhow::Result<usize> {
            let mut vol = Volume::open_floppy_side(&mut copy[..])?;
            let entries = if Pattern::is_wildcard(pat) {
                vol.glob(&Pattern::parse(pat)?)
            } else {
                vol.find(&FileName::parse(pat)?).into_iter().collect()
            };
            if entries.is_empty() {
                return Err(DiskError::NotFound(pat.to_string()).into());
            }
            for e in &entries {
                vol.delete(e, flags & SDE_DISK_FORCE != 0)?;
            }
            Ok(entries.len())
        })();
        match result {
            Ok(n) => {
                std::slice::from_raw_parts_mut(side_buf, side_len).copy_from_slice(&copy);
                if !out_deleted.is_null() {
                    *out_deleted = n;
                }
                SDE_OK
            }
            Err(e) => fail(e),
        }
    })
}
