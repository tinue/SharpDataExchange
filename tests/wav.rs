//! Cassette WAV files: the checked-in recordings decode exactly, and the `sde` verbs
//! read and write them (`get`, `put`, `convert`, `info`) without changing any default.

use std::path::{Path, PathBuf};

use sharpdx::wav::{self, TapeFormat, TapeKind};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(name)).unwrap()
}

/// `10 PRINT "Hi"` as tokenized by bas2img (PC-1500 and PC-1600 tokens agree here).
const SIMPLE: [u8; 10] = [0x00, 0x0A, 0x07, 0xF0, 0x97, 0x22, 0x48, 0x69, 0x22, 0x0D];

#[test]
fn bin2wav_recordings() {
    for (f, format) in [
        ("wav/simple-1500-bin2wav.wav", TapeFormat::Pc1500Ce150),
        ("wav/simple-1600-bin2wav.wav", TapeFormat::Pc1600Ce1600p),
    ] {
        let files = wav::decode_files(&fixture(f)).unwrap();
        assert_eq!(files.len(), 1, "{f}");
        let t = &files[0];
        assert_eq!((t.format, t.kind, t.name.as_str()), (format, TapeKind::Basic, "SIMPLE.BAS"), "{f}");
        assert_eq!(t.payload, SIMPLE, "{f}");
    }
}

#[test]
fn rom_csave_m_recordings() {
    // `CSAVE M` of one poked byte (0B) by the emulated CE-150 / CE-1600P ROM
    // (Calc-U-1600), leaders trimmed.
    let f = &wav::decode_files(&fixture("wav/ml1-1500-rom.wav")).unwrap()[0];
    assert_eq!((f.format, f.kind, f.name.as_str()), (TapeFormat::Pc1500Ce150, TapeKind::Machine, "ML1"));
    assert_eq!((f.load, f.entry, f.payload.as_slice()), (0x7C01, 0xFFFF, &[0x0B][..]));
    let f = &wav::decode_files(&fixture("wav/ml1-1600-rom.wav")).unwrap()[0];
    assert_eq!((f.format, f.kind, f.name.as_str()), (TapeFormat::Pc1600Ce1600p, TapeKind::Machine, "ML1"));
    assert_eq!((f.load, f.payload.as_slice()), (0xD000, &[0x0B][..]));
    assert!(!f.autorun());
}

/// Re-encoding a decoded ROM recording gives back the same file.
#[test]
fn reencode_rom_recordings() {
    for name in ["wav/ml1-1500-rom.wav", "wav/ml1-1600-rom.wav"] {
        let a = wav::decode_files(&fixture(name)).unwrap();
        let b = wav::decode_files(&wav::encode(&a, &Default::default()).unwrap()).unwrap();
        assert_eq!(a[0].header, b[0].header, "{name}");
        assert_eq!(a[0].payload, b[0].payload, "{name}");
    }
}

/// Every WAV of the local research corpus, when it is there:
/// `cargo test --test wav -- --ignored`.
#[test]
#[ignore]
fn corpus_sweep() {
    let home = std::env::var("HOME").unwrap();
    let dirs = [
        format!("{home}/Development/sharp/pc1500/Downloaded"),
        format!("{home}/Development/sharp/Calc-U-1600/headless/tape-matrix"),
        format!("{home}/Development/sharp/SharpWavAnalysis"),
    ];
    let mut wavs = Vec::new();
    for d in &dirs {
        collect(Path::new(d), &mut wavs);
    }
    let (mut ok, mut empty) = (0, 0);
    for p in &wavs {
        let r = wav::decode(&std::fs::read(p).unwrap()).unwrap();
        if r.duration < 0.1 {
            empty += 1; // failed emulator runs leave empty files
            continue;
        }
        let pc1261_etc = p.to_string_lossy().contains("-1261") || p.to_string_lossy().contains("-140");
        if pc1261_etc {
            continue; // other pocket computers, not supported
        }
        assert!(r.issues.is_empty(), "{}: {:?}", p.display(), r.issues);
        assert!(!r.files.is_empty(), "{}: nothing decoded", p.display());
        ok += 1;
    }
    eprintln!("{ok} WAV files decoded, {empty} empty skipped");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue; // e.g. a Python .venv with library test files
        }
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            out.push(p);
        }
    }
}

// ── the `sde` binary ─────────────────────────────────────────────────────

