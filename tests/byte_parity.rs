//! Byte-for-byte fidelity against the checked-in reference `convert` fixtures.

use std::path::Path;

use sharpdx::registry::Device;
use sharpdx::{convert, scanner};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
}

/// Offsets of the CE-158 filename field (16 bytes at 0x05). `sde` upper-cases the
/// basename; the historical fixture stored it lower-case, so these bytes are "don't
/// care" here and the parity check compares everything else exactly and the filename
/// separately.
const CE158_NAME: std::ops::Range<usize> = 5..21;

#[test]
fn depreciation_pc1500_matches_ce158_fixture() {
    let src = fixture("depreciation.bas");
    let out = convert(&src, Device::Pc1500, Some("depreciation"), true).unwrap().bytes;
    let want = fixture("depreciation-tokenized-ce158header.bin");

    assert_eq!(out.len(), want.len(), "total length");
    // Payload: byte-identical.
    assert_eq!(&out[27..], &want[27..], "tokenized payload");
    // Header, everything but the filename field: byte-identical.
    assert_eq!(&out[0..CE158_NAME.start], &want[0..CE158_NAME.start], "header magic/type/COM");
    assert_eq!(&out[CE158_NAME.end..27], &want[CE158_NAME.end..27], "load/length/autorun");
    // Filename: current `sde` behaviour (upper-cased, NUL-padded).
    assert_eq!(&out[CE158_NAME], b"DEPRECIATION\0\0\0\0");
}

#[test]
fn depreciation_pc1600_matches_fixture_exactly() {
    let src = fixture("depreciation.bas");
    let out = convert(&src, Device::Pc1600, None, true).unwrap().bytes;
    assert_eq!(out, fixture("depreciation-tokenized-pc1600header.bin"));
}

#[test]
fn linelengthtest_headerless_payload_matches_fixture() {
    let src = fixture("LineLengthTest.bas");
    // Headerless payload only.
    let out = convert(&src, Device::Pc1500, None, false).unwrap().bytes;
    assert_eq!(out, fixture("LineLengthTest.bin"));
}

#[test]
fn direct_scanner_matches_headerless_fixture() {
    // Same as above but exercising scanner::tokenize directly (no abbreviation pass —
    // this listing has no dotted forms).
    let src = String::from_utf8(fixture("LineLengthTest.bas")).unwrap();
    let out = scanner::tokenize(&src, sharpdx::registry::pc1500(), sharpdx::SegmentMarker::Wire).unwrap();
    assert_eq!(out, fixture("LineLengthTest.bin"));
}

#[test]
fn tokenized_fixtures_survive_detokenize_then_tokenize() {
    for (name, device) in [
        ("depreciation-tokenized-ce158header.bin", Device::Pc1500),
        ("depreciation-tokenized-pc1600header.bin", Device::Pc1600),
    ] {
        let data = fixture(name);
        // header -> ASCII
        let listing = convert(&data, device, None, true).unwrap().bytes;
        // ASCII -> header again
        let retok = convert(&listing, device, Some("depreciation"), true).unwrap().bytes;
        // Payload must be reproduced exactly (skip the CE-158 filename field).
        let hdr = if device == Device::Pc1500 { 27 } else { 16 };
        assert_eq!(&retok[hdr..], &data[hdr..], "{name}: payload round-trip");
    }
}
