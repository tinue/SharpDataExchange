//! Exercises the `extern "C"` surface from Rust, plus the generated-file drift guards.

use std::ffi::{CStr, CString};
use std::path::Path;
use std::process::Command;
use std::ptr;

use sharpdx::ffi::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn take_buf(ptr: *mut u8, len: usize) -> Vec<u8> {
    assert!(!ptr.is_null());
    let v = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    unsafe { sde_buf_free(ptr, len) };
    v
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(sde_last_error()) }.to_string_lossy().into_owned()
}

#[test]
fn tokenize_matches_ce158_payload() {
    let src = fixture("depreciation.bas");
    let name = CString::new("depreciation").unwrap();
    let mut out = ptr::null_mut();
    let mut out_len = 0usize;
    let rc = unsafe {
        sde_tokenize(
            SdeDevice::Pc1500,
            1,
            SdeSegmentMarker::Wire,
            name.as_ptr(),
            src.as_ptr(),
            src.len(),
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    let got = take_buf(out, out_len);
    let want = fixture("depreciation-tokenized-ce158header.bin");
    assert_eq!(&got[27..], &want[27..], "payload");
    assert_eq!(&got[..5], &want[..5]);
}

#[test]
fn tokenize_segment_marker_wire_vs_memory() {
    let src = b"5 \"A\"\n10 END\n#SEGMENT\n5 \"B\"\n10 END\n";
    let name = CString::new("split").unwrap();

    let mut wire = ptr::null_mut();
    let mut wire_len = 0usize;
    let rc = unsafe {
        sde_tokenize(
            SdeDevice::Pc1600,
            0,
            SdeSegmentMarker::Wire,
            name.as_ptr(),
            src.as_ptr(),
            src.len(),
            &mut wire,
            &mut wire_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    let wire_bytes = take_buf(wire, wire_len);

    let mut mem = ptr::null_mut();
    let mut mem_len = 0usize;
    let rc = unsafe {
        sde_tokenize(
            SdeDevice::Pc1600,
            0,
            SdeSegmentMarker::Memory,
            name.as_ptr(),
            src.as_ptr(),
            src.len(),
            &mut mem,
            &mut mem_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    let mem_bytes = take_buf(mem, mem_len);

    assert_eq!(wire_bytes.len(), mem_bytes.len() + 2, "Memory drops the 2 pacing bytes");
    assert!(wire_bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
    assert!(!mem_bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
    assert!(mem_bytes.contains(&0xFF));
}

#[test]
fn detokenize_roundtrips_pc1600_fixture() {
    let data = fixture("depreciation-tokenized-pc1600header.bin");
    let mut out = ptr::null_mut();
    let mut out_len = 0usize;
    let rc = unsafe {
        sde_detokenize(
            SdeDevice::Pc1600,
            data.as_ptr(),
            data.len(),
            SdeLineEnding::Platform,
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    let listing = take_buf(out, out_len);

    // Feed it back through sde_convert (content-driven) and expect the same payload.
    let name = CString::new("depreciation").unwrap();
    let mut b = ptr::null_mut();
    let mut b_len = 0usize;
    let mut kind = SdeContent::Unknown;
    let rc = unsafe {
        sde_convert(
            SdeDevice::Pc1600,
            name.as_ptr(),
            listing.as_ptr(),
            listing.len(),
            SdeLineEnding::Platform,
            &mut b,
            &mut b_len,
            &mut kind,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    assert!(matches!(kind, SdeContent::AsciiBasic));
    let retok = take_buf(b, b_len);
    assert_eq!(&retok[16..], &data[16..]);
}

#[test]
fn detect_reports_kinds() {
    let mut kind = SdeContent::Unknown;
    let ascii = b"10 PRINT 1\n20 END\n";
    assert_eq!(
        unsafe { sde_detect(ascii.as_ptr(), ascii.len(), &mut kind) },
        SDE_OK
    );
    assert!(matches!(kind, SdeContent::AsciiBasic));

    let ce = fixture("depreciation-tokenized-ce158header.bin");
    assert_eq!(unsafe { sde_detect(ce.as_ptr(), ce.len(), &mut kind) }, SDE_OK);
    assert!(matches!(kind, SdeContent::Ce158Basic));
}

#[test]
fn headerless_blob_via_convert_is_an_error_with_message() {
    let blob = [0x00u8, 0x0A, 0x07, 0x22, 0x41, 0x22, 0xF1, 0xB3, 0x30, 0x0D];
    let mut out = ptr::null_mut();
    let mut out_len = 0usize;
    let mut kind = SdeContent::Unknown;
    let rc = unsafe {
        sde_convert(
            SdeDevice::Pc1500,
            ptr::null(),
            blob.as_ptr(),
            blob.len(),
            SdeLineEnding::Platform,
            &mut out,
            &mut out_len,
            &mut kind,
        )
    };
    assert_eq!(rc, SDE_ERR);
    assert!(!last_error().is_empty());
    // ...but sde_detokenize accepts it as a bare payload.
    let rc = unsafe {
        sde_detokenize(
            SdeDevice::Pc1500,
            blob.as_ptr(),
            blob.len(),
            SdeLineEnding::Lf,
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    let listing = take_buf(out, out_len);
    assert_eq!(listing, b"10 \"A\"WAIT 0\n");

    // Same payload, explicit CRLF override.
    let rc = unsafe {
        sde_detokenize(
            SdeDevice::Pc1500,
            blob.as_ptr(),
            blob.len(),
            SdeLineEnding::CrLf,
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_OK, "err: {}", last_error());
    assert_eq!(take_buf(out, out_len), b"10 \"A\"WAIT 0\r\n");
}

#[test]
fn malformed_payload_returns_error_not_panic() {
    let bad = [0x00u8, 0x0A, 0x40, 0x0D]; // length 0x40 but nothing follows
    let mut out = ptr::null_mut();
    let mut out_len = 0usize;
    let rc = unsafe {
        sde_detokenize(
            SdeDevice::Pc1500,
            bad.as_ptr(),
            bad.len(),
            SdeLineEnding::Platform,
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_ERR);
    assert!(last_error().contains("malformed") || last_error().contains("length"));
}

#[test]
fn null_args_rejected() {
    let mut out = ptr::null_mut();
    let mut out_len = 0usize;
    let rc = unsafe {
        sde_tokenize(
            SdeDevice::Pc1500,
            0,
            SdeSegmentMarker::Wire,
            ptr::null(),
            ptr::null(),
            5,
            &mut out,
            &mut out_len,
        )
    };
    assert_eq!(rc, SDE_ERR_ARGS);
}

// ---- generated-file drift guards -------------------------------------------------

#[test]
fn header_is_current() {
    // build.rs regenerates include/sharpdx.h on every build; just assert it is non-empty
    // and mentions the entry points (a stale header would still list the old signatures,
    // but at minimum this catches a missing regeneration).
    let h = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("include/sharpdx.h"),
    )
    .unwrap();
    for sym in [
        "sde_tokenize",
        "sde_detokenize",
        "sde_convert",
        "sde_detect",
        "SdeDevice",
        "SdeLineEnding",
        "SdeSegmentMarker",
    ] {
        assert!(h.contains(sym), "generated header missing {sym}");
    }
}

#[test]
fn keywords_table_is_current() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shared = manifest.parent().unwrap().join("SharpBasicShared");
    if !shared.join("sharp-basic-core").is_dir() {
        eprintln!("skipping: SharpBasicShared not present");
        return;
    }
    let out = Command::new("python3")
        .arg(manifest.join("tools/extract_keywords.py"))
        .arg("--check")
        .current_dir(manifest)
        .output()
        .expect("run extract_keywords.py");
    assert!(
        out.status.success(),
        "src/keywords.rs is stale:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
