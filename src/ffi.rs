//! C ABI over the pure core, for embedding in C / C++ / Swift.
//!
//! Contract:
//! * every entry point returns `int32_t`: `0` = OK, negative = error.
//! * on error, `sde_last_error()` returns a thread-local, NUL-terminated message,
//!   valid until the next `sde_*` call on the same thread.
//! * `*out` / `*out_len` receive a Rust-allocated buffer the caller must release with
//!   `sde_buf_free`.
//! * panics never cross the boundary (`catch_unwind`).

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::detect::Content;
use crate::detokenize::LineEnding;
use crate::registry::Device;

pub const SDE_OK: i32 = 0;
pub const SDE_ERR: i32 = -1;
pub const SDE_ERR_PANIC: i32 = -2;
pub const SDE_ERR_ARGS: i32 = -3;

/// Target machine family.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum SdeDevice {
    Pc1500 = 0,
    Pc1600 = 1,
}

/// Detected input content kind.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum SdeContent {
    Unknown = 0,
    AsciiBasic = 1,
    Ce158Basic = 2,
    Pc1600Basic = 3,
}

/// Line ending for a de-tokenized listing (`sde_detokenize` / `sde_convert`).
/// Ignored when tokenizing — `CR` and `CRLF` input are always accepted.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum SdeLineEnding {
    /// Host default: `\r\n` on Windows, `\n` elsewhere.
    Platform = 0,
    /// `\n` (LF).
    Lf = 1,
    /// `\r\n` (CRLF).
    CrLf = 2,
    /// `\r` (CR) — the PC-1500's own line terminator.
    Cr = 3,
}

impl From<SdeLineEnding> for LineEnding {
    fn from(e: SdeLineEnding) -> Self {
        match e {
            SdeLineEnding::Platform => LineEnding::Platform,
            SdeLineEnding::Lf => LineEnding::Lf,
            SdeLineEnding::CrLf => LineEnding::CrLf,
            SdeLineEnding::Cr => LineEnding::Cr,
        }
    }
}

impl From<SdeDevice> for Device {
    fn from(d: SdeDevice) -> Self {
        match d {
            SdeDevice::Pc1500 => Device::Pc1500,
            SdeDevice::Pc1600 => Device::Pc1600,
        }
    }
}

impl From<Content> for SdeContent {
    fn from(c: Content) -> Self {
        match c {
            Content::AsciiBasic => SdeContent::AsciiBasic,
            Content::Ce158Basic => SdeContent::Ce158Basic,
            Content::Pc1600Basic => SdeContent::Pc1600Basic,
            // Machine language is outside the C ABI's scope (tokenize/de-tokenize
            // BASIC only); matches this enum's pre-existing behavior, which already
            // reported a machine-language header as Unknown.
            Content::Ce158Machine | Content::Pc1600Machine | Content::Unknown => SdeContent::Unknown,
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").unwrap());
}

fn set_error(msg: &str) {
    let c = CString::new(msg.replace('\0', " ")).unwrap_or_else(|_| CString::new("error").unwrap());
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

fn clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new("").unwrap());
}

/// Crate version, static NUL-terminated string.
#[no_mangle]
pub extern "C" fn sde_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Last error message on this thread. Never NULL; empty string if no error.
#[no_mangle]
pub extern "C" fn sde_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// Release a buffer returned via an `out` / `out_len` pair.
///
/// # Safety
/// `ptr`/`len` must be exactly what a prior `sde_*` call wrote, and must be freed once.
#[no_mangle]
pub unsafe extern "C" fn sde_buf_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

unsafe fn slice<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    if p.is_null() {
        (len == 0).then(|| &[][..])
    } else {
        Some(std::slice::from_raw_parts(p, len))
    }
}

unsafe fn opt_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

unsafe fn write_buf(bytes: Vec<u8>, out: *mut *mut u8, out_len: *mut usize) -> Result<(), i32> {
    if out.is_null() || out_len.is_null() {
        return Err(SDE_ERR_ARGS);
    }
    let mut boxed = bytes.into_boxed_slice();
    let ptr = boxed.as_mut_ptr();
    let len = boxed.len();
    std::mem::forget(boxed);
    *out = ptr;
    *out_len = len;
    Ok(())
}

