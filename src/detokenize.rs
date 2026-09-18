//! Tokenized payload bytes -> normalized ASCII BASIC listing, modelled on the ROM
//! `LIST` walk.
//!
//! Output is *normalized*, not the original source: `<lineNo><space>` then, for each
//! keyword, its full name plus exactly one trailing space and no leading space; literal
//! bytes are emitted with no added spacing. Round-trip contract: the line-number
//! sequence survives one pass, and the listing is a fixed point on further round-trips.

use anyhow::{bail, Result};

use crate::cp437;
use crate::registry::{Registry, REM_CODE};

/// De-tokenize `payload` (the bytes *after* any serial header) to one `String` per line.
pub fn detokenize(payload: &[u8], reg: &Registry) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut pos = 0usize;

    while pos + 1 < payload.len() {
        // Program terminator (real-hardware CSAVE files); the encoder never writes it.
        if payload[pos] == 0 && payload[pos + 1] == 0 {
            break;
        }
        // 0xFF 0x00 0x00: boundary between this named program segment and the next
        // one (the device supports multiple GOSUB "LABEL"-addressable sub-programs
        // per save; each restarts its own line numbering, and its own first line is
        // normally a `<n> "LABEL"` line naming it). Confirmed on real PC-1600
        // hardware: RUN executes the last segment first, then jumps to the first;
        // touching PRO/scroll after loading corrupts the program. "#SEGMENT" is the
        // reserved round-trippable marker line for this byte sequence (see
        // `scanner::tokenize`'s handling of it).
        if payload[pos] == 0xFF && pos + 2 < payload.len() && payload[pos + 1] == 0 && payload[pos + 2] == 0 {
            lines.push("#SEGMENT".to_string());
            pos += 3;
            continue;
        }
        if pos + 2 >= payload.len() {
            bail!("malformed payload: truncated line header at offset {pos}");
        }
        let line_no = ((payload[pos] as u16) << 8) | payload[pos + 1] as u16;
        let length = payload[pos + 2] as usize;
        pos += 3;
        if length == 0 {
            bail!("malformed payload: zero length byte for line {line_no} at offset {}", pos - 1);
        }
        let content_end = pos + length - 1; // index of the trailing 0x0D
        if content_end >= payload.len() {
            bail!(
                "malformed payload: line {line_no} at offset {} declares length {length} but only {} bytes remain",
                pos - 3,
                payload.len() - pos
            );
        }

        let mut s = format!("{line_no} ");
        while pos < content_end {
            let b1 = payload[pos];
            if reg.is_two_byte_token_high_byte(b1) {
                let b2 = payload[pos + 1];
                let code = ((b1 as u16) << 8) | b2 as u16;
                match reg.lookup_code(code) {
                    Some(_) if code == REM_CODE => {
                        pos += 2;
                        s.push_str("REM");
                        while pos < content_end {
                            s.push(cp437::to_char(payload[pos]));
                            pos += 1;
                        }
                        break;
                    }
                    Some(kw) => {
                        s.push_str(kw.name);
                        s.push(' ');
                        pos += 2;
                    }
                    None => {
                        // Unknown 2-byte code: emit the lead byte as a literal, advance 1.
                        s.push(cp437::to_char(b1));
                        pos += 1;
                    }
                }
            } else if b1 == 0x1F && pos + 3 <= content_end && payload[pos + 3] == 0x00 {
                // 0x1F [lineNum hi] [lineNum lo] 0x00: the compact binary line-number
                // target the PC-1600 patches in for a constant GOTO/GOSUB/THEN target
                // (confirmed against real PC-1600 memory dumps; the PC-1500 always
                // uses plain ASCII digits instead).
                let target = ((payload[pos + 1] as u16) << 8) | payload[pos + 2] as u16;
                s.push_str(&target.to_string());
                pos += 4;
            } else if b1 == 0x22 {
                s.push('"');
                pos += 1;
                while pos < content_end && payload[pos] != 0x22 {
                    s.push(cp437::to_char(payload[pos]));
                    pos += 1;
                }
                if pos < content_end && payload[pos] == 0x22 {
                    s.push('"');
                    pos += 1;
                }
            } else {
                s.push(cp437::to_char(b1));
                pos += 1;
            }
        }

        pos = content_end + 1; // skip the 0x0D
        lines.push(strip_one_trailing_space(s));
    }

    Ok(lines)
}

