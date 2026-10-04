//! Round-trip and robustness tests: encode, distort like a real tape / sound card
//! would, decode, and require the exact payload — or a clean rejection.

use super::*;

fn file(format: TapeFormat, kind: TapeKind, len: usize) -> TapeFile {
    let mut x = 0x1234_5678u32;
    let payload = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    TapeFile {
        format,
        kind,
        name: "TEST".into(),
        load: if kind == TapeKind::Machine { 0x7C01 } else { 0 },
        entry: if kind == TapeKind::Machine { 0xFFFF } else { 0 },
        payload,
        header: Vec::new(),
        start_time: 0.0,
        end_time: 0.0,
        speed: 1.0,
    }
}

fn short_leader(rate: u32) -> EncodeOptions {
    EncodeOptions { sample_rate: rate, leader: Leader::Seconds(0.5) }
}

fn decode_samples(s: &[f32], rate: u32) -> DecodeReport {
    decode(&riff::write_pcm16(s, rate)).unwrap()
}

fn assert_round_trip(f: &TapeFile, s: &[f32], rate: u32, what: &str) {
    let r = decode_samples(s, rate);
    assert!(r.issues.is_empty(), "{what}: issues {:?}", r.issues);
    assert_eq!(r.files.len(), 1, "{what}: {} files", r.files.len());
    let g = &r.files[0];
    assert_eq!((g.format, g.kind, &g.name), (f.format, f.kind, &f.name), "{what}");
    assert_eq!(g.payload, f.payload, "{what}: payload");
    if f.kind == TapeKind::Machine {
        assert_eq!((g.load, g.autorun()), (f.load, f.autorun()), "{what}: addresses");
    }
}

#[test]
fn round_trip_sizes_both_formats() {
    for format in [TapeFormat::Pc1500Ce150, TapeFormat::Pc1600Ce1600p] {
        for kind in [TapeKind::Basic, TapeKind::Machine] {
            for len in [1, 79, 80, 81, 255, 256, 257] {
                let f = file(format, kind, len);
                let s = encode_samples(std::slice::from_ref(&f), &short_leader(22050)).unwrap();
                assert_round_trip(&f, &s, 22050, &format!("{format:?} {kind:?} {len}"));
            }
        }
    }
}

#[test]
fn pc1500_reserve_and_basic_empty_program() {
    let f = file(TapeFormat::Pc1500Ce150, TapeKind::Reserve, 188);
    let s = encode_samples(std::slice::from_ref(&f), &short_leader(22050)).unwrap();
    assert_round_trip(&f, &s, 22050, "reserve");
    // An empty BASIC program is just the FF end mark on tape.
    let f = file(TapeFormat::Pc1500Ce150, TapeKind::Basic, 0);
    let s = encode_samples(std::slice::from_ref(&f), &short_leader(22050)).unwrap();
    assert_round_trip(&f, &s, 22050, "empty basic");
}

#[test]
fn several_files_on_one_tape() {
    for format in [TapeFormat::Pc1500Ce150, TapeFormat::Pc1600Ce1600p] {
        let mut a = file(format, TapeKind::Basic, 100);
        a.name = "ONE".into();
        let mut b = file(format, TapeKind::Machine, 30);
        b.name = "TWO".into();
        let s = encode_samples(&[a.clone(), b.clone()], &short_leader(22050)).unwrap();
        let r = decode_samples(&s, 22050);
        assert!(r.issues.is_empty(), "{:?}", r.issues);
        let names: Vec<_> = r.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["ONE", "TWO"], "{format:?}");
        assert_eq!(r.files[1].payload, b.payload);
    }
}

