//! CE-158 / PC-1600 serial file headers that wrap a tokenized BASIC or machine-language
//! payload.

use crate::cp437;
use crate::registry::Device;

const CE158_LEN: usize = 27;
const PC1600_LEN: usize = 16;

/// The payload type recorded in a header. `Reserve` and `Variables` are PC-1500/1500A
/// (CE-158) only — the PC-1600 header format has no equivalent type byte for either
/// (whether the PC-1600 protocol has one at all is *unresearched*, not confirmed
/// absent; [`pc1600_file_type`] simply doesn't model one). Like `Basic`, neither
/// carries a meaningful start/run address — those fields are `Machine`-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Basic,
    Machine,
    Reserve,
    Variables,
}

/// A recognized header found in a byte buffer, fully parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedHeader {
    pub device: Device,
    pub file_type: FileType,
    /// Offset of the header's first byte within the input buffer.
    pub offset: usize,
    /// Header length in bytes (`offset + header_len` is where the payload starts).
    pub header_len: usize,
    /// Payload length in bytes, already corrected for CE-158's "capacity - 1" encoding.
    /// Not meaningful for `FileType::Variables`: the wire always encodes it as `0`
    /// regardless of the true payload size, so this decodes to `1` for that type —
    /// callers must parse a Variables payload to end-of-buffer instead of trusting it.
    pub length: usize,
    /// Load start address. Only meaningful when `file_type == Machine`, else 0.
    pub start_addr: u32,
    /// Auto-run address. Only meaningful when `file_type == Machine`, else 0.
    pub run_addr: u32,
    /// CE-158 filename field (CP437, trimmed), `None` if blank or not a CE-158 header
    /// (the PC-1600 header has no filename field at all).
    pub filename: Option<String>,
}

impl ParsedHeader {
    pub fn payload_start(&self) -> usize {
        self.offset + self.header_len
    }
}

/// Locate and fully parse a CE-158 or PC-1600 header, tolerating leading capture noise
/// (e.g. stray `0x00` bytes) before the magic. Returns the first match whose type byte
/// is recognized; a header with an unrecognized type byte is treated as not found and
/// scanning does not continue past it — it stops at the first magic match rather than
/// searching for a second, later one.
pub fn find(data: &[u8]) -> Option<ParsedHeader> {
    for i in 0..data.len() {
        // CE-158: 0x01, <type>, "COM"
        if data[i] == 0x01 && data.get(i + 2..i + 5) == Some(b"COM") {
            return parse_ce158(data, i);
        }
        // PC-1600: FF 10 00 00
        if data.get(i..i + 4) == Some(&[0xFF, 0x10, 0x00, 0x00][..]) {
            return parse_pc1600(data, i);
        }
    }
    None
}

fn ce158_file_type(type_char: u8) -> Option<FileType> {
    match type_char {
        0x40 => Some(FileType::Basic),     // '@'
        0x42 => Some(FileType::Machine),   // 'B'
        0x41 => Some(FileType::Reserve),   // 'A'
        0x48 => Some(FileType::Variables), // 'H'
        _ => None,
    }
}

fn pc1600_file_type(type_byte: u8) -> Option<FileType> {
    match type_byte {
        0x21 => Some(FileType::Basic),
        0x10 => Some(FileType::Machine),
        // Deliberately no Reserve/Variables mapping: the PC-1600 has no equivalent
        // header type (or none is known — see FileType's doc comment), not an
        // oversight. No other PC-1600 type byte is modeled either — unresearched.
        _ => None,
    }
}

fn parse_ce158(data: &[u8], offset: usize) -> Option<ParsedHeader> {
    if data.len() < offset + CE158_LEN {
        return None;
    }
    let file_type = ce158_file_type(data[offset + 1])?;

    let name_bytes = &data[offset + 0x05..offset + 0x15];
    let decoded = cp437::decode(name_bytes);
    let trimmed = decoded.trim_end_matches(['\0', ' ']);
    let filename = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

    let start_addr_raw = be16(data, offset + 0x15) as u32;
    // Wire stores "capacity - 1" (CE-158 Technical Reference Manual §13).
    let length = be16(data, offset + 0x17) as usize + 1;
    let run_addr_raw = be16(data, offset + 0x19) as u32;

    let (start_addr, run_addr) = if file_type == FileType::Machine {
        (start_addr_raw, run_addr_raw)
    } else {
        (0, 0)
    };

    Some(ParsedHeader {
        device: Device::Pc1500,
        file_type,
        offset,
        header_len: CE158_LEN,
        length,
        start_addr,
        run_addr,
        filename,
    })
}

