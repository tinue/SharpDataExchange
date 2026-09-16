//! CE-158 / PC-1600 serial file headers that wrap a tokenized BASIC payload.
//! Ported from Java `header/Ce158Header`, `header/Pc1600Header` and
//! `SharpDataExchange.findHeaderOffset`.

use crate::cp437;
use crate::registry::Device;

const CE158_LEN: usize = 27;
const PC1600_LEN: usize = 16;

/// A recognized header found in a byte buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedHeader {
    pub device: Device,
    /// Offset of the header's first byte within the input buffer.
    pub offset: usize,
    /// Header length in bytes (`offset + header_len` is where the payload starts).
    pub header_len: usize,
}

impl ParsedHeader {
    pub fn payload_start(&self) -> usize {
        self.offset + self.header_len
    }
}

/// Locate and identify a CE-158 or PC-1600 header, tolerating leading capture noise
/// (e.g. stray `0x00` bytes) before the magic. Returns the first match.
pub fn find(data: &[u8]) -> Option<ParsedHeader> {
    for i in 0..data.len() {
        // CE-158: 0x01, <type>, "COM"
        if data[i] == 0x01 && data.get(i + 2..i + 5) == Some(b"COM") {
            return Some(ParsedHeader { device: Device::Pc1500, offset: i, header_len: CE158_LEN });
        }
        // PC-1600: FF 10 00 00
        if data.get(i..i + 4) == Some(&[0xFF, 0x10, 0x00, 0x00][..]) {
            return Some(ParsedHeader { device: Device::Pc1600, offset: i, header_len: PC1600_LEN });
        }
    }
    None
}

/// Build the serial header for `device` wrapping a tokenized BASIC payload of
/// `payload_len` bytes. `name` supplies the CE-158 filename (upper-cased, CP437,
/// truncated to 16 chars, NUL-padded); it is unused for PC-1600.
pub fn build(device: Device, name: Option<&str>, payload_len: usize) -> Vec<u8> {
    match device {
        Device::Pc1500 => build_ce158(name.unwrap_or(""), payload_len),
        Device::Pc1600 => build_pc1600(payload_len),
    }
}

fn build_ce158(name: &str, payload_len: usize) -> Vec<u8> {
    let mut h = vec![0u8; CE158_LEN];
    h[0] = 0x01; // magic
    h[1] = 0x40; // type '@' = tokenized BASIC
    h[2..5].copy_from_slice(b"COM");

    let upper: String = name.chars().take(16).collect::<String>().to_ascii_uppercase();
    let mut fname = cp437::encode_lossy(&upper);
    fname.truncate(16);
    h[5..5 + fname.len()].copy_from_slice(&fname);
    // 0x15..0x17 load address = 0 (already zero)

    // 0x17..0x19 data length, big-endian, "capacity - 1"
    let dl = payload_len.wrapping_sub(1) as u16;
    h[0x17] = (dl >> 8) as u8;
    h[0x18] = (dl & 0xFF) as u8;
    // 0x19..0x1B auto-run address = 0
    h
}

fn build_pc1600(payload_len: usize) -> Vec<u8> {
    let mut h = vec![0u8; PC1600_LEN];
    h[0..4].copy_from_slice(&[0xFF, 0x10, 0x00, 0x00]);
    h[4] = 0x21; // type = tokenized BASIC

    // 0x05..0x08 data length, little-endian 3 bytes, exact payload length
    let dl = payload_len as u32;
    h[5] = (dl & 0xFF) as u8;
    h[6] = ((dl >> 8) & 0xFF) as u8;
    h[7] = ((dl >> 16) & 0xFF) as u8;
    // 0x08..0x0B load address = 0, 0x0B..0x0E auto-run = 0

    // 0x0E..0x10 end-of-header marker. Confirmed against a real PC-1600 capture
    // (ff 10 00 00 21 ... 00 0f) and matches current Java `Pc1600Header.getHeader()`,
    // which appends {0x00, 0x0F} here.
    h[14] = 0x00;
    h[15] = 0x0F;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ce158_roundtrip_shape() {
        let h = build(Device::Pc1500, Some("depreciation"), 586);
        assert_eq!(h.len(), 27);
        assert_eq!(&h[0..5], &[0x01, 0x40, b'C', b'O', b'M']);
        assert_eq!(&h[5..17], b"DEPRECIATION"); // upper-cased
        assert_eq!(&h[0x17..0x19], &[0x02, 0x49]); // 585 BE
        let p = find(&h).unwrap();
        assert_eq!(p.device, Device::Pc1500);
        assert_eq!(p.payload_start(), 27);
    }

    #[test]
    fn pc1600_roundtrip_shape() {
        let h = build(Device::Pc1600, None, 586);
        assert_eq!(h.len(), 16);
        assert_eq!(&h[0..5], &[0xFF, 0x10, 0x00, 0x00, 0x21]);
        assert_eq!(&h[5..8], &[0x4A, 0x02, 0x00]); // 586 LE
        assert_eq!(&h[14..16], &[0x00, 0x0F]);
        let p = find(&h).unwrap();
        assert_eq!(p.device, Device::Pc1600);
        assert_eq!(p.payload_start(), 16);
    }

    #[test]
    fn find_tolerates_leading_noise() {
        let mut buf = vec![0x00, 0x00, 0x00];
        buf.extend(build(Device::Pc1500, Some("x"), 10));
        assert_eq!(find(&buf).unwrap().offset, 3);
    }
}
