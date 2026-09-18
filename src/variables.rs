//! Variables codec: PC-1500/1500A CE-158 payload type `'H'`.
//!
//! Binary payload (the bytes after the CE-158 header — its length field is not
//! meaningful for this type, see `header::ParsedHeader::length`) is a sequence of
//! records with no fixed total length and no terminator; parsing stops when the buffer
//! is exhausted. Each record:
//! ```text
//!   0x00 separator
//!   4-byte prefix: [record_len - 1, dim_max, 0x00, type]
//!   data bytes (record_len - 4 of them)
//! ```
//! `type == 0x88` marks numeric data (8-byte BCD elements, see [`crate::bcd`]);
//! otherwise `type` is the string element length in bytes. `dim_max == 0` marks a
//! scalar; `dim_max > 0` marks an array of `dim_max + 1` elements (i.e. it was
//! `DIM(dim_max)`/`DIM$(dim_max)` on the device).
//!
//! ASCII form ("SDAV"):
//! ```text
//! ; sde-variables:1.0 pc1500
//! ; Count: <N>
//! ; Filename: MYAPP
//! <value-line>
//! ...
//! ```
//! `N` is the number of top-level records (one per scalar or whole array). A numeric
//! scalar is a plain decimal line; a string scalar is a double-quoted, escaped line; a
//! numeric array is a `DIM (<dimMax>)` header followed by `dimMax + 1` decimal lines; a
//! string array is a `DIM $(<dimMax>)*<maxLen>` header followed by `dimMax + 1` quoted
//! lines.

use anyhow::{bail, Result};

use crate::bcd;

pub(crate) const MARKER: &str = "; sde-variables:";