#[test]
fn header_fields_match_the_rom() {
    // PC-1500: as the CE-150's own CSAVE M writes it (ROM-made recording, ml80).
    let mut f = file(TapeFormat::Pc1500Ce150, TapeKind::Machine, 80);
    f.name = "ML80".into();
    let r = decode_samples(&encode_samples(&[f], &short_leader(22050)).unwrap(), 22050);
    assert_eq!(
        r.files[0].header,
        [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x00, b'M', b'L', b'8', b'0', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x7C, 0x01, 0x00, 0x4F, 0xFF, 0xFF
        ]
    );
    // PC-1600: as the CE-1600P's CSAVE M writes it (ROM-made recording, ml257).
    let mut f = file(TapeFormat::Pc1600Ce1600p, TapeKind::Machine, 257);
    f.name = "ML257".into();
    f.load = 0xD000;
    let r = decode_samples(&encode_samples(&[f], &short_leader(22050)).unwrap(), 22050);
    let mut want = vec![0u8; 48];
    want[..6].copy_from_slice(&[0x01, b'M', b'L', b'2', b'5', b'7']);
    want[0x11..0x20].copy_from_slice(&[0x0D, 0x01, 0x01, 0x00, 0xD0, 0xFF, 0xFF, 0x00, 1, 1, 0, 0, 0, 0, 0xFF]);
    assert_eq!(r.files[0].header, want);
}

// ---- distortions -------------------------------------------------------------------

