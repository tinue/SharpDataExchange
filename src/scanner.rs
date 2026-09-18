//! ROM-style BASIC tokenizer: ASCII listing text -> tokenized payload bytes (no header).
//!
//! Single left-to-right pass per line, modelled on the PC-1500 ROM tokenizer
//! (`TOK_INBUF` at `$F957`) rather than a grammar:
//!
//! 1. `'` (0x27) is a `REM` shorthand: it is kept, and everything after it on the line
//!    is copied through verbatim with no further parsing (same as the `REM` case below).
//!    A line that is only a line number plus `'` therefore tokenizes to a single-byte
//!    body — the machine accepts it; the old ANTLR path rejected it.
//! 2. `"` toggles verbatim string mode; inside a string every byte (spaces included) is
//!    copied through.
//! 3. Spaces outside strings are dropped.
//! 4. A byte `>= 0xE0` is treated as the lead of an already-tokenized 2-byte keyword and
//!    copied through with the following byte.
//! 5. `A..=Z` starts a greedy longest keyword match against the active registry. A hit
//!    emits the 2-byte token; a miss stores the single letter and rescans from the next
//!    character (so `FORMAT` used as a name would tokenize `FOR` + `MAT`, exactly as the
//!    ROM does). Immediately after emitting `REM`, the rest of the line is copied
//!    verbatim.
//! 6. Everything else (digits, operators, `:` `;` `,` `#` `@`, high CP437 bytes) is
//!    stored as its CP437 byte(s).
//!
//! Only upper-case `A..=Z` triggers keyword matching, matching the ROM (and the old
//! grammar, whose keyword literals are upper-case).

use anyhow::{bail, Result};

use crate::cp437;
use crate::registry::{Device, Registry, REM_CODE};

const CR: u8 = 0x0D;
/// Max tokenized content bytes per line: the length prefix is a single byte and also
/// counts the trailing `0x0D`, so `content + 1 <= 255`.
const MAX_CONTENT: usize = 254;

/// How a `#SEGMENT` marker line is rendered into bytes.
///
/// A device saving multiple GOSUB "LABEL" program segments together emits, on the
/// wire, a 3-byte sentinel: `0xFF` (end of this segment) followed by two `0x00`
/// bytes. But the ROM's own serial *receiver* only stores that leading `0xFF` into
/// the program area -- the two trailing bytes never land in RAM (confirmed against
/// a real PC-1600's `LOAD "COM1:"` pointers: `BASPRG_END` comes out exactly 2 bytes
/// short of the 3-byte form, once per marker). So a caller building bytes to go out
/// over a real or emulated serial line wants [`SegmentMarker::Wire`]; a caller
/// building bytes to poke directly into RAM (bypassing the serial protocol
/// entirely, e.g. a fast preset loader) wants [`SegmentMarker::Memory`], which
/// emits just the `0xFF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentMarker {
    /// `0xFF 0x00 0x00` -- what actually goes out over COM1:.
    Wire,
    /// `0xFF` alone -- what the ROM's receiver actually stores in the program area.
    Memory,
}

