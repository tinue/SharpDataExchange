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
    /// Anything else: a header for an out-of-scope file type (Reserve, Variables),
    /// binary data, or text that does not look like a BASIC listing.
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
        return match (h.device, h.file_type) {
            (Device::Pc1500, FileType::Basic) => Content::Ce158Basic,
            (Device::Pc1500, FileType::Machine) => Content::Ce158Machine,
            (Device::Pc1600, FileType::Basic) => Content::Pc1600Basic,
            (Device::Pc1600, FileType::Machine) => Content::Pc1600Machine,
        };
    }
    if looks_like_ascii_basic(data) {
        Content::AsciiBasic
    } else {
        Content::Unknown
    }
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
}