fn strip_one_trailing_space(mut s: String) -> String {
    if s.ends_with(' ') {
        s.pop();
    }
    s
}

/// Line terminator for a de-tokenized listing written back out as a host text file.
///
/// The PC-1500 itself terminates each program line with a bare `CR` (`0x0D`), but that
/// is rarely wanted in a file on disk, so the default ([`LineEnding::Platform`]) follows
/// the host: `\r\n` on Windows, `\n` elsewhere. Tokenizing always accepts `CR` *and*
/// `CRLF` input regardless of platform, so a listing written with any variant round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// `\n` (LF) — Unix, macOS.
    Lf,
    /// `\r\n` (CRLF) — Windows / DOS.
    CrLf,
    /// `\r` (CR) — the PC-1500's own line terminator.
    Cr,
    /// Follow the host platform: `\r\n` on Windows, `\n` elsewhere. The default.
    #[default]
    Platform,
}

impl LineEnding {
    /// The terminator string this variant emits, resolving [`LineEnding::Platform`] for
    /// the current build target.
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
            LineEnding::Platform => {
                if cfg!(windows) {
                    "\r\n"
                } else {
                    "\n"
                }
            }
        }
    }
}

/// Convenience: de-tokenize to a single UTF-8 string, each line terminated by `eol`
/// (a trailing terminator included). Pass [`LineEnding::Platform`] for the host default.
pub fn detokenize_to_text(payload: &[u8], reg: &Registry, eol: LineEnding) -> Result<String> {
    let lines = detokenize(payload, reg)?;
    let sep = eol.as_str();
    let mut text = lines.join(sep);
    text.push_str(sep);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    #[test]
    fn simple_line() {
        let payload = [0x00u8, 0x0A, 0x0A, 0x22, 0x41, 0x22, 0x3A, 0xF1, 0x87, 0x3A, 0xF1, 0xB3, 0x0D];
        let lines = detokenize(&payload, registry::pc1500()).unwrap();
        assert_eq!(lines, vec!["10 \"A\":CLEAR :WAIT"]);
    }

    #[test]
    fn empty_body_line() {
        let lines = detokenize(&[0x00, 0x0A, 0x01, 0x0D], registry::pc1500()).unwrap();
        assert_eq!(lines, vec!["10"]);
    }

    #[test]
    fn stops_at_program_terminator() {
        let payload = [0x00, 0x0A, 0x01, 0x0D, 0x00, 0x00, 0x99, 0x99];
        let lines = detokenize(&payload, registry::pc1500()).unwrap();
        assert_eq!(lines, vec!["10"]);
    }

    #[test]
    fn bad_length_is_error() {
        assert!(detokenize(&[0x00, 0x0A, 0x40, 0x0D], registry::pc1500()).is_err());
    }

    #[test]
    fn line_ending_override() {
        let payload = [0x00u8, 0x0A, 0x01, 0x0D, 0x00, 0x14, 0x01, 0x0D];
        let reg = registry::pc1500();
        assert_eq!(detokenize_to_text(&payload, reg, LineEnding::Lf).unwrap(), "10\n20\n");
        assert_eq!(detokenize_to_text(&payload, reg, LineEnding::CrLf).unwrap(), "10\r\n20\r\n");
        assert_eq!(detokenize_to_text(&payload, reg, LineEnding::Cr).unwrap(), "10\r20\r");
    }

    #[test]
    fn platform_line_ending_matches_target() {
        let want = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(LineEnding::Platform.as_str(), want);
    }
}
