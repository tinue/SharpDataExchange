//! Pure, content-driven conversion — the shared core behind the CLI and the C ABI.
//! No filesystem access. Mirrors `SharpDataExchange.runConvert` / `encodeAsciiBasic`.

use anyhow::{bail, Result};

use crate::detect::{self, Content};
use crate::detokenize::LineEnding;
use crate::registry::{Device, Registry};
use crate::scanner::SegmentMarker;
use crate::{abbrev, detokenize, header, scanner, text};

/// Result of a conversion.
#[derive(Debug)]
pub struct ConvertOutcome {
    /// The output bytes: a header + tokenized payload when tokenizing (unless
    /// `with_header` was false), or a UTF-8 ASCII listing when de-tokenizing.
    pub bytes: Vec<u8>,
    /// What the input was detected as.
    pub content: Content,
    /// The device actually used (taken from the input header when de-tokenizing).
    pub device: Device,
}

/// Convert `input`, choosing direction from its detected content.
///
/// * ASCII BASIC in  -> tokenized payload out (prefixed with the CE-158 / PC-1600
///   serial header when `with_header`). `device` selects the keyword table + header;
///   `name` supplies the CE-158 filename.
/// * Tokenized BASIC in (with a CE-158 / PC-1600 header) -> ASCII listing out. The
///   device is taken from the header; `device` and `name` are ignored.
///
/// A de-tokenized listing is terminated with the host-default line ending (`\r\n` on
/// Windows, `\n` elsewhere); use [`convert_with`] to override it. `CR` / `CRLF` input to
/// a tokenize is always accepted regardless of platform.
///
/// A `#SEGMENT` marker line tokenizes to its wire form ([`SegmentMarker::Wire`]) --
/// what an actual `SAVE "COM1:"` transmits. Use [`convert_with`] for
/// [`SegmentMarker::Memory`] when building bytes to poke directly into RAM instead.
pub fn convert(
    input: &[u8],
    device: Device,
    name: Option<&str>,
    with_header: bool,
) -> Result<ConvertOutcome> {
    convert_with(input, device, name, with_header, LineEnding::Platform, SegmentMarker::Wire)
}

/// As [`convert`], but with an explicit [`LineEnding`] for a de-tokenized listing and
/// an explicit [`SegmentMarker`] style for a `#SEGMENT` line when tokenizing (ignored
/// when de-tokenizing). When tokenizing, `eol` is unused (`CR` / `CRLF` input is
/// always accepted).
pub fn convert_with(
    input: &[u8],
    device: Device,
    name: Option<&str>,
    with_header: bool,
    eol: LineEnding,
    segment_marker: SegmentMarker,
) -> Result<ConvertOutcome> {
    let content = detect::detect(input);
    match content {
        Content::AsciiBasic => {
            let listing = text::decode_bas_listing(input);
            text::require_ascii_for_pc1500(&listing, device)?;
            let reg = Registry::for_device(device);
            let expanded = expand_all(&listing, reg);
            let payload = scanner::tokenize(&expanded, reg, segment_marker)?;
            let bytes = if with_header {
                let mut out = header::build(device, name, payload.len());
                out.extend_from_slice(&payload);
                out
            } else {
                payload
            };
            Ok(ConvertOutcome { bytes, content, device })
        }
        Content::Ce158Basic | Content::Pc1600Basic => {
            let h = header::find(input).expect("detect() guarantees a header here");
            let payload = &input[h.payload_start()..];
            let reg = Registry::for_device(h.device);
            let listing = detokenize::detokenize_to_text(payload, reg, eol)?;
            Ok(ConvertOutcome { bytes: listing.into_bytes(), content, device: h.device })
        }
        Content::Ce158Machine | Content::Pc1600Machine | Content::Unknown => bail!(
            "convert only handles BASIC; got {}. A tokenized file must include a CE-158 or PC-1600 header.",
            content.describe()
        ),
    }
}

fn expand_all(listing: &str, reg: &Registry) -> String {
    let mut out = String::with_capacity(listing.len());
    for (i, line) in listing.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&abbrev::expand(line, reg));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_to_tokenized_and_back() {
        let src = b"10 \"A\":CLEAR :WAIT\n20 GOTO 10\n";
        let out = convert(src, Device::Pc1500, Some("t"), true).unwrap();
        assert_eq!(out.content, Content::AsciiBasic);
        assert_eq!(&out.bytes[0..5], &[0x01, 0x40, b'C', b'O', b'M']);

        let back =
            convert_with(&out.bytes, Device::Pc1500, None, true, LineEnding::Lf, SegmentMarker::Wire)
                .unwrap();
        assert_eq!(back.content, Content::Ce158Basic);
        assert_eq!(back.bytes, b"10 \"A\":CLEAR :WAIT\n20 GOTO 10\n");
    }

    #[test]
    fn tokenize_accepts_cr_and_crlf_input_on_any_platform() {
        let lf = convert(b"10 \"A\":WAIT\n20 GOTO 10\n", Device::Pc1500, Some("t"), true).unwrap();
        let crlf = convert(b"10 \"A\":WAIT\r\n20 GOTO 10\r\n", Device::Pc1500, Some("t"), true).unwrap();
        let cr = convert(b"10 \"A\":WAIT\r20 GOTO 10\r", Device::Pc1500, Some("t"), true).unwrap();
        assert_eq!(lf.bytes, crlf.bytes, "CRLF input must tokenize identically to LF");
        assert_eq!(lf.bytes, cr.bytes, "bare-CR input must tokenize identically to LF");
    }

    #[test]
    fn detokenize_line_ending_is_overridable() {
        let bbin = convert(b"10 GOTO 10\n20 END\n", Device::Pc1500, Some("t"), true).unwrap().bytes;
        let crlf =
            convert_with(&bbin, Device::Pc1500, None, true, LineEnding::CrLf, SegmentMarker::Wire)
                .unwrap();
        assert_eq!(crlf.bytes, b"10 GOTO 10\r\n20 END\r\n");
        let cr = convert_with(&bbin, Device::Pc1500, None, true, LineEnding::Cr, SegmentMarker::Wire)
            .unwrap();
        assert_eq!(cr.bytes, b"10 GOTO 10\r20 END\r");
    }

    #[test]
    fn unknown_rejected() {
        assert!(convert(b"just some prose here\n", Device::Pc1500, None, true).is_err());
    }

    #[test]
    fn segment_marker_style_is_threaded_through() {
        let src = b"5 \"A\"\n10 END\n#SEGMENT\n5 \"B\"\n10 END\n";
        let wire = convert_with(src, Device::Pc1600, None, false, LineEnding::Platform, SegmentMarker::Wire)
            .unwrap();
        let mem = convert_with(src, Device::Pc1600, None, false, LineEnding::Platform, SegmentMarker::Memory)
            .unwrap();
        assert_eq!(wire.bytes.len(), mem.bytes.len() + 2);
        assert!(wire.bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
        assert!(!mem.bytes.windows(3).any(|w| w == [0xFF, 0x00, 0x00]));
    }

    #[test]
    fn pc1500_rejects_non_ascii() {
        let e = convert("10 PRINT \"\u{00dc}\"\n".as_bytes(), Device::Pc1500, None, true).unwrap_err();
        assert!(e.to_string().contains("7-bit"));
        // PC-1600 accepts it
        assert!(convert("10 PRINT \"\u{00dc}\"\n".as_bytes(), Device::Pc1600, None, true).is_ok());
    }
}
