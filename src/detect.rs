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
    /// Anything else: binary data, or text that does not look like any recognized
    /// listing/marker format.
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
    let text = crate::cp437::decode(data);
    let mut checked = 0usize;
    let mut matched = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        checked += 1;
        if is_basic_line(trimmed) {
            matched += 1;
        }
        if checked == 5 {
            break;
        }
    }
    checked > 0 && matched >= checked.min(3)
}

/// `^\d+\s` — one or more digits, then a whitespace character.
fn is_basic_line(line: &str) -> bool {
    let mut chars = line.chars();
    let mut saw_digit = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else {
            return saw_digit && c.is_whitespace();
        }
    }
    false // all digits, no delimiter
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;

    #[test]
    fn ascii_listing() {
        assert_eq!(detect(b"10 PRINT \"HI\"\n20 END\n"), Content::AsciiBasic);
        assert_eq!(detect(b"   10 \"A\":CLEAR\n  20 GOTO 10\n"), Content::AsciiBasic);
    }

    #[test]
    fn not_basic() {
        assert_eq!(detect(b"hello world\nthis is prose\n"), Content::Unknown);
        assert_eq!(detect(&[0x00, 0x01, 0x02, 0x03]), Content::Unknown);
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