fn parse_pc1600(data: &[u8], offset: usize) -> Option<ParsedHeader> {
    if data.len() < offset + PC1600_LEN {
        return None;
    }
    let file_type = pc1600_file_type(data[offset + 0x04])?;

    let length = le24(data, offset + 0x05) as usize;
    let start_addr_raw = le24(data, offset + 0x08);
    let run_addr_raw = le24(data, offset + 0x0B);

    let (start_addr, run_addr) = if file_type == FileType::Machine {
        (start_addr_raw, run_addr_raw)
    } else {
        (0, 0)
    };

    Some(ParsedHeader {
        device: Device::Pc1600,
        file_type,
        offset,
        header_len: PC1600_LEN,
        length,
        start_addr,
        run_addr,
        filename: None,
    })
}

pub(crate) fn be16(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([data[at], data[at + 1]])
}

fn le24(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], 0])
}

/// Scan a growing buffer (as bytes arrive over serial) and report the total number of
/// bytes expected for a complete transfer (header + payload), once known.
///
/// Returns `None` when:
/// - no recognized header magic/type has been found yet (need more bytes, or the stream
///   never carries one, e.g. a plain ASCII listing);
/// - a header's magic was found but there aren't yet enough bytes to read it fully.
///
/// It stops at the first magic match rather than continuing to scan for a second one
/// once bytes are insufficient.
///
/// For `FileType::Variables`, the wire length field is unreliable (see `ParsedHeader`),
/// so this resolves to a total that only covers the header itself plus 1 byte — under
/// what a real transfer actually sends. A live receive of a Variables payload cannot
/// rely on this for framing and must fall back to idle-timeout-based completion, the
/// same way headerless content already does.
pub fn expected_total_bytes(data: &[u8]) -> Option<usize> {
    find(data).map(|h| h.payload_start() + h.length)
}

/// Arguments for building a serial header. `name` supplies the CE-158 filename
/// (upper-cased, CP437, truncated to 16 chars, NUL-padded); unused for PC-1600.
/// `start_addr`/`run_addr` are only written when `file_type == Machine`.
pub struct BuildHeader<'a> {
    pub device: Device,
    pub file_type: FileType,
    pub name: Option<&'a str>,
    pub payload_len: usize,
    pub start_addr: u32,
    pub run_addr: u32,
}

/// Build the serial header for `device` wrapping a tokenized BASIC payload of
/// `payload_len` bytes. Thin wrapper over [`build_header`] for the common BASIC case,
/// kept so existing call sites (and their tests) are unaffected by the richer API.
pub fn build(device: Device, name: Option<&str>, payload_len: usize) -> Vec<u8> {
    build_header(BuildHeader {
        device,
        file_type: FileType::Basic,
        name,
        payload_len,
        start_addr: 0,
        run_addr: 0,
    })
}

/// Build a serial header per `spec`. See [`BuildHeader`].
pub fn build_header(spec: BuildHeader) -> Vec<u8> {
    match spec.device {
        Device::Pc1500 => build_ce158(spec),
        Device::Pc1600 => build_pc1600(spec),
    }
}

fn build_ce158(spec: BuildHeader) -> Vec<u8> {
    let mut h = vec![0u8; CE158_LEN];
    h[0] = 0x01; // magic
    h[1] = match spec.file_type {
        FileType::Basic => 0x40,     // '@'
        FileType::Machine => 0x42,   // 'B'
        FileType::Reserve => 0x41,   // 'A'
        FileType::Variables => 0x48, // 'H'
    };
    h[2..5].copy_from_slice(b"COM");

    let name = spec.name.unwrap_or("");
    let upper: String = name.chars().take(16).collect::<String>().to_ascii_uppercase();
    let mut fname = cp437::encode_lossy(&upper);
    fname.truncate(16);
    h[5..5 + fname.len()].copy_from_slice(&fname);

    if spec.file_type == FileType::Machine {
        h[0x15..0x17].copy_from_slice(&(spec.start_addr as u16).to_be_bytes());
    }

    // 0x17..0x19 data length, big-endian, "capacity - 1". A Variables payload is
    // always sent with wire value 0 regardless of true size — the receiving device
    // doesn't use this field for that type (see `ParsedHeader::length`'s doc comment).
    let dl: u16 = if spec.file_type == FileType::Variables {
        0
    } else {
        spec.payload_len.wrapping_sub(1) as u16
    };
    h[0x17..0x19].copy_from_slice(&dl.to_be_bytes());

    if spec.file_type == FileType::Machine {
        h[0x19..0x1B].copy_from_slice(&(spec.run_addr as u16).to_be_bytes());
    }
    h
}

