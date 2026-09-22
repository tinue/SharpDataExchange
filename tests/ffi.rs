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

// ---- disk sides ------------------------------------------------------------------

fn floppy_side(fixture: &str, side: sharpdx::floppy_image::Side) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floppy").join(fixture);
    sharpdx::floppy_image::read_path(&p).unwrap().side(side).to_vec()
}

fn list(side: &[u8]) -> (Vec<(String, SdeDiskKind, u32)>, u32) {
    let mut entries = [SdeDirEntry {
        name: [0; 13],
        attr: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        size: 0,
        kind: SdeDiskKind::Unknown,
        load_addr: 0,
        run_addr: 0,
    }; SDE_DISK_MAX_ENTRIES];
    let (mut count, mut free) = (0usize, 0u32);
    let rc = unsafe { sde_disk_list(side.as_ptr(), side.len(), entries.as_mut_ptr(), entries.len(), &mut count, &mut free) };
    assert_eq!(rc, SDE_OK, "{}", last_error());
    let out = entries[..count]
        .iter()
        .map(|e| {
            let name = unsafe { CStr::from_ptr(e.name.as_ptr()) }.to_string_lossy().into_owned();
            (name, e.kind, e.size)
        })
        .collect();
    (out, free)
}

#[test]
fn disk_list_and_get() {
    use sharpdx::floppy_image::Side;
    let side = floppy_side("dw.floppy.yaml", Side::A);
    let (files, free) = list(&side);
    assert_eq!(
        files,
        vec![("GLOBUS.BAS".to_string(), SdeDiskKind::Basic, 5967), ("BIO.BAS".to_string(), SdeDiskKind::Basic, 2059)]
    );
    assert_eq!(free, 53760);

    let name = CString::new("bio.bas").unwrap();
    let (mut out, mut out_len, mut kind) = (ptr::null_mut(), 0usize, SdeDiskKind::Unknown);
    let rc = unsafe {
        sde_disk_get(side.as_ptr(), side.len(), name.as_ptr(), SdeDiskGetMode::Auto, SdeLineEnding::Lf, &mut out, &mut out_len, &mut kind)
    };
    assert_eq!(rc, SDE_OK, "{}", last_error());
    assert_eq!(kind, SdeDiskKind::Basic);
    let listing = String::from_utf8(take_buf(out, out_len)).unwrap();
    assert!(listing.lines().next().unwrap().chars().next().unwrap().is_ascii_digit(), "{listing}");

    let rc = unsafe {
        sde_disk_get(side.as_ptr(), side.len(), name.as_ptr(), SdeDiskGetMode::Payload, SdeLineEnding::Lf, &mut out, &mut out_len, ptr::null_mut())
    };
    assert_eq!(rc, SDE_OK);
    assert_eq!(take_buf(out, out_len).len(), 2059 - 16);

    let missing = CString::new("NOPE.BAS").unwrap();
    let rc = unsafe {
        sde_disk_get(side.as_ptr(), side.len(), missing.as_ptr(), SdeDiskGetMode::Raw, SdeLineEnding::Lf, &mut out, &mut out_len, ptr::null_mut())
    };
    assert_eq!(rc, SDE_ERR_NOT_FOUND);
    assert!(last_error().contains("NOPE.BAS"));

    let blank = floppy_side("dw.floppy.yaml", Side::B);
    let mut count = 0usize;
    let rc = unsafe { sde_disk_list(blank.as_ptr(), blank.len(), ptr::null_mut(), 0, &mut count, ptr::null_mut()) };
    assert_eq!(rc, SDE_ERR_NOT_FORMATTED);
    let rc = unsafe { sde_disk_list(blank.as_ptr(), 100, ptr::null_mut(), 0, &mut count, ptr::null_mut()) };
    assert_eq!(rc, SDE_ERR_ARGS);
}

