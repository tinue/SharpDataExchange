//! Content-type detection. `convert` chooses its direction from this, never from the
//! file name.

use crate::header::{self, FileType};
use crate::registry::Device;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// A plain-text BASIC listing (`<digits><space>...` lines).
    AsciiBasic,
    /// Tokenized BASIC behind a CE-158 header (PC-1500 family).
    Ce158Basic,
    /// Tokenized BASIC behind a PC-1600 header.
    Pc1600Basic,
    /// Machine language behind a CE-158 header (PC-1500 family). Not handled by
    /// `convert`/`paths` (BASIC-only); used by `get`/`put`.
    Ce158Machine,
    /// Machine language behind a PC-1600 header. Not handled by `convert`/`paths`
    /// (BASIC-only); used by `get`/`put`.
    Pc1600Machine,
    /// Reserve Area (key-assignment layers), binary behind a CE-158 header or
    /// headerless SDAR ASCII text. PC-1500/1500A only; not handled by `convert`.
    Ce158Reserve,
    /// Variables, binary behind a CE-158 header or headerless SDAV ASCII text.
    /// PC-1500/1500A only; not handled by `convert`.
    Ce158Variables,
    /// Plain UTF-8 text that is not a BASIC listing or marker format (assembler
    /// source, config files, …). Every character maps to the Sharp (CP437) set.
    Text,
    /// Anything else: binary data, or text with characters outside the Sharp set.
    Unknown,
}

impl Content {
    pub fn describe(self) -> &'static str {
        match self {
            Content::AsciiBasic => "ASCII BASIC",
            Content::Ce158Basic => "tokenized BASIC (CE-158 header)",
            Content::Pc1600Basic => "tokenized BASIC (PC-1600 header)",
            Content::Ce158Machine => "machine language (CE-158 header)",
            Content::Pc1600Machine => "machine language (PC-1600 header)",
            Content::Ce158Reserve => "Reserve Area (CE-158 header)",
            Content::Ce158Variables => "Variables (CE-158 header)",
            Content::Text => "text",
            Content::Unknown => "unrecognized content",
        }
    }

    /// The tokenized-BASIC device, if this is a tokenized-BASIC kind.
    pub fn tokenized_device(self) -> Option<Device> {
        match self {
            Content::Ce158Basic => Some(Device::Pc1500),
            Content::Pc1600Basic => Some(Device::Pc1600),
            _ => None,
        }
    }
}

pub fn detect(data: &[u8]) -> Content {
    detect_from_header(header::find(data).as_ref(), data)
}

/// Same as [`detect`], but for a caller that has already located the header (or knows
/// there isn't one) and wants to avoid re-scanning `data` for the header magic.
pub fn detect_from_header(header: Option<&header::ParsedHeader>, data: &[u8]) -> Content {
    if let Some(h) = header {
        return match h.device {
            Device::Pc1500 => match h.file_type {
                FileType::Basic => Content::Ce158Basic,
                FileType::Machine => Content::Ce158Machine,
                FileType::Reserve => Content::Ce158Reserve,
                FileType::Variables => Content::Ce158Variables,
            },
            Device::Pc1600 => match h.file_type {
                FileType::Basic => Content::Pc1600Basic,
                FileType::Machine => Content::Pc1600Machine,
                // No PC-1600 header ever decodes to these -- header::pc1600_file_type
                // never maps a type byte to them.
                FileType::Reserve | FileType::Variables => {
                    unreachable!("PC-1600 headers never decode to Reserve/Variables")
                }
            },
        };
    }
    let first_line = first_non_blank_line(data);
    if first_line.as_deref().is_some_and(|l| l.starts_with(crate::reserve::MARKER)) {
        Content::Ce158Reserve
    } else if first_line.as_deref().is_some_and(|l| l.starts_with(crate::variables::MARKER)) {
        Content::Ce158Variables
    } else if looks_like_ascii_basic(data) {
        Content::AsciiBasic
    } else if looks_like_text(data) {
        Content::Text
    } else {
        Content::Unknown
    }
}

/// The first non-blank line of `data` decoded as CP437 text, or `None` if `data` isn't
/// decodable as plain text at all (contains binary control bytes).
fn first_non_blank_line(data: &[u8]) -> Option<String> {
    if data
        .iter()
        .any(|&b| b < 0x20 && !matches!(b, 0x09 | 0x0A | 0x0D | 0x1A))
    {
        return None;
    }
    let text = crate::cp437::decode(data);
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

fn looks_like_ascii_basic(data: &[u8]) -> bool {
    // Reject if it contains control bytes other than TAB/LF/CR/SUB — i.e. it's binary.
    if data
        .iter()
        .any(|&b| b < 0x20 && !matches!(b, 0x09 | 0x0A | 0x0D | 0x1A))
    {
        return false;
    }
    // A DOS/PC-1600 end-of-file byte ends the listing.
    let data = match data.iter().position(|&b| b == 0x1A) {
        Some(i) => &data[..i],
        None => data,
    };
    let text = crate::cp437::decode(data);
    let mut checked = 0usize;
    let mut matched = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        checked += 1;
        if is_basic_line(line.trim_end()) {
            matched += 1;
        }
        if checked == 5 {
            break;
        }
    }
    checked > 0 && matched >= checked.min(3)
}

/// Host text: valid UTF-8 (an optional BOM is ignored), no control bytes other than
/// TAB/LF/CR/SUB, and every character representable in the Sharp (CP437) set.
pub fn looks_like_text(data: &[u8]) -> bool {
    let data = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    !text.trim().is_empty()
        && text.chars().all(|c| {
            if (c as u32) < 0x20 {
                matches!(c, '\t' | '\n' | '\r' | '\u{1A}')
            } else {
                crate::cp437::to_byte(c).is_some()
            }
        })
}

