//! Reserve Area (key-assignment layers) codec: PC-1500/1500A CE-158 payload type `'A'`.
//!
//! Binary payload is a fixed 188 bytes (the bytes after the CE-158 header):
//! ```text
//!   0..26    layer 1 label (CP437, NUL-padded)
//!   26..52   layer 2 label
//!   52..78   layer 3 label
//!   78..188  key-content pool (110 bytes)
//! ```
//! The pool is a sequence of `[key code][content bytes...]` entries terminated by a
//! `0x00` byte. Each of the three layers has six keys; key codes are not contiguous
//! across layers:
//! ```text
//!   layer 1, keys 1-6: 0x01 0x02 0x03 0x04 0x05 0x06
//!   layer 2, keys 1-6: 0x11 0x12 0x13 0x14 0x15 0x16
//!   layer 3, keys 1-6: 0x09 0x0A 0x0B 0x0C 0x0D 0x0E
//! ```
//! A key with no assigned content simply has no entry in the pool. Content bytes are a
//! mix of literal CP437 characters and 2-byte BASIC keyword tokens (no line framing, no
//! REM/string/space handling the way a BASIC program's tokenized bytes do).
//!
//! ASCII form ("SDAR"):
//! ```text
//! ; sde-reserve:1.0 pc1500
//! ; Filename: MYAPP
//!
//! [layer 1]
//! label: <text>
//! key 1: <text>
//! ...
//! key 6: <text>
//!
//! [layer 2]
//! ...
//! [layer 3]
//! ...
//! ```

use anyhow::{bail, Result};

use crate::cp437;
use crate::registry::Registry;

pub(crate) const MARKER: &str = "; sde-reserve:";

const LABEL_SIZE: usize = 26;
const NUM_LAYERS: usize = 3;
const NUM_KEYS: usize = 6;
const POOL_SIZE: usize = 110;
const PAYLOAD_SIZE: usize = NUM_LAYERS * LABEL_SIZE + POOL_SIZE;

const LAYER_KEY_CODES: [[u8; NUM_KEYS]; NUM_LAYERS] = [
    [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
    [0x11, 0x12, 0x13, 0x14, 0x15, 0x16],
    [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E],
];

fn key_index(code: u8) -> Option<(usize, usize)> {
    for (layer, codes) in LAYER_KEY_CODES.iter().enumerate() {
        if let Some(key) = codes.iter().position(|&c| c == code) {
            return Some((layer, key));
        }
    }
    None
}

/// A fully decoded Reserve Area layout: three layers, each with a label and six keys.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ReserveLayout {
    pub filename: Option<String>,
    pub labels: [String; NUM_LAYERS],
    pub keys: [[String; NUM_KEYS]; NUM_LAYERS],
}

/// Decode a 188-byte Reserve Area payload (CE-158 header already stripped).
pub fn decode_payload(payload: &[u8], reg: &Registry) -> Result<ReserveLayout> {
    if payload.len() != PAYLOAD_SIZE {
        bail!(
            "Reserve Area payload must be exactly {PAYLOAD_SIZE} bytes, got {}",
            payload.len()
        );
    }

    let mut layout = ReserveLayout::default();
    for layer in 0..NUM_LAYERS {
        let start = layer * LABEL_SIZE;
        layout.labels[layer] = cp437::decode(&payload[start..start + LABEL_SIZE])
            .trim_end_matches('\0')
            .to_string();
    }

    let pool = &payload[NUM_LAYERS * LABEL_SIZE..];
    let mut pos = 0usize;
    let mut terminated = false;
    while pos < pool.len() {
        let code = pool[pos];
        if code == 0x00 {
            terminated = true;
            break;
        }
        let Some((layer, key)) = key_index(code) else {
            bail!(
                "Reserve Area pool: unrecognized key code 0x{code:02X} at payload offset {}",
                NUM_LAYERS * LABEL_SIZE + pos
            );
        };
        pos += 1;

        let content_start = pos;
        while pos < pool.len() && pool[pos] != 0x00 && key_index(pool[pos]).is_none() {
            pos += 1;
        }
        layout.keys[layer][key] = detokenize_content(&pool[content_start..pos], reg);
    }
    if !terminated {
        bail!("Reserve Area pool: missing terminator within {POOL_SIZE}-byte pool region");
    }

    Ok(layout)
}

