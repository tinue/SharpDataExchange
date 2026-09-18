//! CP437 <-> text conversion.
//!
//! The Sharp PC-1500/1500A/1600 character set is treated as IBM PC Code Page 437.
//! ASCII `0x00..=0x7F` is identity; `0x80..=0xFF` are the CP437 glyphs. Every byte
//! decodes to some scalar, and (for CP437) the mapping is a bijection.

#[rustfmt::skip]
mod tables {
    include!("cp437_tables.rs");
}

use tables::{FROM_CHAR, TO_CHAR};

/// Decode a single unsigned byte to its CP437 character. Total over `0x00..=0xFF`.
#[inline]
pub fn to_char(b: u8) -> char {
    TO_CHAR[b as usize]
}

/// Decode a byte slice to an owned `String` (never fails).
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| to_char(b)).collect()
}

/// Encode a single char to a CP437 byte, or `None` if unrepresentable.
#[inline]
pub fn to_byte(c: char) -> Option<u8> {
    let key = c as u32;
    FROM_CHAR
        .binary_search_by_key(&key, |&(k, _)| k)
        .ok()
        .map(|i| FROM_CHAR[i].1)
}

/// Encode text to CP437 bytes. Unrepresentable characters become `?` (0x3F).
pub fn encode_lossy(s: &str) -> Vec<u8> {
    s.chars().map(|c| to_byte(c).unwrap_or(b'?')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_identity() {
        for b in 0u8..0x80 {
            assert_eq!(to_char(b) as u32, b as u32);
            assert_eq!(to_byte(to_char(b)), Some(b));
        }
    }

    #[test]
    fn known_high_glyphs() {
        assert_eq!(to_char(0x9A), 'Ü');
        assert_eq!(to_char(0x8E), 'Ä');
        assert_eq!(to_char(0x81), 'ü');
        assert_eq!(to_byte('Ü'), Some(0x9A));
    }

    #[test]
    fn roundtrip_all_bytes() {
        for b in 0u8..=0xFF {
            assert_eq!(to_byte(to_char(b)), Some(b), "byte {b:#04x}");
        }
    }

    #[test]
    fn unrepresentable_becomes_question_mark() {
        assert_eq!(encode_lossy("\u{2603}"), vec![b'?']); // snowman
    }
}
