//! End-to-end `sde dir/get/put/del` on Calc-U-1600 floppy images, driving the real binary
//! against copies of the fixtures in `tests/fixtures/floppy/`.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::Command;

use sharpdx::diskfs::{FileName, Volume};
use sharpdx::floppy_image::{self, Side};
use sharpdx::registry::{self, Device};
use sharpdx::{detokenize, header, LineEnding, SegmentMarker};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A fresh scratch directory holding copies of the floppy fixtures.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sde_disk_{test}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["blank.floppy.yaml", "formatted.floppy.yaml", "dw.floppy.yaml"] {
        std::fs::copy(fixture_dir().join("floppy").join(f), dir.join(f)).unwrap();
    }
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

/// The image text without its `saved:` line.
fn without_saved(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap().lines().filter(|l| !l.starts_with("saved:")).collect::<Vec<_>>().join("\n")
}

#[test]
fn dir_lists_both_sides() {
    let dir = scratch("dir");
    let out = ok(&dir, &["dir", "dw.floppy.yaml"]);
    assert!(out.contains("GLOBUS.BAS"), "{out}");
    assert!(out.contains("BIO.BAS"), "{out}");
    assert!(out.contains("2 files, 53760 bytes free"), "{out}");
    assert!(out.contains("side B\n  not formatted"), "{out}");
    let out = ok(&dir, &["dir", "formatted.floppy.yaml:B"]);
    assert!(out.contains("0 files, 62464 bytes free"), "{out}");
    let out = ok(&dir, &["dir", "dw.floppy.yaml:A:G*.*"]);
    assert!(out.contains("GLOBUS.BAS") && !out.contains("BIO.BAS"), "{out}");
}

/// A program the PC-1600 ROM saved re-tokenizes to exactly the stored bytes.
#[test]
fn rom_saved_basic_retokenizes_identically() {
    let img = floppy_image::read_path(&fixture_dir().join("floppy/dw.floppy.yaml")).unwrap();
    let vol = Volume::open_floppy_side(img.side(Side::A)).unwrap();
    let stored = vol.read(&vol.find(&FileName::parse("BIO.BAS").unwrap()).unwrap()).unwrap();
    let h = header::find(&stored).unwrap();
    let listing = detokenize::detokenize_to_text(&stored[h.payload_start()..], registry::pc1600(), LineEnding::Lf)
        .unwrap();
    let again =
        sharpdx::convert_with(listing.as_bytes(), Device::Pc1600, None, true, LineEnding::Lf, SegmentMarker::Memory)
            .unwrap();
    assert_eq!(again.bytes, stored);
}

