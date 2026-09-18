//! PC-1500 8-byte numeric-variable codec, used by the Variables (SDAV) payload format.
//!
//! Format:
//! ```text
//!   byte 0    signed exponent, two's complement, range -99..=99
//!   byte 1    sign: 0x00 positive/zero, 0x80 negative
//!   bytes 2-6 5-byte packed BCD mantissa: 10 decimal digits, decimal point after digit 1
//!   byte 7    always 0x00
//! ```
//! `value = sign * d0.d1d2...d9 * 10^exponent`. All-zero bytes decode to `"0"`.
//!
//! A second, decode-only variant is also recognized: when byte 4 is `0xB2`, bytes 5-6
//! hold a big-endian signed 16-bit integer and the rest of the record is don't-care.
//! `encode` never produces this form.

use anyhow::{bail, Result};

const LEN: usize = 8;
const ALT_INT_MARKER_BYTE: usize = 4;
const ALT_INT_MARKER: u8 = 0xB2;

/// Decode an 8-byte PC-1500 numeric value to its canonical decimal text: plain
/// fixed-point for a base-10 exponent in `-4..=9`, otherwise `<d>.<ddd>E<+/-><exp>`.
pub fn decode(bytes: &[u8]) -> Result<String> {
    if bytes.len() != LEN {
        bail!(
            "BCD numeric value must be exactly 8 bytes, got {}",
            bytes.len()
        );
    }
    if bytes[ALT_INT_MARKER_BYTE] == ALT_INT_MARKER {
        let v = i16::from_be_bytes([bytes[5], bytes[6]]);
        return Ok(v.to_string());
    }

    let exponent = bytes[0] as i8 as i32;
    let negative = bytes[1] == 0x80;
    let mantissa: [u8; 5] = bytes[2..7].try_into().unwrap();
    let digits = unpack_bcd5(&mantissa);

    if digits.iter().all(|&d| d == 0) {
        return Ok("0".to_string());
    }

    Ok(format_value(&digits, exponent, negative))
}

/// Encode decimal text (plain or scientific, e.g. `"3.14"`, `"-5"`, `"1.5e-10"`) to the
/// 8-byte PC-1500 form. Rounds to 10 significant digits, half-up. Errors if the
/// resulting exponent falls outside `-99..=99`, or the text doesn't parse as a number.
pub fn encode(value: &str) -> Result<[u8; 8]> {
    let (negative, digits, point_pos) = parse_decimal(value)?;

    let Some(first_nonzero) = digits.iter().position(|&d| d != 0) else {
        return Ok([0u8; 8]);
    };
    let sig = &digits[first_nonzero..];

    let mut mantissa = [0u8; 10];
    let mut exp_adjust = 0i32;
    if sig.len() <= 10 {
        mantissa[..sig.len()].copy_from_slice(sig);
    } else {
        mantissa.copy_from_slice(&sig[..10]);
        if sig[10] >= 5 {
            let mut i: isize = 9;
            let mut carry = true;
            while carry && i >= 0 {
                if mantissa[i as usize] == 9 {
                    mantissa[i as usize] = 0;
                } else {
                    mantissa[i as usize] += 1;
                    carry = false;
                }
                i -= 1;
            }
            if carry {
                // All ten digits were 9: the cascade above already zeroed them all.
                mantissa[0] = 1;
                exp_adjust = 1;
            }
        }
    }

    let exponent = point_pos - first_nonzero as i32 - 1 + exp_adjust;
    if !(-99..=99).contains(&exponent) {
        bail!("numeric value out of range for PC-1500 storage (exponent {exponent}, must be -99..=99)");
    }

    let mut out = [0u8; 8];
    out[0] = exponent as i8 as u8;
    out[1] = if negative { 0x80 } else { 0x00 };
    out[2..7].copy_from_slice(&pack_bcd5(&mantissa));
    Ok(out)
}

fn unpack_bcd5(b: &[u8; 5]) -> [u8; 10] {
    let mut d = [0u8; 10];
    for i in 0..5 {
        d[i * 2] = (b[i] >> 4) & 0x0F;
        d[i * 2 + 1] = b[i] & 0x0F;
    }
    d
}

fn pack_bcd5(d: &[u8; 10]) -> [u8; 5] {
    let mut b = [0u8; 5];
    for i in 0..5 {
        b[i] = (d[i * 2] << 4) | d[i * 2 + 1];
    }
    b
}

/// Render normalized digits (`digits[0]` significant/nonzero) plus a base-10 exponent
/// as decimal text, matching `encode`'s rounding/formatting contract.
fn format_value(digits: &[u8; 10], exponent: i32, negative: bool) -> String {
    let last_nonzero = digits.iter().rposition(|&d| d != 0).unwrap_or(0);
    let sig = &digits[..=last_nonzero];
    let sign_str = if negative { "-" } else { "" };

    if (-4..=9).contains(&exponent) {
        plain_decimal(sig, exponent + 1, sign_str)
    } else {
        let mantissa = if sig.len() == 1 {
            sig[0].to_string()
        } else {
            let frac: String = sig[1..].iter().map(|d| d.to_string()).collect();
            format!("{}.{}", sig[0], frac)
        };
        let exp_sign = if exponent >= 0 { "+" } else { "-" };
        format!("{sign_str}{mantissa}E{exp_sign}{}", exponent.abs())
    }
}