/// Play back `speed` times faster (linear-interpolation resampling).
fn resample(s: &[f32], speed: f64) -> Vec<f32> {
    let n = (s.len() as f64 / speed) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * speed;
            let k = x as usize;
            let f = (x - k as f64) as f32;
            let a = s[k.min(s.len() - 1)];
            let b = s[(k + 1).min(s.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

/// Wow and flutter: the playback speed swings by ±`depth` at `hz`.
fn flutter(s: &[f32], rate: u32, depth: f64, hz: f64) -> Vec<f32> {
    let mut out = Vec::with_capacity(s.len());
    let mut x = 0.0f64;
    let mut i = 0usize;
    while (x as usize) + 1 < s.len() {
        let k = x as usize;
        let f = (x - k as f64) as f32;
        out.push(s[k] + (s[k + 1] - s[k]) * f);
        i += 1;
        x += 1.0 + depth * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin();
    }
    out
}

fn noise(s: &[f32], amplitude: f32) -> Vec<f32> {
    let mut x = 0x9E37_79B9u32;
    s.iter()
        .map(|&v| {
            // Sum of 4 uniforms ≈ Gaussian.
            let mut g = 0.0f32;
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                g += (x as f32 / u32::MAX as f32) - 0.5;
            }
            v + g * amplitude
        })
        .collect()
}

fn lowpass(s: &[f32], rate: u32, hz: f64) -> Vec<f32> {
    let a = (-2.0 * std::f64::consts::PI * hz / rate as f64).exp() as f32;
    let mut y = 0.0;
    s.iter()
        .map(|&v| {
            y = a * y + (1.0 - a) * v;
            y
        })
        .collect()
}

fn quantize8(s: &[f32]) -> Vec<f32> {
    s.iter().map(|&v| ((v * 127.0).round() / 127.0).clamp(-1.0, 1.0)).collect()
}

fn distortions(s: &[f32], rate: u32) -> Vec<(&'static str, Vec<f32>)> {
    vec![
        ("15 % fast", resample(s, 1.15)),
        ("25 % fast", resample(s, 1.25)),
        ("25 % slow", resample(s, 0.75)),
        ("flutter 5 % / 6 Hz", flutter(s, rate, 0.05, 6.0)),
        ("wow 8 % / 0.5 Hz", flutter(s, rate, 0.08, 0.5)),
        ("low-pass 2.5 kHz", lowpass(s, rate, 2500.0)),
        ("15 % slow", resample(s, 0.85)),
        ("wow 3 % / 1 Hz", flutter(s, rate, 0.03, 1.0)),
        ("flutter 2 % / 12 Hz", flutter(s, rate, 0.02, 12.0)),
        ("-40 dBFS", s.iter().map(|v| v * 0.01).collect()),
        ("noise 20 dB SNR", noise(s, 0.12)),
        ("inverted", s.iter().map(|v| -v).collect()),
        ("DC offset", s.iter().map(|v| v * 0.5 + 0.3).collect()),
        ("low-pass 4 kHz", lowpass(s, rate, 4000.0)),
        ("8-bit", quantize8(&s.iter().map(|v| v * 0.3).collect::<Vec<_>>())),
        (
            "slow + flutter + noise + quiet",
            noise(&flutter(&resample(s, 0.9), rate, 0.02, 8.0), 0.01).iter().map(|v| v * 0.1).collect(),
        ),
    ]
}

#[test]
fn survives_tape_and_sound_card_distortions() {
    let rate = 44100;
    for format in [TapeFormat::Pc1500Ce150, TapeFormat::Pc1600Ce1600p] {
        let f = file(format, TapeKind::Machine, 300);
        let clean = encode_samples(std::slice::from_ref(&f), &short_leader(rate)).unwrap();
        for (what, s) in distortions(&clean, rate) {
            assert_round_trip(&f, &s, rate, &format!("{format:?}, {what}"));
        }
    }
}

#[test]
fn pc1500_at_5khz_8bit() {
    let f = file(TapeFormat::Pc1500Ce150, TapeKind::Basic, 200);
    let s = encode_samples(std::slice::from_ref(&f), &short_leader(5000)).unwrap();
    assert_round_trip(&f, &quantize8(&s), 5000, "5 kHz");
}

#[test]
fn corrupted_block_is_rejected_not_misread() {
    let rate = 22050;
    for format in [TapeFormat::Pc1500Ce150, TapeFormat::Pc1600Ce1600p] {
        let f = file(format, TapeKind::Machine, 200);
        let mut s = encode_samples(std::slice::from_ref(&f), &short_leader(rate)).unwrap();
        // Wipe 3 ms inside the data: a dropout on the tape. (PC-1500: the data fills
        // most of the tape; PC-1600: it is the last second before the 0.5 s of silence.)
        let at = match format {
            TapeFormat::Pc1500Ce150 => s.len() / 2,
            TapeFormat::Pc1600Ce1600p => s.len() - rate as usize,
        };
        for v in &mut s[at..at + rate as usize * 3 / 1000] {
            *v = 0.0;
        }
        let r = decode_samples(&s, rate);
        assert!(r.files.is_empty(), "{format:?}: a damaged file must not decode");
        assert_eq!(r.issues.len(), 1, "{format:?}: {:?}", r.issues);
        assert_eq!(r.issues[0].name.as_deref(), Some("TEST"));
        assert!(matches!(decode_files(&riff::write_pcm16(&s, rate)), Err(TapeError::Corrupt(_))));
    }
}

#[test]
fn silence_and_non_tape_audio_have_no_files() {
    let rate = 22050;
    let silence = vec![0.0f32; rate as usize];
    assert!(matches!(decode_files(&riff::write_pcm16(&silence, rate)), Err(TapeError::NoSignal)));
    let tone: Vec<f32> = (0..rate * 2).map(|i| (i as f32 * 0.31).sin() * 0.5).collect();
    let r = decode_samples(&tone, rate);
    assert!(r.files.is_empty() && r.issues.is_empty());
}

#[test]
fn leader_lengths() {
    let f = file(TapeFormat::Pc1500Ce150, TapeKind::Machine, 10);
    let len = |l| {
        let s = encode_samples(std::slice::from_ref(&f), &EncodeOptions { sample_rate: 8000, leader: l }).unwrap();
        duration(&s, 8000)
    };
    let (d, rom, long) = (len(Leader::Default), len(Leader::Rom), len(Leader::Seconds(20.0)));
    assert!(rom - d > 5.5 && rom - d < 6.5, "default {d} rom {rom}");
    assert!((long - d - 18.0).abs() < 0.1);
    assert!(encode_samples(&[f], &EncodeOptions { sample_rate: 8000, leader: Leader::Seconds(0.0) }).is_err());
}

#[test]
fn image_round_trip() {
    // CE-158 BASIC image → tape file → image is the identity.
    let raw =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/depreciation-tokenized-ce158header.bin"))
            .unwrap();
    let f = from_image(&raw, None).unwrap();
    assert_eq!((f.format, f.kind, f.name.as_str()), (TapeFormat::Pc1500Ce150, TapeKind::Basic, "depreciation"));
    assert_eq!(f.payload, raw[27..]);
    let back = to_image(&f).unwrap();
    assert_eq!(back[27..], raw[27..]);
    let raw =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/depreciation-tokenized-pc1600header.bin"))
            .unwrap();
    let f = from_image(&raw, Some("DEPR")).unwrap();
    assert_eq!((f.format, f.name.as_str()), (TapeFormat::Pc1600Ce1600p, "DEPR"));
    assert_eq!(to_image(&f).unwrap(), raw);
}