/// Tokenize a full ASCII BASIC listing. `source` must already have had dotted
/// abbreviations expanded (see [`crate::abbrev`]); newlines may be `\n` or `\r\n`.
/// `marker` selects how a `#SEGMENT` line is rendered -- see [`SegmentMarker`]. A bare
/// `99999` line is also accepted as an alias for `#SEGMENT` (tokenizing only).
pub fn tokenize(source: &str, reg: &Registry, marker: SegmentMarker) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for raw in source.split('\n') {
        let line = raw.trim_start_matches([' ', '\t', '\r']);
        // "#SEGMENT" is the reserved marker line for the boundary between two named
        // GOSUB "LABEL" program segments saved together (confirmed on real PC-1600
        // hardware). Checked before the generic comment-drop rule below, since it
        // also starts with '#'. See `detokenize::detokenize`'s handling of the same
        // byte sequence, and [`SegmentMarker`] for which bytes this emits.
        //
        // A bare `99999` line is accepted here too: real downloaded PC-1600 listings
        // conventionally use line number 99999 as a segment separator instead of the
        // `#SEGMENT` marker. This is a tokenize-only convenience alias -- detokenize
        // always emits `#SEGMENT`, never `99999`, so there's no round-trip ambiguity.
        let trimmed_end = line.trim_end_matches([' ', '\t', '\r']);
        if trimmed_end == "#SEGMENT" || trimmed_end == "99999" {
            match marker {
                SegmentMarker::Wire => out.extend_from_slice(&[0xFF, 0x00, 0x00]),
                SegmentMarker::Memory => out.push(0xFF),
            }
            continue;
        }
        // Column-0 `//` or `#` documentation comments are never sent to the device.
        if line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(line.len());
        if digits_end == 0 {
            // No line number -> unnumbered content is dropped.
            continue;
        }
        let line_no = (line[..digits_end].parse::<u64>().unwrap_or(0) & 0xFFFF) as u16;
        let rest = &line[digits_end..];

        let content = scan_line(rest, reg);
        if content.len() > MAX_CONTENT {
            bail!(
                "line {line_no} is too long: {} tokenized bytes (max {MAX_CONTENT})",
                content.len()
            );
        }
        out.extend_from_slice(&line_no.to_be_bytes());
        out.push((content.len() + 1) as u8);
        out.extend_from_slice(&content);
        out.push(CR);
    }
    Ok(out)
}