const NUMERIC_TYPE: u8 = 0x88;
const NUMERIC_ELEM_LEN: usize = 8;
const DEFAULT_STRING_SCALAR_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarValue {
    NumericScalar(String),
    StringScalar(Vec<u8>),
    NumericArray(Vec<String>),
    StringArray {
        max_len: usize,
        elements: Vec<Vec<u8>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VarFile {
    pub filename: Option<String>,
    pub values: Vec<VarValue>,
}

/// Decode a Variables payload (bytes after the CE-158 header, to end of buffer).
pub fn decode_payload(payload: &[u8]) -> Result<VarFile> {
    let mut values = Vec::new();
    let mut pos = 0usize;

    while pos < payload.len() {
        if payload[pos] != 0x00 {
            bail!("Variables payload: expected record separator at offset {pos}");
        }
        if pos + 5 > payload.len() {
            bail!("Variables payload: truncated record prefix at offset {pos}");
        }
        let prefix = &payload[pos + 1..pos + 5];
        let record_len = prefix[0] as usize + 1;
        let dim_max = prefix[1];
        let type_byte = prefix[3];
        if record_len < 4 {
            bail!("Variables payload: record at offset {pos} declares an impossible length {record_len}");
        }
        let data_len = record_len - 4;
        let data_start = pos + 5;
        if data_start + data_len > payload.len() {
            bail!(
                "Variables payload: record at offset {pos} declares {record_len} bytes but only {} remain",
                payload.len() - data_start
            );
        }
        let data = &payload[data_start..data_start + data_len];

        let is_numeric = type_byte == NUMERIC_TYPE;
        let value = if dim_max == 0 {
            if is_numeric {
                if data_len != NUMERIC_ELEM_LEN {
                    bail!(
                        "Variables payload: numeric scalar at offset {pos} has data length {data_len}, expected {NUMERIC_ELEM_LEN}"
                    );
                }
                VarValue::NumericScalar(bcd::decode(data)?)
            } else {
                if data_len != type_byte as usize {
                    bail!(
                        "Variables payload: string scalar at offset {pos} has data length {data_len}, expected {type_byte}"
                    );
                }
                VarValue::StringScalar(trim_trailing_nulls(data))
            }
        } else {
            let count = dim_max as usize + 1;
            if is_numeric {
                if data_len != count * NUMERIC_ELEM_LEN {
                    bail!(
                        "Variables payload: numeric array at offset {pos} has data length {data_len}, expected {}",
                        count * NUMERIC_ELEM_LEN
                    );
                }
                let elements = data
                    .as_chunks::<NUMERIC_ELEM_LEN>()
                    .0
                    .iter()
                    .map(|c| bcd::decode(c))
                    .collect::<Result<Vec<_>>>()?;
                VarValue::NumericArray(elements)
            } else {
                let max_len = type_byte as usize;
                if data_len != count * max_len {
                    bail!(
                        "Variables payload: string array at offset {pos} has data length {data_len}, expected {}",
                        count * max_len
                    );
                }
                let elements = data
                    .chunks_exact(max_len)
                    .map(trim_trailing_nulls)
                    .collect();
                VarValue::StringArray { max_len, elements }
            }
        };
        values.push(value);
        pos = data_start + data_len;
    }

    Ok(VarFile {
        filename: None,
        values,
    })
}

fn trim_trailing_nulls(data: &[u8]) -> Vec<u8> {
    let mut len = data.len();
    while len > 0 && data[len - 1] == 0x00 {
        len -= 1;
    }
    data[..len].to_vec()
}

/// Encode values back to the binary payload (concatenated records, each preceded by
/// its `0x00` separator).
pub fn encode_payload(values: &[VarValue]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for value in values {
        out.push(0x00);
        match value {
            VarValue::NumericScalar(text) => {
                out.extend_from_slice(&[0x0B, 0x00, 0x00, NUMERIC_TYPE]);
                out.extend_from_slice(&bcd::encode(text)?);
            }
            VarValue::StringScalar(bytes) => {
                out.extend_from_slice(&[0x13, 0x00, 0x00, DEFAULT_STRING_SCALAR_LEN as u8]);
                out.extend_from_slice(&pad_slot(bytes, DEFAULT_STRING_SCALAR_LEN));
            }
            VarValue::NumericArray(elements) => {
                let dim_max = array_dim_max(elements.len())?;
                let data_len = elements.len() * NUMERIC_ELEM_LEN;
                let record_len = record_len_byte(data_len)?;
                out.extend_from_slice(&[record_len, dim_max, 0x00, NUMERIC_TYPE]);
                for e in elements {
                    out.extend_from_slice(&bcd::encode(e)?);
                }
            }
            VarValue::StringArray { max_len, elements } => {
                let dim_max = array_dim_max(elements.len())?;
                let max_len_byte: u8 = (*max_len).try_into().map_err(|_| {
                    anyhow::anyhow!("Variables string array element length {max_len} exceeds 255")
                })?;
                let data_len = elements.len() * max_len;
                let record_len = record_len_byte(data_len)?;
                out.extend_from_slice(&[record_len, dim_max, 0x00, max_len_byte]);
                for e in elements {
                    out.extend_from_slice(&pad_slot(e, *max_len));
                }
            }
        }
    }
    Ok(out)
}

fn array_dim_max(len: usize) -> Result<u8> {
    if len == 0 {
        bail!("Variables array must have at least one element");
    }
    (len - 1).try_into().map_err(|_| {
        anyhow::anyhow!(
            "array too large to encode: {len} elements exceeds the format's 256-element limit"
        )
    })
}

fn record_len_byte(data_len: usize) -> Result<u8> {
    (data_len + 4).checked_sub(1).and_then(|v| u8::try_from(v).ok()).ok_or_else(|| {
        anyhow::anyhow!("array too large to encode: {data_len} data bytes exceeds the format's 8-bit record-length limit")
    })
}

fn pad_slot(content: &[u8], len: usize) -> Vec<u8> {
    let mut slot = vec![0u8; len];
    let n = content.len().min(len);
    slot[..n].copy_from_slice(&content[..n]);
    slot
}

/// Render a `VarFile` as SDAV ASCII text.
pub fn to_ascii(file: &VarFile) -> String {
    let mut s = String::new();
    s.push_str(MARKER);
    s.push_str("1.0 pc1500\n");
    s.push_str(&format!("; Count: {}\n", file.values.len()));
    if let Some(name) = &file.filename {
        if !name.is_empty() {
            s.push_str("; Filename: ");
            s.push_str(name);
            s.push('\n');
        }
    }
    for value in &file.values {
        match value {
            VarValue::NumericScalar(text) => {
                s.push_str(text);
                s.push('\n');
            }
            VarValue::StringScalar(bytes) => {
                s.push('"');
                s.push_str(&escape_string(bytes));
                s.push_str("\"\n");
            }
            VarValue::NumericArray(elements) => {
                s.push_str(&format!("DIM ({})\n", elements.len() - 1));
                for e in elements {
                    s.push_str(e);
                    s.push('\n');
                }
            }
            VarValue::StringArray { max_len, elements } => {
                s.push_str(&format!("DIM $({})*{}\n", elements.len() - 1, max_len));
                for e in elements {
                    s.push('"');
                    s.push_str(&escape_string(e));
                    s.push_str("\"\n");
                }
            }
        }
    }
    s
}

fn escape_string(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            b'\\' => s.push_str("\\\\"),
            b'"' => s.push_str("\\\""),
            0x20..=0x7E => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02X}")),
        }
    }
    s
}