/// A listing line: at most [`MAX_LINE_INDENT`] spaces/tabs, a line number, exactly one
/// space or tab, then the statement — `^[ \t]{0,4}\d+[ \t]\S`.
///
/// Deliberately strict: text wrongly taken for BASIC gets tokenized by `put` and is
/// destroyed, while a listing wrongly taken for text still works (the PC-1600 `LOAD`s an
/// ASCII program file) and `put -f binary` forces tokenization. So a column of numbers
/// (`      50        1 MARTIN`, a high-score file) must not pass. Line numbers are not
/// required to increase: `#SEGMENT` restarts them.
fn is_basic_line(line: &str) -> bool {
    let body = line.trim_start_matches([' ', '\t']);
    if line.len() - body.len() > MAX_LINE_INDENT {
        return false;
    }
    let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return false;
    }
    let mut rest = body[digits..].chars();
    matches!(rest.next(), Some(' ' | '\t')) && rest.next().is_some_and(|c| !c.is_whitespace())
}

/// Most leading whitespace before a line number: device listings right-align line
/// numbers up to 65279, i.e. at most 4 spaces.
const MAX_LINE_INDENT: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;

    #[test]
    fn ascii_listing() {
        assert_eq!(detect(b"10 PRINT \"HI\"\n20 END\n"), Content::AsciiBasic);
        assert_eq!(detect(b"10 PRINT \"HI\"\r\n20 END\r\n\x1A"), Content::AsciiBasic);
        assert_eq!(detect(b"   10 \"A\":CLEAR\n  20 GOTO 10\n"), Content::AsciiBasic);
    }

    #[test]
    fn not_basic() {
        assert_eq!(detect(b"hello world\nthis is prose\n"), Content::Text);
        assert_eq!(detect(&[0x00, 0x01, 0x02, 0x03]), Content::Unknown);
    }

    #[test]
    fn number_columns_are_text_not_basic() {
        // A game's high-score file (ATLANTIS.PUN): a number, many spaces, more fields.
        let pun = b"                 50                     1 MARTIN\r\n\x1A";
        assert_eq!(detect(pun), Content::Text);
        assert_eq!(detect(b"10  PRINT\n"), Content::Text); // two spaces after the number
        assert_eq!(detect(b"     10 PRINT\n"), Content::Text); // indented more than 4
        assert_eq!(detect(b"65279 END\n"), Content::AsciiBasic);
        assert_eq!(detect(b"    10 PRINT\n"), Content::AsciiBasic);
        assert_eq!(detect(b"10\tPRINT\n"), Content::AsciiBasic);
        // #SEGMENT restarts line numbers; decreasing numbers are fine.
        assert_eq!(detect(b"10 GOSUB \"A\"\n20 END\n#SEGMENT\n10 \"A\" RETURN\n"), Content::AsciiBasic);
    }

    #[test]
    fn text_detection() {
        assert_eq!(detect("; Z80 source\n\tLD A,0 ; Grüße\r\n".as_bytes()), Content::Text);
        assert_eq!(detect("\u{FEFF}KEY=VALUE\n".as_bytes()), Content::Text);
        // Outside the Sharp character set -> not text.
        assert_eq!(detect("snow \u{2603}\n".as_bytes()), Content::Unknown);
        // CP437 bytes that are not valid UTF-8 are not host text.
        assert_eq!(detect(&[b'A', 0x81, b'\n']), Content::Unknown);
        assert_eq!(detect(b"  \n"), Content::Unknown);
    }

    #[test]
    fn tokenized_with_headers() {
        let mut ce = header::build(Device::Pc1500, Some("x"), 4);
        ce.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        assert_eq!(detect(&ce), Content::Ce158Basic);

        let mut p16 = header::build(Device::Pc1600, None, 4);
        p16.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        assert_eq!(detect(&p16), Content::Pc1600Basic);
    }

    #[test]
    fn headerless_tokenized_is_unknown() {
        // LineLengthTest.bin style: starts 00 0A 07 22 ...
        assert_eq!(detect(&[0x00, 0x0A, 0x07, 0x22, 0x41, 0x22, 0xF1, 0xB3, 0x30, 0x0D]), Content::Unknown);
    }

    #[test]
    fn headered_reserve_and_variables() {
        let h = header::build_header(header::BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Reserve,
            name: Some("x"),
            payload_len: 188,
            start_addr: 0,
            run_addr: 0,
        });
        assert_eq!(detect(&h), Content::Ce158Reserve);

        let h = header::build_header(header::BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Variables,
            name: Some("x"),
            payload_len: 12,
            start_addr: 0,
            run_addr: 0,
        });
        assert_eq!(detect(&h), Content::Ce158Variables);
    }

    #[test]
    fn headerless_marker_wins_over_ascii_basic_heuristic() {
        // A body that would otherwise look like a BASIC listing must not shadow the
        // marker line.
        let text = "; sde-reserve:1.0 pc1500\n\n[layer 1]\nlabel: 10 PRINT\nkey 1: 20 GOTO 10\n";
        assert_eq!(detect(text.as_bytes()), Content::Ce158Reserve);

        let text = "; sde-variables:1.0 pc1500\n; Count: 1\n10\n";
        assert_eq!(detect(text.as_bytes()), Content::Ce158Variables);
    }
}