#[test]
fn disk_put_and_delete() {
    use sharpdx::floppy_image::Side;
    let mut side = floppy_side("formatted.floppy.yaml", Side::A);
    let when = SdeDiskTime { month: 9, day: 22, hour: 12, minute: 0, second: 0 };
    let put = |side: &mut Vec<u8>, name: &str, data: &[u8], mode, start, run, flags| {
        let n = CString::new(name).unwrap();
        unsafe { sde_disk_put(side.as_mut_ptr(), side.len(), n.as_ptr(), data.as_ptr(), data.len(), mode, start, run, flags, &when) }
    };
    let listing = b"10 PRINT \"HI\"\n20 END\n";
    assert_eq!(put(&mut side, "HI.BAS", listing, SdeDiskPutMode::Auto, 0, 0, 0), SDE_OK, "{}", last_error());
    assert_eq!(put(&mut side, "HIA.BAS", listing, SdeDiskPutMode::AsciiListing, 0, 0, 0), SDE_OK);
    assert_eq!(put(&mut side, "NOTE.TXT", "Grüße\n".as_bytes(), SdeDiskPutMode::Auto, 0, 0, 0), SDE_OK);
    assert_eq!(put(&mut side, "MC.BIN", &[0xC9], SdeDiskPutMode::Machine, 0x01C000, SDE_DISK_NO_RUN, 0), SDE_OK);
    let pun = b"                 50                     1 MARTIN\r\n\x1A";
    assert_eq!(put(&mut side, "SCORE.PUN", pun, SdeDiskPutMode::Auto, 0, 0, 0), SDE_OK);
    assert_eq!(put(&mut side, "SCORE.BAS", pun, SdeDiskPutMode::Tokenize, 0, 0, 0), SDE_OK, "{}", last_error());
    let before = side.clone();
    assert_eq!(put(&mut side, "HI.BAS", listing, SdeDiskPutMode::Auto, 0, 0, 0), SDE_ERR_EXISTS);
    assert_eq!(put(&mut side, "BLOB", &[0, 1, 2], SdeDiskPutMode::Auto, 0, 0, 0), SDE_ERR);
    assert_eq!(put(&mut side, "BAD NAME", listing, SdeDiskPutMode::Auto, 0, 0, 0), SDE_ERR_BAD_NAME);
    assert_eq!(put(&mut side, "BIG", &vec![1u8; 70000], SdeDiskPutMode::Raw, 0, 0, 0), SDE_ERR_DISK_FULL);
    assert_eq!(side, before, "a failed put must not change the side");
    assert_eq!(put(&mut side, "HI.BAS", b"10 END\n", SdeDiskPutMode::Auto, 0, 0, SDE_DISK_FORCE), SDE_OK);

    let (files, _) = list(&side);
    let kinds: Vec<(&str, SdeDiskKind)> = files.iter().map(|(n, k, _)| (n.as_str(), *k)).collect();
    assert_eq!(
        kinds,
        vec![
            ("HI.BAS", SdeDiskKind::Basic),
            ("HIA.BAS", SdeDiskKind::AsciiBasic),
            ("NOTE.TXT", SdeDiskKind::Text),
            ("MC.BIN", SdeDiskKind::Machine),
            ("SCORE.PUN", SdeDiskKind::Text),
            ("SCORE.BAS", SdeDiskKind::Basic)
        ]
    );

    let pat = CString::new("HI*.BAS").unwrap();
    let mut deleted = 0usize;
    let rc = unsafe { sde_disk_delete(side.as_mut_ptr(), side.len(), pat.as_ptr(), 0, &mut deleted) };
    assert_eq!((rc, deleted), (SDE_OK, 2));
    let rc = unsafe { sde_disk_delete(side.as_mut_ptr(), side.len(), pat.as_ptr(), 0, &mut deleted) };
    assert_eq!(rc, SDE_ERR_NOT_FOUND);
    assert_eq!(list(&side).0.len(), 4);
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
        "sde_disk_list",
        "sde_disk_get",
        "sde_disk_put",
        "sde_disk_delete",
        "SdeDirEntry",
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