fn detokenize_content(bytes: &[u8], reg: &Registry) -> String {
    let mut s = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b1 = bytes[i];
        if reg.is_two_byte_token_high_byte(b1) && i + 1 < bytes.len() {
            let code = u16::from_be_bytes([b1, bytes[i + 1]]);
            if let Some(kw) = reg.lookup_code(code) {
                s.push_str(kw.name);
                i += 2;
                continue;
            }
        }
        s.push(cp437::to_char(b1));
        i += 1;
    }
    s
}

/// Encode a layout back to the fixed 188-byte binary payload. Errors if the encoded
/// key-content pool would exceed the 110-byte hardware limit.
pub fn encode_payload(layout: &ReserveLayout, reg: &Registry) -> Result<[u8; PAYLOAD_SIZE]> {
    let mut out = [0u8; PAYLOAD_SIZE];

    for layer in 0..NUM_LAYERS {
        let mut label = cp437::encode_lossy(&layout.labels[layer]);
        label.truncate(LABEL_SIZE);
        let start = layer * LABEL_SIZE;
        out[start..start + label.len()].copy_from_slice(&label);
    }

    let mut pool: Vec<u8> = Vec::with_capacity(POOL_SIZE);
    for (codes, keys) in LAYER_KEY_CODES.iter().zip(layout.keys.iter()) {
        for (&code, content) in codes.iter().zip(keys.iter()) {
            if content.is_empty() {
                continue;
            }
            let tokenized = tokenize_content(content, reg);
            let needed = pool.len() + 1 + tokenized.len() + 1; // + this entry's code byte + terminator
            if needed > POOL_SIZE {
                bail!(
                    "Reserve Area key content overflows the {POOL_SIZE}-byte pool \
                     (need {needed}, have {POOL_SIZE}); shorten a key's content"
                );
            }
            pool.push(code);
            pool.extend_from_slice(&tokenized);
        }
    }
    pool.push(0x00);

    let pool_start = NUM_LAYERS * LABEL_SIZE;
    out[pool_start..pool_start + pool.len()].copy_from_slice(&pool);
    Ok(out)
}

