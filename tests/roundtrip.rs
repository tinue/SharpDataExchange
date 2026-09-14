//! Round-trip contract: line-number sequence preserved after one tokenize/de-tokenize
//! pass, and the de-tokenized listing is a fixed point on any further round-trip.

use std::path::Path;

use sharpdx::registry::Device;
use sharpdx::{convert, convert_with, scanner, LineEnding};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn line_numbers(listing: &[u8]) -> Vec<u32> {
    String::from_utf8_lossy(listing)
        .lines()
        .filter_map(|l| {
            let d: String = l.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            d.parse().ok()
        })
        .collect()
}

fn tokenize(src: &[u8], dev: Device) -> Vec<u8> {
    convert(src, dev, Some("t"), true).unwrap().bytes
}
fn detokenize(bbin: &[u8], dev: Device) -> Vec<u8> {
    convert(bbin, dev, None, true).unwrap().bytes
}

#[test]
fn roundtrip_contract() {
    for (name, dev) in [
        ("depreciation.bas", Device::Pc1500),
        ("vartest.bas", Device::Pc1500),
        ("printtest-1600.bas", Device::Pc1600),
        ("ce163f.bas", Device::Pc1500),
    ] {
        let src = fixture(name);
        let bbin1 = tokenize(&src, dev);
        let ascii1 = detokenize(&bbin1, dev);

        assert_eq!(
            line_numbers(&src),
            line_numbers(&ascii1),
            "{name}: line-number sequence changed on round-trip"
        );

        let bbin2 = tokenize(&ascii1, dev);
        let ascii2 = detokenize(&bbin2, dev);
        assert_eq!(
            String::from_utf8_lossy(&ascii1),
            String::from_utf8_lossy(&ascii2),
            "{name}: de-tokenized listing is not a fixed point"
        );
        // Tokenized form is stable too, once normalized.
        assert_eq!(bbin1[27.min(bbin1.len())..], bbin2[27.min(bbin2.len())..], "{name}: payload not stable");
    }
}

#[test]
fn empty_quote_comment_roundtrips_without_error() {
    let out = convert(b"10 '\n", Device::Pc1500, None, false).unwrap().bytes;
    assert_eq!(out, vec![0x00, 0x0A, 0x02, b'\'', 0x0D]);

    // With a header, then back to ASCII (LF forced so the assertion holds on Windows).
    let bbin = convert(b"10 '\n", Device::Pc1500, Some("t"), true).unwrap().bytes;
    let ascii = convert_with(&bbin, Device::Pc1500, None, true, LineEnding::Lf).unwrap().bytes;
    assert_eq!(ascii, b"10 '\n");
}

#[test]
fn line_length_boundary() {
    // 79-byte tokenized line (LineLengthTest line 20) is fine.
    assert!(convert(&fixture("LineLengthTest.bas"), Device::Pc1500, None, false).is_ok());

    // A synthetic >254-byte tokenized line is rejected.
    let huge = format!("10 {}\n", "\"x\";:".repeat(60));
    let err = scanner::tokenize(&huge, sharpdx::registry::pc1500()).unwrap_err();
    assert!(err.to_string().contains("too long"));
}