fn guard(f: impl FnOnce() -> i32) -> i32 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(code) => code,
        Err(_) => {
            set_error("panic in sharpdx");
            SDE_ERR_PANIC
        }
    }
}

/// Detect the content kind of `in`. Writes `*out_kind`.
///
/// # Safety
/// `in`/`in_len` describe a readable buffer; `out_kind` is a writable `SdeContent`.
#[no_mangle]
pub unsafe extern "C" fn sde_detect(
    input: *const u8,
    in_len: usize,
    out_kind: *mut SdeContent,
) -> i32 {
    guard(|| {
        clear_error();
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        if out_kind.is_null() {
            return SDE_ERR_ARGS;
        }
        *out_kind = crate::detect::detect(data).into();
        SDE_OK
    })
}

/// ASCII BASIC bytes -> tokenized payload. `with_header != 0` prepends the serial header.
///
/// # Safety
/// Pointer/length pairs must describe readable buffers; `name` is NULL or a C string;
/// `out`/`out_len` are writable.
#[no_mangle]
pub unsafe extern "C" fn sde_tokenize(
    device: SdeDevice,
    with_header: c_int,
    name: *const c_char,
    input: *const u8,
    in_len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guard(|| {
        clear_error();
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        let nm = opt_str(name);
        match crate::convert::convert(data, device.into(), nm, with_header != 0) {
            Ok(o) if matches!(o.content, Content::AsciiBasic) => match write_buf(o.bytes, out, out_len) {
                Ok(()) => SDE_OK,
                Err(c) => c,
            },
            Ok(_) => {
                set_error("input is not ASCII BASIC");
                SDE_ERR
            }
            Err(e) => {
                set_error(&e.to_string());
                SDE_ERR
            }
        }
    })
}

/// Tokenized bytes -> ASCII BASIC (UTF-8). Accepts a CE-158 / PC-1600 header (device
/// then taken from it) or a bare payload (uses `device`). `line_ending` sets the
/// listing's line terminator (`SDE_LINE_ENDING_PLATFORM` = the host default).
///
/// # Safety
/// As `sde_tokenize`.
#[no_mangle]
pub unsafe extern "C" fn sde_detokenize(
    device: SdeDevice,
    input: *const u8,
    in_len: usize,
    line_ending: SdeLineEnding,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guard(|| {
        clear_error();
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        let eol: LineEnding = line_ending.into();

        let result = match crate::detect::detect(data) {
            Content::Ce158Basic | Content::Pc1600Basic => {
                crate::convert::convert_with(data, device.into(), None, true, eol).map(|o| o.bytes)
            }
            _ => {
                // Treat as a headerless payload with the caller's device.
                let reg = crate::registry::Registry::for_device(device.into());
                crate::detokenize::detokenize_to_text(data, reg, eol).map(String::into_bytes)
            }
        };
        match result {
            Ok(bytes) => match write_buf(bytes, out, out_len) {
                Ok(()) => SDE_OK,
                Err(c) => c,
            },
            Err(e) => {
                set_error(&e.to_string());
                SDE_ERR
            }
        }
    })
}

/// Content-driven convert (mirrors the CLI): picks direction from `in`. Writes
/// `*out_kind` with the detected input kind when non-NULL. `line_ending` sets the
/// line terminator of a de-tokenized listing (`SDE_LINE_ENDING_PLATFORM` = the
/// host default); it is ignored when the input is ASCII BASIC.
///
/// # Safety
/// As `sde_tokenize`; `out_kind` is NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn sde_convert(
    device: SdeDevice,
    name: *const c_char,
    input: *const u8,
    in_len: usize,
    line_ending: SdeLineEnding,
    out: *mut *mut u8,
    out_len: *mut usize,
    out_kind: *mut SdeContent,
) -> i32 {
    guard(|| {
        clear_error();
        let Some(data) = slice(input, in_len) else { return SDE_ERR_ARGS };
        let nm = opt_str(name);
        match crate::convert::convert_with(data, device.into(), nm, true, line_ending.into()) {
            Ok(o) => {
                if !out_kind.is_null() {
                    *out_kind = o.content.into();
                }
                match write_buf(o.bytes, out, out_len) {
                    Ok(()) => SDE_OK,
                    Err(c) => c,
                }
            }
            Err(e) => {
                set_error(&e.to_string());
                SDE_ERR
            }
        }
    })
}