fn unescape_string(text: &str) -> Result<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'x' if i + 3 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[i + 2..i + 4])
                        .map_err(|_| anyhow::anyhow!("Variables ASCII: bad \\x escape"))?;
                    let v = u8::from_str_radix(hex, 16)
                        .map_err(|_| anyhow::anyhow!("Variables ASCII: bad \\x escape: {hex}"))?;
                    out.push(v);
                    i += 4;
                }
                _ => bail!("Variables ASCII: unknown escape sequence at byte {i}"),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// Parse SDAV ASCII text back into a `VarFile`. Errors if the parsed record count
/// disagrees with a declared `; Count:` line.
pub fn from_ascii(text: &str) -> Result<VarFile> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut declared_count: Option<usize> = None;
    let mut filename = None;
    let mut values = Vec::new();

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("; Count:") {
            declared_count = Some(
                rest.trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Variables ASCII: bad Count line: {line}"))?,
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix("; Filename:") {
            let name = rest.trim();
            if !name.is_empty() {
                filename = Some(name.to_string());
            }
            continue;
        }
        if line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("DIM $(") {
            let (dim_str, after) = rest.split_once(')').ok_or_else(|| {
                anyhow::anyhow!("Variables ASCII: malformed string array header: {line}")
            })?;
            let max_len_str = after.strip_prefix('*').ok_or_else(|| {
                anyhow::anyhow!("Variables ASCII: malformed string array header: {line}")
            })?;
            let dim_max: usize = dim_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Variables ASCII: bad dimension in: {line}"))?;
            let max_len: usize = max_len_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Variables ASCII: bad element length in: {line}"))?;
            let mut elements = Vec::with_capacity(dim_max + 1);
            for _ in 0..=dim_max {
                let (elem_line, next_i) = next_non_blank(&lines, i)?;
                i = next_i;
                let content = quoted_content(elem_line)?;
                elements.push(unescape_string(content)?);
            }
            values.push(VarValue::StringArray { max_len, elements });
        } else if let Some(rest) = line.strip_prefix("DIM (") {
            let dim_str = rest.strip_suffix(')').ok_or_else(|| {
                anyhow::anyhow!("Variables ASCII: malformed numeric array header: {line}")
            })?;
            let dim_max: usize = dim_str
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Variables ASCII: bad dimension in: {line}"))?;
            let mut elements = Vec::with_capacity(dim_max + 1);
            for _ in 0..=dim_max {
                let (elem_line, next_i) = next_non_blank(&lines, i)?;
                i = next_i;
                elements.push(elem_line.trim().to_string());
            }
            values.push(VarValue::NumericArray(elements));
        } else if line.starts_with('"') {
            let content = quoted_content(line)?;
            values.push(VarValue::StringScalar(unescape_string(content)?));
        } else {
            values.push(VarValue::NumericScalar(line.to_string()));
        }
    }

    if let Some(declared) = declared_count {
        if declared != values.len() {
            bail!(
                "Variables ASCII: declared Count {declared} does not match {} parsed value(s)",
                values.len()
            );
        }
    }

    Ok(VarFile { filename, values })
}