fn scan_line(rest: &str, reg: &Registry) -> Vec<u8> {
    let bytes = cp437::encode_lossy(rest);
    let mut content = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut in_string = false;
    // PC-1600 patches a constant line-number target into a compact binary form
    // (0x1F [hi] [lo] 0x00) instead of plain ASCII digits, right after GOTO/GOSUB/
    // THEN/RESUME/RUN/RESTORE; PC-1500 always uses ASCII digits. Confirmed against
    // real memory dumps of the PC-1600 for each of those keywords (e.g. RESUME via
    // "10 RESUME 100" -> ... F2 8D 1F 00 64 00 0D). `ON x GOTO/GOSUB n` needs no
    // separate handling: the target directly follows GOTO/GOSUB, already in this
    // list, so it's covered the same way (confirmed: "10 ON A GOSUB 100" -> ...
    // F1 9C 41 F1 94 1F 00 64 00 0D). Set right after emitting one of these
    // keywords; consumed (or dropped) by the very next token.
    let use_binary_line_number_targets = reg.device() == Device::Pc1600;
    let mut expect_line_number_target = false;

    while i < bytes.len() {
        let b = bytes[i];

        if in_string {
            content.push(b);
            i += 1;
            if b == 0x22 {
                in_string = false;
            }
            continue;
        }

        if b == 0x20 {
            // Space outside a string is transparent to the line-number-target flag:
            // "GOSUB 50" (space from normalized/user text) must still be recognized,
            // same as the glued "GOSUB50".
            i += 1;
            continue;
        }

        let was_expecting_line_number_target = expect_line_number_target;
        expect_line_number_target = false;

        if use_binary_line_number_targets && was_expecting_line_number_target && b.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() && i - start < 5 {
                i += 1;
            }
            if let Ok(target) = std::str::from_utf8(&bytes[start..i]).unwrap().parse::<u32>() {
                if target <= 0xFFFF {
                    content.push(0x1F);
                    content.extend_from_slice(&(target as u16).to_be_bytes());
                    content.push(0x00);
                    continue;
                }
            }
            // Not a plain 16-bit line number after all (e.g. overflowed): fall back
            // to emitting the digits as plain CP437 bytes.
            content.extend_from_slice(&bytes[start..i]);
            continue;
        }

        match b {
            0x27 => {
                // ' — REM shorthand: keep it, copy the rest of the line verbatim.
                content.push(b);
                content.extend_from_slice(&bytes[i + 1..]);
                i = bytes.len();
            }
            0x22 => {
                content.push(b);
                in_string = true;
                i += 1;
            }
            0xE0..=0xFF => {
                // Already-tokenized keyword lead byte: pass this and the next byte.
                content.push(b);
                i += 1;
                if i < bytes.len() {
                    content.push(bytes[i]);
                    i += 1;
                }
            }
            b'A'..=b'Z' => match reg.longest_keyword_prefix(&bytes[i..]) {
                Some(kw) => {
                    content.extend_from_slice(&kw.code.to_be_bytes());
                    i += kw.name.len();
                    if kw.code == REM_CODE {
                        content.extend_from_slice(&bytes[i..]);
                        i = bytes.len();
                    } else if matches!(kw.name, "GOTO" | "GOSUB" | "THEN" | "RESUME" | "RUN" | "RESTORE") {
                        expect_line_number_target = true;
                    }
                }
                None => {
                    content.push(b);
                    i += 1;
                }
            },
            _ => {
                content.push(b);
                i += 1;
            }
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    fn tok(src: &str) -> Vec<u8> {
        tokenize(src, registry::pc1500(), SegmentMarker::Wire).unwrap()
    }

    #[test]
    fn simple_line() {
        // 10 "A":CLEAR :WAIT   ->  00 0A 0A 22 41 22 3A F1 87 3A F1 B3 0D
        assert_eq!(
            tok("   10 \"A\":CLEAR :WAIT \n"),
            vec![0x00, 0x0A, 0x0A, 0x22, 0x41, 0x22, 0x3A, 0xF1, 0x87, 0x3A, 0xF1, 0xB3, 0x0D]
        );
    }

    #[test]
    fn glued_keyword_after_digit() {
        // 90 FOR I=1TO B  ->  content F1 A5 49 3D 31 F1 B1 42
        assert_eq!(
            tok("90 FOR I=1TO B\n"),
            vec![0x00, 0x5A, 0x09, 0xF1, 0xA5, 0x49, 0x3D, 0x31, 0xF1, 0xB1, 0x42, 0x0D]
        );
    }

    #[test]
    fn empty_line_is_empty_body_not_an_error() {
        assert_eq!(tok("10\n"), vec![0x00, 0x0A, 0x01, 0x0D]);
    }

    #[test]
    fn quote_is_rem_shorthand_and_copies_rest_verbatim() {
        // 10 'X1 = 1.POSITION ON STACK -> ' kept, rest untouched (no keyword matching)
        let got = tok("10 'X1 = 1.POSITION ON STACK\n");
        assert_eq!(got[..3], [0x00, 0x0A, 0x1A]);
        assert_eq!(&got[3..], b"'X1 = 1.POSITION ON STACK\r");

        // A bare quote still keeps the byte instead of vanishing.
        assert_eq!(tok("10 '\n"), vec![0x00, 0x0A, 0x02, b'\'', 0x0D]);
    }

    #[test]
    fn rem_copies_rest_verbatim() {
        // 10 REM FOR I  -> F1 AB then " FOR I" literal (FOR not tokenized)
        let got = tok("10 REM FOR I\n");
        assert_eq!(&got[..3], &[0x00, 0x0A, 0x09]);
        assert_eq!(&got[3..], &[0xF1, 0xAB, b' ', b'F', b'O', b'R', b' ', b'I', 0x0D]);
    }

    #[test]
    fn keyword_inside_string_is_literal() {
        // 40 INPUT "REM. RATE (%)?";O
        let got = tok("40 INPUT \"REM. RATE (%)?\";O\n");
        // starts: line 40, then F0 91 (INPUT), then the quoted text verbatim
        assert_eq!(&got[..5], &[0x00, 0x28, 0x15, 0xF0, 0x91]);
        assert!(got.windows(4).any(|w| w == b"REM.")); // literal, not a token
    }

    #[test]
    fn doc_comments_dropped() {
        assert!(tok("// hello\n# also\n").is_empty());
    }

    #[test]
    fn unnumbered_lines_dropped() {
        assert_eq!(tok("PRINT 1\n20 END\n"), tok("20 END\n"));
    }

    fn tok1600(src: &str) -> Vec<u8> {
        tokenize(src, registry::pc1600(), SegmentMarker::Wire).unwrap()
    }

    #[test]
    fn pc1600_binary_line_number_targets() {
        // Confirmed against real PC-1600 memory dumps: a constant GOTO/GOSUB/THEN/
        // RESUME/RUN/RESTORE target is patched into 0x1F [hi] [lo] 0x00 instead of
        // ASCII digits.
        // "10 RESUME 100" -> 0A 07 F2 8D 1F 00 64 00 0D
        assert_eq!(
            tok1600("10 RESUME 100\n"),
            vec![0x00, 0x0A, 0x07, 0xF2, 0x8D, 0x1F, 0x00, 0x64, 0x00, 0x0D]
        );
        // "10 RUN 100" -> 0A 07 F1 A4 1F 00 64 00 0D
        assert_eq!(
            tok1600("10 RUN 100\n"),
            vec![0x00, 0x0A, 0x07, 0xF1, 0xA4, 0x1F, 0x00, 0x64, 0x00, 0x0D]
        );
        // "10 RESTORE 100" -> 0A 07 F1 A7 1F 00 64 00 0D
        assert_eq!(
            tok1600("10 RESTORE 100\n"),
            vec![0x00, 0x0A, 0x07, 0xF1, 0xA7, 0x1F, 0x00, 0x64, 0x00, 0x0D]
        );
        // "10 ON A GOSUB 100" -> 0A 0A F1 9C 41 F1 94 1F 00 64 00 0D -- ON itself
        // needs no special-casing: the target follows GOSUB, already in the list.
        assert_eq!(
            tok1600("10 ON A GOSUB 100\n"),
            vec![0x00, 0x0A, 0x0A, 0xF1, 0x9C, 0x41, 0xF1, 0x94, 0x1F, 0x00, 0x64, 0x00, 0x0D]
        );
        let has_binary_target = |bytes: &[u8]| bytes.windows(4).any(|w| w == [0x1F, 0x00, 0x64, 0x00]);
        assert!(has_binary_target(&tok1600("10 GOTO 100\n")));
        assert!(has_binary_target(&tok1600("10 GOSUB 100\n")));
        assert!(has_binary_target(&tok1600("10 IF 1 THEN 100\n")));
    }

    #[test]
    fn pc1500_never_uses_binary_line_number_targets() {
        // The PC-1500 always uses plain ASCII digits, even for the same keywords.
        assert!(!tok("10 RESUME 100\n").contains(&0x1F));
        assert!(!tok("10 GOTO 100\n").contains(&0x1F));
    }

    #[test]
    fn line_too_long_errors() {
        let long = format!("10 {}\n", "\"x\";:".repeat(60)); // ~300 content bytes
        assert!(tokenize(&long, registry::pc1500(), SegmentMarker::Wire).is_err());
    }

    #[test]
    fn segment_marker_wire_vs_memory() {
        assert_eq!(
            tokenize("#SEGMENT\n", registry::pc1500(), SegmentMarker::Wire).unwrap(),
            vec![0xFF, 0x00, 0x00]
        );
        assert_eq!(
            tokenize("#SEGMENT\n", registry::pc1500(), SegmentMarker::Memory).unwrap(),
            vec![0xFF]
        );
    }

    #[test]
    fn bare_99999_line_is_a_segment_marker_alias() {
        assert_eq!(
            tokenize("99999\n", registry::pc1500(), SegmentMarker::Wire).unwrap(),
            tokenize("#SEGMENT\n", registry::pc1500(), SegmentMarker::Wire).unwrap(),
        );
        assert_eq!(
            tokenize("99999\n", registry::pc1500(), SegmentMarker::Memory).unwrap(),
            vec![0xFF]
        );
    }
}