fn build_pc1600(spec: BuildHeader) -> Vec<u8> {
    let mut h = vec![0u8; PC1600_LEN];
    h[0..4].copy_from_slice(&[0xFF, 0x10, 0x00, 0x00]);
    h[4] = match spec.file_type {
        FileType::Basic => 0x21,
        FileType::Machine => 0x10,
        FileType::Reserve | FileType::Variables => {
            unreachable!("PC-1600 has no Reserve/Variables header type -- see FileType's doc comment")
        }
    };

    // 0x05..0x08 data length, little-endian 3 bytes, exact payload length
    h[5..8].copy_from_slice(&(spec.payload_len as u32).to_le_bytes()[..3]);

    if spec.file_type == FileType::Machine {
        h[8..11].copy_from_slice(&spec.start_addr.to_le_bytes()[..3]);
        h[11..14].copy_from_slice(&spec.run_addr.to_le_bytes()[..3]);
    }

    // 0x0E..0x10 end-of-header marker. Confirmed against a real PC-1600 capture
    // (ff 10 00 00 21 ... 00 0f), which ends with {0x00, 0x0F} here.
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
        assert_eq!(p.file_type, FileType::Basic);
        assert_eq!(p.length, 586);
        assert_eq!(p.start_addr, 0);
        assert_eq!(p.run_addr, 0);
        assert_eq!(p.filename.as_deref(), Some("DEPRECIATION"));
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
        assert_eq!(p.file_type, FileType::Basic);
        assert_eq!(p.length, 586);
        assert_eq!(p.filename, None);
        assert_eq!(p.payload_start(), 16);
    }

    #[test]
    fn ce158_machine_roundtrip() {
        let h = build_header(BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Machine,
            name: Some("prog"),
            payload_len: 100,
            start_addr: 0x38C5,
            run_addr: 0xFFFF,
        });
        let p = find(&h).unwrap();
        assert_eq!(p.file_type, FileType::Machine);
        assert_eq!(p.length, 100);
        assert_eq!(p.start_addr, 0x38C5);
        assert_eq!(p.run_addr, 0xFFFF);
        assert_eq!(p.filename.as_deref(), Some("PROG"));
    }

    #[test]
    fn pc1600_machine_roundtrip() {
        let h = build_header(BuildHeader {
            device: Device::Pc1600,
            file_type: FileType::Machine,
            name: None,
            payload_len: 4096,
            start_addr: 0x123456,
            run_addr: 0xABCDEF & 0xFFFFFF,
        });
        let p = find(&h).unwrap();
        assert_eq!(p.file_type, FileType::Machine);
        assert_eq!(p.length, 4096);
        assert_eq!(p.start_addr, 0x123456);
        assert_eq!(p.run_addr, 0xABCDEF);
    }

    #[test]
    fn find_tolerates_leading_noise() {
        let mut buf = vec![0x00, 0x00, 0x00];
        buf.extend(build(Device::Pc1500, Some("x"), 10));
        assert_eq!(find(&buf).unwrap().offset, 3);
    }

    #[test]
    fn ce158_reserve_roundtrip_shape() {
        let h = build_header(BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Reserve,
            name: Some("x"),
            payload_len: 188,
            start_addr: 0,
            run_addr: 0,
        });
        assert_eq!(h[1], b'A');
        let p = find(&h).unwrap();
        assert_eq!(p.file_type, FileType::Reserve);
        assert_eq!(p.length, 188);
        assert_eq!(p.start_addr, 0);
        assert_eq!(p.run_addr, 0);
    }

    #[test]
    fn ce158_variables_roundtrip_shape() {
        // The wire length field is always 0 for Variables, regardless of the true
        // payload length passed in -- the device doesn't use this field for this type.
        let h = build_header(BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Variables,
            name: Some("x"),
            payload_len: 999,
            start_addr: 0,
            run_addr: 0,
        });
        assert_eq!(h[1], b'H');
        assert_eq!(&h[0x17..0x19], &[0x00, 0x00]);
        let p = find(&h).unwrap();
        assert_eq!(p.file_type, FileType::Variables);
        assert_eq!(p.length, 1); // decoded from the always-0 wire value; not meaningful
    }

    #[test]
    fn expected_total_bytes_stages() {
        // No magic at all yet.
        assert_eq!(expected_total_bytes(b"not a header"), None);

        let full = build(Device::Pc1500, Some("x"), 10);
        // Magic present but header incomplete.
        assert_eq!(expected_total_bytes(&full[..10]), None);

        // Full header, no payload yet -- still resolvable (length is known).
        assert_eq!(expected_total_bytes(&full), Some(27 + 10));

        // Full header + full payload.
        let mut with_payload = full.clone();
        with_payload.extend(std::iter::repeat_n(0u8, 10));
        assert_eq!(expected_total_bytes(&with_payload), Some(27 + 10));
    }

    #[test]
    fn expected_total_bytes_undercounts_for_variables() {
        // The always-0 wire length field makes this resolve, but to a total that's
        // too small to be useful for framing a live receive -- documented behavior,
        // not a bug; see `expected_total_bytes`'s doc comment.
        let h = build_header(BuildHeader {
            device: Device::Pc1500,
            file_type: FileType::Variables,
            name: Some("x"),
            payload_len: 999,
            start_addr: 0,
            run_addr: 0,
        });
        assert_eq!(expected_total_bytes(&h), Some(CE158_LEN + 1));
    }
}