fn next_non_blank<'a>(lines: &[&'a str], mut i: usize) -> Result<(&'a str, usize)> {
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;
        if !line.is_empty() {
            return Ok((line, i));
        }
    }
    bail!("Variables ASCII: unexpected end of input while reading an array")
}

fn quoted_content(line: &str) -> Result<&str> {
    let inner = line
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| anyhow::anyhow!("Variables ASCII: unterminated quoted string: {line}"))?;
    Ok(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_scalar_round_trips() {
        let values = vec![VarValue::NumericScalar("1500".to_string())];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
    }

    #[test]
    fn string_scalar_round_trips() {
        let values = vec![VarValue::StringScalar(b"Hi there!".to_vec())];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
    }

    #[test]
    fn mixed_scalars_round_trip() {
        let values = vec![
            VarValue::NumericScalar("3.14159265".to_string()),
            VarValue::StringScalar(b"ABC".to_vec()),
            VarValue::NumericScalar("0".to_string()),
        ];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
        assert_eq!(
            to_ascii(&VarFile {
                filename: None,
                values: decoded.values
            })
            .lines()
            .nth(1)
            .unwrap(),
            "; Count: 3"
        );
    }

    #[test]
    fn numeric_array_round_trips() {
        let values = vec![VarValue::NumericArray(vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
        ])];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
    }

    #[test]
    fn string_arrays_with_different_widths_round_trip() {
        let values = vec![
            VarValue::StringArray {
                max_len: 80,
                elements: vec![b"a".to_vec(), b"b".to_vec()],
            },
            VarValue::StringArray {
                max_len: 40,
                elements: vec![b"x".to_vec(), b"y".to_vec(), b"z".to_vec()],
            },
        ];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
    }

    #[test]
    fn mixed_arrays_round_trip() {
        let values = vec![
            VarValue::NumericArray((0..=5).map(|n| n.to_string()).collect()),
            VarValue::StringArray {
                max_len: 16,
                elements: vec![b"one".to_vec(), b"two".to_vec()],
            },
        ];
        let payload = encode_payload(&values).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.values, values);
    }

    #[test]
    fn ascii_round_trip_all_shapes() {
        let file = VarFile {
            filename: Some("VARS".to_string()),
            values: vec![
                VarValue::NumericScalar("42".to_string()),
                VarValue::StringScalar(b"Hi".to_vec()),
                VarValue::NumericArray(vec!["1".to_string(), "2".to_string()]),
                VarValue::StringArray {
                    max_len: 16,
                    elements: vec![b"a".to_vec(), b"b".to_vec()],
                },
            ],
        };
        let text = to_ascii(&file);
        let parsed = from_ascii(&text).unwrap();
        assert_eq!(parsed, file);
    }

    #[test]
    fn count_mismatch_errors() {
        let text = "; sde-variables:1.0 pc1500\n; Count: 2\n1\n2\n3\n";
        assert!(from_ascii(text).is_err());
    }

    #[test]
    fn string_escaping_round_trips() {
        let bytes = vec![b'a', b'\\', b'"', 0x01, b'z'];
        let escaped = escape_string(&bytes);
        assert!(escaped.contains("\\\\"));
        assert!(escaped.contains("\\\""));
        assert!(escaped.contains("\\x01"));
        assert_eq!(unescape_string(&escaped).unwrap(), bytes);
    }

    #[test]
    fn decode_rejects_bad_separator() {
        assert!(decode_payload(&[0x01, 0x0B, 0x00, 0x00, 0x88, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn decode_rejects_truncated_prefix() {
        assert!(decode_payload(&[0x00, 0x0B, 0x00]).is_err());
    }

    #[test]
    fn decode_rejects_record_running_past_buffer() {
        assert!(decode_payload(&[0x00, 0x0B, 0x00, 0x00, 0x88, 1, 2, 3]).is_err());
    }
}
