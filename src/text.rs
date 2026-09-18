//! ASCII BASIC listing decode / normalization.

/// Decode a `.bas` file's bytes to text: strict UTF-8 first, else CP437. Then normalize
/// line endings (CRLF / CR -> LF) and strip any trailing `0x1A` (SUB / DOS EOF) markers.
pub fn decode_bas_listing(raw: &[u8]) -> String {
    let decoded = match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => crate::cp437::decode(raw),
    };
    normalize(&decoded)
}

fn normalize(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = s.trim_end_matches('\u{001A}');
    trimmed.to_string()
}

/// First character with a code point `> 0x7F`, as `(line, col, ch)` with 1-based line and
/// column. Used to reject non-7-bit input when targeting the PC-1500. `None` if all ASCII.
pub fn first_non_ascii_for_pc1500(text: &str) -> Option<(usize, usize, char)> {
    let mut line = 1usize;
    let mut col = 1usize;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            col = 1;
            continue;
        }
        if ch as u32 > 0x7F {
            return Some((line, col, ch));
        }
        col += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_eof_marker() {
        assert_eq!(decode_bas_listing(b"10 PRINT\r\n20 END\r\n\x1a"), "10 PRINT\n20 END\n");
        assert_eq!(decode_bas_listing(b"10 A\r20 B"), "10 A\n20 B");
    }

    #[test]
    fn cp437_fallback() {
        // 0x9A is 'Ü' in CP437 and not valid UTF-8 on its own.
        assert_eq!(decode_bas_listing(&[b'1', b'0', b' ', 0x9A]), "10 Ü");
    }

    #[test]
    fn seven_bit_check() {
        assert_eq!(first_non_ascii_for_pc1500("10 PRINT\n20 END\n"), None);
        assert_eq!(first_non_ascii_for_pc1500("10 A\n20 \u{00dc}"), Some((2, 4, 'Ü')));
    }
}