fn tokenize_content(content: &str, reg: &Registry) -> Vec<u8> {
    let bytes = cp437::encode_lossy(content);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_uppercase() || b == b'$' {
            match reg.longest_keyword_prefix(&bytes[i..]) {
                Some(kw) => {
                    out.extend_from_slice(&kw.code.to_be_bytes());
                    i += kw.name.len();
                    continue;
                }
                None => {
                    out.push(b);
                    i += 1;
                }
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// Render a layout as SDAR ASCII text.
pub fn to_ascii(layout: &ReserveLayout) -> String {
    let mut s = String::new();
    s.push_str(MARKER);
    s.push_str("1.0 pc1500\n");
    if let Some(name) = &layout.filename {
        if !name.is_empty() {
            s.push_str("; Filename: ");
            s.push_str(name);
            s.push('\n');
        }
    }
    for layer in 0..NUM_LAYERS {
        s.push('\n');
        s.push_str(&format!("[layer {}]\n", layer + 1));
        s.push_str(&format!("label: {}\n", layout.labels[layer]));
        for key in 0..NUM_KEYS {
            s.push_str(&format!("key {}: {}\n", key + 1, layout.keys[layer][key]));
        }
    }
    s
}

/// Parse SDAR ASCII text back into a layout. Tolerates blank lines and any other
/// `;`-prefixed comment line.
pub fn from_ascii(text: &str) -> Result<ReserveLayout> {
    let mut layout = ReserveLayout::default();
    let mut current_layer: Option<usize> = None;

    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("; Filename:") {
            let name = rest.trim();
            if !name.is_empty() {
                layout.filename = Some(name.to_string());
            }
            continue;
        }
        if trimmed.starts_with(';') {
            continue; // format marker line or other comment
        }
        if let Some(rest) = trimmed
            .strip_prefix("[layer ")
            .and_then(|r| r.strip_suffix(']'))
        {
            let n: usize = rest
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Reserve Area ASCII: bad layer header: {trimmed}"))?;
            if n == 0 || n > NUM_LAYERS {
                bail!("Reserve Area ASCII: layer number out of range: {trimmed}");
            }
            current_layer = Some(n - 1);
            continue;
        }
        let Some(layer) = current_layer else {
            bail!("Reserve Area ASCII: line outside any [layer N] section: {trimmed}");
        };
        if let Some(rest) = trimmed.strip_prefix("label:") {
            layout.labels[layer] = rest.strip_prefix(' ').unwrap_or(rest).to_string();
        } else if let Some(rest) = trimmed.strip_prefix("key ") {
            let (num_str, content) = rest.split_once(':').ok_or_else(|| {
                anyhow::anyhow!("Reserve Area ASCII: malformed key line: {trimmed}")
            })?;
            let key: usize = num_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Reserve Area ASCII: bad key number: {trimmed}"))?;
            if key == 0 || key > NUM_KEYS {
                bail!("Reserve Area ASCII: key number out of range: {trimmed}");
            }
            layout.keys[layer][key - 1] = content.strip_prefix(' ').unwrap_or(content).to_string();
        } else {
            bail!("Reserve Area ASCII: unrecognized line: {trimmed}");
        }
    }

    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    fn reg() -> &'static Registry {
        Registry::for_device(registry::Device::Pc1500)
    }

    #[test]
    fn empty_layout_round_trips() {
        let layout = ReserveLayout::default();
        let payload = encode_payload(&layout, reg()).unwrap();
        assert_eq!(payload.len(), PAYLOAD_SIZE);
        assert_eq!(payload[NUM_LAYERS * LABEL_SIZE], 0x00);
        let decoded = decode_payload(&payload, reg()).unwrap();
        assert_eq!(decoded.labels, layout.labels);
        assert_eq!(decoded.keys, layout.keys);
    }

    #[test]
    fn tokenize_keyword_then_literal() {
        let out = tokenize_content("ABS(", reg());
        let abs_code = reg().lookup("ABS").unwrap().code;
        assert_eq!(&out[..2], &abs_code.to_be_bytes());
        assert_eq!(out.last(), Some(&b'('));
    }

    #[test]
    fn tokenize_digits_stay_literal() {
        assert_eq!(tokenize_content("123", reg()), vec![b'1', b'2', b'3']);
    }

    #[test]
    fn pool_overflow_errors() {
        let mut layout = ReserveLayout::default();
        for key in 0..NUM_KEYS {
            layout.keys[0][key] = "ABCDEFGHIJKLMNOPQRST".to_string(); // 20 literal chars
        }
        assert!(encode_payload(&layout, reg()).is_err());
    }

    #[test]
    fn decode_rejects_unknown_key_code() {
        let mut payload = [0u8; PAYLOAD_SIZE];
        payload[NUM_LAYERS * LABEL_SIZE] = 0x7F; // not a valid key code
        assert!(decode_payload(&payload, reg()).is_err());
    }

    #[test]
    fn decode_rejects_wrong_size() {
        assert!(decode_payload(&[0u8; 100], reg()).is_err());
    }

    #[test]
    fn ascii_round_trip() {
        let mut layout = ReserveLayout {
            filename: Some("MYAPP".to_string()),
            ..Default::default()
        };
        layout.labels[0] = "MENU".to_string();
        layout.keys[0][0] = "PRINT".to_string();
        layout.keys[1][3] = "ABC".to_string();
        let text = to_ascii(&layout);
        let parsed = from_ascii(&text).unwrap();
        assert_eq!(parsed, layout);
    }

    #[test]
    fn to_ascii_omits_filename_line_when_none() {
        let layout = ReserveLayout::default();
        assert!(!to_ascii(&layout).contains("Filename"));
    }

    #[test]
    fn to_ascii_includes_filename_when_present() {
        let layout = ReserveLayout {
            filename: Some("MYAPP".to_string()),
            ..Default::default()
        };
        assert!(to_ascii(&layout).contains("; Filename: MYAPP"));
    }
}