#[cfg(feature = "cli")]
mod cli {
    use std::process::Command;

    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sde_wav_{test}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["depreciation.bas", "depreciation-tokenized-ce158header.bin"] {
            std::fs::copy(fixture_dir().join(f), dir.join(f)).unwrap();
        }
        std::fs::copy(fixture_dir().join("floppy/formatted.floppy.yaml"), dir.join("f.floppy.yaml")).unwrap();
        dir
    }

    fn sde(dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_sde")).args(args).current_dir(dir).output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(dir: &Path, args: &[&str]) -> String {
        let (success, stdout, stderr) = sde(dir, args);
        assert!(success, "sde {args:?} failed: {stderr}");
        stdout
    }

    fn fails(dir: &Path, args: &[&str]) -> String {
        let (success, _, stderr) = sde(dir, args);
        assert!(!success, "sde {args:?} should have failed");
        stderr
    }

    #[test]
    fn convert_listing_to_wav_and_back() {
        let d = scratch("convert");
        ok(&d, &["convert", "depreciation.bas", "-f", "wav", "--leader", "0.5"]);
        let w = std::fs::read(d.join("depreciation.wav")).unwrap();
        let f = &wav::decode_files(&w).unwrap()[0];
        assert_eq!((f.format, f.name.as_str()), (TapeFormat::Pc1500Ce150, "DEPRECIATION"));
        let want = fixture("depreciation-tokenized-ce158header.bin");
        assert_eq!(f.payload, want[27..]);

        // A WAV input converts like a received file: BASIC comes out as a listing,
        // named after the file on the tape.
        ok(&d, &["convert", "depreciation.wav"]);
        let listing = std::fs::read_to_string(d.join("DEPRECIATION.bas")).unwrap();
        assert!(listing.contains("INPUT \"COST?\";A"), "{listing}");

        // PC-1600 format with a name and sample rate.
        ok(
            &d,
            &[
                "convert",
                "depreciation.bas",
                "p16",
                "-f",
                "wav",
                "-d",
                "pc1600",
                "--name",
                "DEP",
                "--sample-rate",
                "22050",
            ],
        );
        let w = std::fs::read(d.join("p16.wav")).unwrap();
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 22050);
        let f = &wav::decode_files(&w).unwrap()[0];
        assert_eq!((f.format, f.name.as_str()), (TapeFormat::Pc1600Ce1600p, "DEP"));
    }

    #[test]
    fn defaults_are_unchanged() {
        let d = scratch("defaults");
        // convert still tokenizes to .bbin, never writes a WAV unasked.
        ok(&d, &["convert", "depreciation.bas"]);
        assert!(d.join("depreciation.bbin").exists());
        assert!(!d.join("depreciation.wav").exists());
        // The tape options need -f wav.
        assert!(fails(&d, &["convert", "depreciation.bas", "--leader", "3"]).contains("-f wav"));
        assert!(fails(&d, &["convert", "depreciation.bas", "-f", "binary"]).contains("only -f wav"));
    }

    #[test]
    fn get_info_put_from_a_wav() {
        let d = scratch("get");
        ok(&d, &["convert", "depreciation-tokenized-ce158header.bin", "tape.wav", "-f", "wav", "--leader", "0.5"]);
        let info = ok(&d, &["info", "tape.wav"]);
        assert!(info.starts_with("PC-1500 (CE-150) cassette WAV: BASIC program \"depreciation\""), "{info}");
        assert!(info.contains("kind:") && info.contains("wav-pc1500"), "{info}");

        // get: by content, no option; binary keeps the header.
        ok(&d, &["get", "tape.wav", "-f", "binary", "out.bin"]);
        let got = std::fs::read(d.join("out.bin")).unwrap();
        let want = fixture("depreciation-tokenized-ce158header.bin");
        // Same header fields (the CE-158 name is written upper-case) and payload.
        assert_eq!(got[..5], want[..5]);
        assert_eq!(&got[5..17], b"DEPRECIATION");
        assert_eq!(got[17..], want[17..]);
        // Picking a file by name, and a wrong name.
        ok(&d, &["get", "tape.wav:DEPRECIATION", "named.bas"]);
        assert!(d.join("named.bas").exists());
        assert!(fails(&d, &["get", "tape.wav:NOPE"]).contains("no file \"NOPE\""));

        // put: the WAV is decoded and sent like the image.
        let sent = ok(&d, &["put", "tape.wav", "--dry-run"]);
        assert!(sent.contains("613 bytes"), "{sent}");
        // PC-1500 tapes don't go onto the PC-1600 floppy (same rule as the image).
        assert!(fails(&d, &["put", "tape.wav", "f.floppy.yaml:A:"]).contains("PC-1600 only"));
    }

    #[test]
    fn get_wav_from_disk_and_put_wav_on_disk() {
        let d = scratch("disk");
        ok(&d, &["put", "depreciation.bas", "f.floppy.yaml:A:DEPR.BAS"]);
        ok(&d, &["get", "f.floppy.yaml:A:DEPR.BAS", ".", "-f", "wav", "--leader", "0.5"]);
        let f = &wav::decode_files(&std::fs::read(d.join("DEPR.wav")).unwrap()).unwrap()[0];
        assert_eq!((f.format, f.name.as_str()), (TapeFormat::Pc1600Ce1600p, "DEPR"));
        // The tape name names the disk file.
        ok(&d, &["put", "DEPR.wav", "f.floppy.yaml:B:"]);
        assert!(ok(&d, &["dir", "f.floppy.yaml:B"]).contains("DEPR.BAS"));
    }

    #[test]
    fn play_dry_run() {
        let d = scratch("play");
        let out = ok(&d, &["put", "depreciation.bas", "-f", "wav", "--dry-run", "--leader", "rom"]);
        assert!(out.contains("PC-1500 (CE-150) tape") && out.contains("would play"), "{out}");
        assert!(fails(&d, &["put", "depreciation.bas", "-f", "wav", "f.floppy.yaml:A:"]).contains("audio output"));
        assert!(fails(&d, &["put", "depreciation.bas", "--clean"]).contains("-f wav"));
    }
}