#[test]
fn put_get_roundtrips() {
    let dir = scratch("roundtrip");
    std::fs::copy(fixture_dir().join("printtest-1600.bas"), dir.join("print.bas")).unwrap();
    std::fs::copy(fixture_dir().join("depreciation-tokenized-pc1600header.bin"), dir.join("dep.bin")).unwrap();
    std::fs::write(dir.join("notes.txt"), "Grüße, Ärger\n\tindented\n").unwrap();
    std::fs::write(dir.join("mc.bin"), [0x3E, 0x01, 0xC9]).unwrap();

    ok(&dir, &["put", "print.bas", "dep.bin", "notes.txt", "formatted.floppy.yaml:A:"]);
    ok(&dir, &["put", "mc.bin", "formatted.floppy.yaml:B:", "--start-address", "0x2C000"]);
    let listing = ok(&dir, &["dir", "formatted.floppy.yaml"]);
    assert!(listing.contains("PRINT.BAS") && listing.contains("DEP.BAS") && listing.contains("NOTES.TXT"), "{listing}");
    assert!(listing.contains("machine, load 02C000"), "{listing}");

    std::fs::create_dir(dir.join("out")).unwrap();
    ok(&dir, &["get", "formatted.floppy.yaml:A:*", "out", "--eol", "lf"]);
    // BASIC listing: a fixed point of tokenize/de-tokenize, so compare with sde's own view.
    let want = sharpdx::convert_with(
        &std::fs::read(dir.join("print.bas")).unwrap(),
        Device::Pc1600,
        None,
        true,
        LineEnding::Lf,
        SegmentMarker::Memory,
    )
    .unwrap();
    let got = std::fs::read(dir.join("out/PRINT.bas")).unwrap();
    let back = sharpdx::convert_with(&got, Device::Pc1600, None, true, LineEnding::Lf, SegmentMarker::Memory).unwrap();
    assert_eq!(back.bytes, want.bytes);
    // A file with a PC-1600 header is stored unchanged and comes back identical in binary.
    ok(&dir, &["get", "formatted.floppy.yaml:A:DEP.BAS", "dep2.bin", "-f", "binary"]);
    assert_eq!(std::fs::read(dir.join("dep2.bin")).unwrap(), std::fs::read(dir.join("dep.bin")).unwrap());
    // Text round-trips exactly (UTF-8 -> CP437 -> UTF-8).
    assert_eq!(std::fs::read_to_string(dir.join("out/NOTES.TXT")).unwrap(), "Grüße, Ärger\n\tindented\n");
    // Machine code keeps its header by default; --skip-header gives the bare code.
    ok(&dir, &["get", "formatted.floppy.yaml:B:MC.BIN", "mc2.bin", "--skip-header"]);
    assert_eq!(std::fs::read(dir.join("mc2.bin")).unwrap(), [0x3E, 0x01, 0xC9]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn put_then_del_restores_image() {
    let dir = scratch("putdel");
    let img = dir.join("dw.floppy.yaml");
    let before = without_saved(&img);
    std::fs::write(dir.join("a.asm"), "LD A,0\n").unwrap();
    std::fs::write(dir.join("b.cfg"), "X=1\n").unwrap();
    ok(&dir, &["put", "a.asm", "b.cfg", "dw.floppy.yaml:A:"]);
    assert_ne!(without_saved(&img), before);
    ok(&dir, &["del", "dw.floppy.yaml:A:A.ASM", "dw.floppy.yaml:A:B.CFG"]);
    // Deleted entries are marked E5 rather than restored to 00, so compare the files.
    let vol_img = floppy_image::read_path(&img).unwrap();
    let vol = Volume::open_floppy_side(vol_img.side(Side::A)).unwrap();
    let names: Vec<String> = vol.list().iter().map(|e| e.name.to_string()).collect();
    assert_eq!(names, vec!["GLOBUS.BAS", "BIO.BAS"]);
    assert_eq!(vol.free_bytes(), 53760);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failures_leave_the_image_unchanged() {
    let dir = scratch("failures");
    let img = dir.join("dw.floppy.yaml");
    let before = std::fs::read(&img).unwrap();
    std::fs::write(dir.join("ok.txt"), "fine\n").unwrap();
    std::fs::write(dir.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(dir.join("globus.bas"), "10 END\n").unwrap();

    // Second file fails -> nothing written, not even the first.
    let err = fails(&dir, &["put", "ok.txt", "blob.bin", "dw.floppy.yaml:A:"]);
    assert!(err.contains("--start-address"), "{err}");
    // Existing file without --force.
    let err = fails(&dir, &["put", "globus.bas", "dw.floppy.yaml:A:"]);
    assert!(err.contains("already exists"), "{err}");
    // Dry run.
    ok(&dir, &["put", "ok.txt", "dw.floppy.yaml:A:", "--dry-run"]);
    ok(&dir, &["del", "dw.floppy.yaml:A:*.BAS", "--dry-run"]);
    // Delete of a missing file; unformatted side; bad side; serial-only options.
    assert!(fails(&dir, &["del", "dw.floppy.yaml:A:NOPE.BAS"]).contains("not found"));
    assert!(fails(&dir, &["put", "ok.txt", "dw.floppy.yaml:B:"]).contains("not formatted"));
    assert!(fails(&dir, &["get", "dw.floppy.yaml:C:X"]).contains("A or B"));
    assert!(fails(&dir, &["put", "ok.txt", "dw.floppy.yaml:A:", "--device", "pc1500"]).contains("PC-1600"));
    assert!(fails(&dir, &["get", "dw.floppy.yaml:A:BIO.BAS", "--port", "/dev/null"]).contains("--port"));
    assert_eq!(std::fs::read(&img).unwrap(), before);

    // --force replaces.
    ok(&dir, &["put", "globus.bas", "dw.floppy.yaml:A:", "--force"]);
    let out = ok(&dir, &["dir", "dw.floppy.yaml:A"]);
    assert!(out.contains("GLOBUS.BAS") && out.contains("59392 bytes free"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file whose first field is a number is text, not BASIC; `-f binary` forces it.
#[test]
fn number_column_data_file_stays_text() {
    let dir = scratch("pun");
    std::fs::copy(fixture_dir().join("floppy/ATLANTIS.PUN"), dir.join("ATLANTIS.PUN")).unwrap();
    ok(&dir, &["put", "ATLANTIS.PUN", "formatted.floppy.yaml:A:"]);
    let out = ok(&dir, &["dir", "formatted.floppy.yaml:A"]);
    assert!(out.contains("ATLANTIS.PUN") && out.contains("text"), "{out}");
    ok(&dir, &["get", "formatted.floppy.yaml:A:ATLANTIS.PUN", "raw.pun", "--raw"]);
    assert_eq!(std::fs::read(dir.join("raw.pun")).unwrap(), std::fs::read(dir.join("ATLANTIS.PUN")).unwrap());
    std::fs::create_dir(dir.join("out")).unwrap();
    let got = ok(&dir, &["get", "formatted.floppy.yaml:A:ATLANTIS.PUN", "out"]);
    let want = std::path::Path::new("out").join("ATLANTIS.PUN").display().to_string();
    assert!(got.contains(&want) && got.contains("(text"), "{got}");

    ok(&dir, &["put", "ATLANTIS.PUN", "formatted.floppy.yaml:A:", "-f", "binary"]);
    let out = ok(&dir, &["dir", "formatted.floppy.yaml:A"]);
    assert!(out.contains("ATLANTIS.BAS") && out.contains("BASIC"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejects_a_newer_format_version() {
    let dir = scratch("version");
    let text = std::fs::read_to_string(dir.join("blank.floppy.yaml")).unwrap();
    std::fs::write(dir.join("v2.floppy.yaml"), text.replace("format-version: 1", "format-version: 2")).unwrap();
    assert!(fails(&dir, &["dir", "v2.floppy.yaml"]).contains("unsupported floppy format-version 2"));
    let _ = std::fs::remove_dir_all(&dir);
}