/// `sig` is the trimmed significant-digit sequence; `point_pos` is how many of its
/// digits sit before the decimal point (may be `<= 0` or `>= sig.len()`).
fn plain_decimal(sig: &[u8], point_pos: i32, sign_str: &str) -> String {
    let n = sig.len() as i32;
    let digits_str = |d: &[u8]| -> String { d.iter().map(|x| x.to_string()).collect() };
    if point_pos <= 0 {
        format!(
            "{sign_str}0.{}{}",
            "0".repeat((-point_pos) as usize),
            digits_str(sig)
        )
    } else if point_pos >= n {
        format!(
            "{sign_str}{}{}",
            digits_str(sig),
            "0".repeat((point_pos - n) as usize)
        )
    } else {
        let (int_digits, frac_digits) = sig.split_at(point_pos as usize);
        format!(
            "{sign_str}{}.{}",
            digits_str(int_digits),
            digits_str(frac_digits)
        )
    }
}

/// Parse decimal text into `(negative, digits, point_pos)`, where `digits` is the
/// concatenation of the integer and fractional parts (no leading/trailing-zero
/// stripping) and `point_pos` is how many of those digits sit before the decimal
/// point, adjusted for any `e`/`E` exponent.
fn parse_decimal(input: &str) -> Result<(bool, Vec<u8>, i32)> {
    let s = input.trim();
    let mut rest = s;
    let mut negative = false;
    match rest.as_bytes().first() {
        Some(b'-') => {
            negative = true;
            rest = &rest[1..];
        }
        Some(b'+') => rest = &rest[1..],
        _ => {}
    }

    let (mantissa_part, exp_part) = match rest.find(['e', 'E']) {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    let sci_exp: i32 = match exp_part {
        Some(e) => e
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid decimal value: {input:?}"))?,
        None => 0,
    };

    let (int_part, frac_part) = match mantissa_part.find('.') {
        Some(idx) => (&mantissa_part[..idx], &mantissa_part[idx + 1..]),
        None => (mantissa_part, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        bail!("invalid decimal value: {input:?}");
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("invalid decimal value: {input:?}");
    }

    let mut digits: Vec<u8> = Vec::with_capacity(int_part.len() + frac_part.len());
    digits.extend(int_part.bytes().map(|b| b - b'0'));
    digits.extend(frac_part.bytes().map(|b| b - b'0'));
    let point_pos = int_part.len() as i32 + sci_exp;
    Ok((negative, digits, point_pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_round_trips() {
        assert_eq!(decode(&[0, 0, 0, 0, 0, 0, 0, 0]).unwrap(), "0");
        assert_eq!(encode("0").unwrap(), [0u8; 8]);
    }

    #[test]
    fn positive_integer() {
        let bytes = [0x03, 0x00, 0x15, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode(&bytes).unwrap(), "1500");
        assert_eq!(encode("1500").unwrap(), bytes);
    }

    #[test]
    fn negative_integer() {
        let bytes = [0x08, 0x80, 0x12, 0x34, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(decode(&bytes).unwrap(), "-123400000");
        assert_eq!(encode("-123400000").unwrap(), bytes);
    }

    #[test]
    fn small_decimal_strips_trailing_zero_on_decode() {
        let bytes = [0xFD, 0x00, 0x12, 0x34, 0x56, 0x78, 0x90, 0x00];
        assert_eq!(decode(&bytes).unwrap(), "0.00123456789");
        assert_eq!(encode("0.001234567890").unwrap(), bytes);
    }

    #[test]
    fn ten_digit_mantissa() {
        let bytes = [0x00, 0x00, 0x31, 0x41, 0x59, 0x26, 0x54, 0x00];
        assert_eq!(decode(&bytes).unwrap(), "3.141592654");
        assert_eq!(encode("3.141592654").unwrap(), bytes);
    }

    #[test]
    fn alt_int_variant_decodes() {
        assert_eq!(
            decode(&[0x00, 0x00, 0x00, 0x00, 0xB2, 0x05, 0xDC, 0x00]).unwrap(),
            "1500"
        );
        assert_eq!(
            decode(&[0x00, 0x00, 0x00, 0x00, 0xB2, 0xFF, 0xFB, 0x00]).unwrap(),
            "-5"
        );
    }

    #[test]
    fn encode_rejects_out_of_range_exponent() {
        assert!(encode("1e100").is_err());
    }

    #[test]
    fn decode_rejects_wrong_length() {
        assert!(decode(&[0u8; 7]).is_err());
        assert!(decode(&[0u8; 9]).is_err());
    }

    #[test]
    fn encode_rejects_garbage() {
        assert!(encode("not a number").is_err());
        assert!(encode("").is_err());
    }

    #[test]
    fn round_trip_value_identical() {
        for v in ["1500", "-123400000", "3.141592654", "1", "0.5", "-0.5"] {
            let bytes = encode(v).unwrap();
            let back = decode(&bytes).unwrap();
            assert_eq!(encode(&back).unwrap(), bytes, "round-trip mismatch for {v}");
        }
    }

    #[test]
    fn rounds_half_up_beyond_ten_significant_digits() {
        // 11 significant digits (1234567891|5) -> the 11th digit (5) rounds the 10th up.
        let bytes = encode("1.2345678915").unwrap();
        assert_eq!(decode(&bytes).unwrap(), "1.234567892");
    }

    #[test]
    fn rounding_carry_overflows_all_nines() {
        // 9.999999999_5 rounds up through every digit -> 10.00000000, exponent bumps by one.
        let bytes = encode("9.9999999995").unwrap();
        assert_eq!(decode(&bytes).unwrap(), "10");
    }

    #[test]
    fn scientific_notation_round_trips() {
        let bytes = encode("1.23e50").unwrap();
        let text = decode(&bytes).unwrap();
        assert_eq!(text, "1.23E+50");
        assert_eq!(encode(&text).unwrap(), bytes);

        let bytes = encode("1.23e-50").unwrap();
        let text = decode(&bytes).unwrap();
        assert_eq!(text, "1.23E-50");
        assert_eq!(encode(&text).unwrap(), bytes);
    }
}
