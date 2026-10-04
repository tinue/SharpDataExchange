//! RIFF/WAVE container: a tolerant reader (any common PCM / float layout, mono or
//! multi-channel) and a plain 16-bit mono PCM writer.

use super::TapeError;

/// Decoded audio: one channel, normalised to `-1.0..=1.0`, plus the source facts.
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub float: bool,
    /// The channel the decoder uses (the loudest one, for multi-channel files).
    pub samples: Vec<f32>,
}

/// `true` when `buf` starts like a RIFF/WAVE file.
pub fn is_wav(buf: &[u8]) -> bool {
    buf.len() >= 12 && &buf[0..4] == b"RIFF" && &buf[8..12] == b"WAVE"
}

const FMT_PCM: u16 = 1;
const FMT_FLOAT: u16 = 3;
const FMT_EXTENSIBLE: u16 = 0xFFFE;

/// Lowest sample rate accepted: a PC-1500 "1" tone (2500 Hz) needs one sample per half
/// cycle, so 5 kHz is the practical floor; 4 kHz leaves a little room for slow tapes.
const MIN_RATE: u32 = 4000;

pub fn read(buf: &[u8]) -> Result<Pcm, TapeError> {
    if !is_wav(buf) {
        return Err(TapeError::NotWav("no RIFF/WAVE signature".into()));
    }
    let mut fmt: Option<(u16, u16, u32, u16, u16)> = None; // tag, channels, rate, align, bits
    let mut data: Option<&[u8]> = None;
    let mut pos = 12;
    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        // Tolerate a truncated last chunk (common in recordings cut short).
        let body_end = body_start.saturating_add(size).min(buf.len());
        let body = &buf[body_start..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                let le16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
                let mut tag = le16(0);
                if tag == FMT_EXTENSIBLE && body.len() >= 26 {
                    tag = le16(24); // first two bytes of the sub-format GUID
                }
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                fmt = Some((tag, le16(2), rate, le16(12), le16(14)));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos = body_start.saturating_add(size + (size & 1));
    }
    let (tag, channels, rate, align, bits) = fmt.ok_or_else(|| TapeError::NotWav("no fmt chunk".into()))?;
    let data = data.ok_or_else(|| TapeError::NotWav("no data chunk".into()))?;
    let float = match (tag, bits) {
        (FMT_PCM, 8 | 16 | 24 | 32) => false,
        (FMT_FLOAT, 32 | 64) => true,
        _ => {
            return Err(TapeError::Unsupported(format!(
                "WAV encoding {tag:#06x} with {bits} bits per sample (use PCM or float)"
            )))
        }
    };
    if channels == 0 || rate < MIN_RATE {
        return Err(TapeError::Unsupported(format!(
            "{channels} channel(s) at {rate} Hz (need at least {MIN_RATE} Hz)"
        )));
    }
    let bytes = (bits / 8) as usize;
    let frame = (align as usize).max(bytes * channels as usize);
    let frames = data.len() / frame;
    let sample = |f: usize, c: usize| -> f32 {
        let o = f * frame + c * bytes;
        let s = &data[o..o + bytes];
        match (float, bytes) {
            (false, 1) => (s[0] as f32 - 128.0) / 128.0,
            (false, 2) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (false, 3) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (false, _) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            (true, 4) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (true, _) => f64::from_le_bytes(s[..8].try_into().unwrap()) as f32,
        }
    };
    // Pick the channel with the most AC energy: stereo rips often carry the tape on
    // one side only, and mixing would halve (or cancel) the signal.
    let mut best = 0;
    if channels > 1 {
        let mut best_e = -1.0f64;
        for c in 0..channels as usize {
            let mean = (0..frames).map(|f| sample(f, c) as f64).sum::<f64>() / frames.max(1) as f64;
            let e: f64 = (0..frames).map(|f| (sample(f, c) as f64 - mean).powi(2)).sum();
            if e > best_e {
                best_e = e;
                best = c;
            }
        }
    }
    let samples = (0..frames).map(|f| sample(f, best)).collect();
    Ok(Pcm { sample_rate: rate, channels, bits, float, samples })
}

/// 16-bit mono PCM WAV from samples in `-1.0..=1.0`.
pub fn write_pcm16(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&FMT_PCM.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_round_trip() {
        let s: Vec<f32> = (0..100).map(|i| ((i as f32) / 10.0).sin() * 0.5).collect();
        let wav = write_pcm16(&s, 48000);
        assert!(is_wav(&wav));
        let pcm = read(&wav).unwrap();
        assert_eq!((pcm.sample_rate, pcm.channels, pcm.bits), (48000, 1, 16));
        for (a, b) in s.iter().zip(&pcm.samples) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn picks_loudest_channel_of_8bit_stereo() {
        let mut wav = Vec::new();
        let frames = 64u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 2, 0]);
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&[2, 0, 8, 0]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            wav.push(128); // silent left
            wav.push(if i % 2 == 0 { 200 } else { 56 }); // tone right
        }
        let pcm = read(&wav).unwrap();
        assert_eq!(pcm.channels, 2);
        assert!(pcm.samples[0] > 0.5 && pcm.samples[1] < -0.5);
    }
}
