//! CLI-only filesystem glue for the `convert` verb: extension rules, output-path
//! derivation, file read/write. Ported from `SharpDataExchange.runConvert` /
//! `deriveConvertOutput` / `appendBasIfMissing`, plus the machine-code header add/strip.
//! Not reachable from the library.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::detect::Content;
use crate::detokenize::LineEnding;
use crate::header;
use crate::registry::Device;
use crate::scanner::SegmentMarker;
use crate::verbosity::narrate;

use crate::filename::{BASIC_ASCII_EXT as ASCII_EXT, BASIC_BINARY_EXT as TOKENIZED_EXT, MACHINE_EXT};
/// Output tags for a machine-code convert: `NAME.pure.bin` (header stripped),
/// `NAME.ce158.bin` / `NAME.pc1600.bin` (header added).
const PURE_TAG: &str = "pure";
const CE158_TAG: &str = "ce158";
const PC1600_TAG: &str = "pc1600";

/// Options for [`run_convert`].
pub struct ConvertOptions {
    /// Keyword table + header flavor when tokenizing or adding a machine header.
    pub device: Device,
    /// Line ending of a de-tokenized listing.
    pub eol: LineEnding,
    /// `--start-address`: treat the (headerless) input as machine code and add a
    /// MACHINE header with this load address.
    pub start_address: Option<u32>,
    /// `--run-address`: auto-start address for the added header.
    pub run_address: Option<u32>,
    pub verbose: bool,
}

/// Run one `convert`: read `infile`, convert, write the derived (or given) output file.
/// Returns a one-line human summary.
///
/// * ASCII BASIC -> tokenized (`.bbas`); tokenized BASIC -> listing (`.bas`). Tokenized
///   BASIC named `.bas` (as on a PC-1600 disk) whose listing would overwrite it is
///   renamed to `.bbas` first.
/// * Machine code with a header -> the bare payload (`NAME.pure.bin`).
/// * Headerless input with `--start-address` -> machine code behind an added header
///   (`NAME.ce158.bin` / `NAME.pc1600.bin`).
///
/// The input is recognized by content; its extension is only narrated when it disagrees.
/// The output never overwrites the input file.
pub fn run_convert(infile: &str, outfile: Option<&str>, opts: &ConvertOptions) -> Result<String> {
    let in_path = append_bas_if_missing(infile);
    let raw = std::fs::read(&in_path)
        .with_context(|| format!("cannot read {}", in_path.display()))?;
    if raw.is_empty() {
        bail!("{} is empty", in_path.display());
    }
    narrate(opts.verbose, format!("Read {} bytes from {}", raw.len(), in_path.display()));

    let content = crate::detect::detect(&raw);
    narrate(opts.verbose, format!("Detected content: {}", content.describe()));

    if let Some(start) = opts.start_address {
        return add_header(&in_path, outfile, &raw, content, start, opts);
    }
    note_extension_mismatch(&in_path, content, opts.verbose);

    let (target_ext, tokenizing) = match content {
        Content::AsciiBasic => (TOKENIZED_EXT, true),
        Content::Ce158Basic | Content::Pc1600Basic => (ASCII_EXT, false),
        Content::Ce158Machine | Content::Pc1600Machine => {
            return strip_header(&in_path, outfile, &raw, opts);
        }
        Content::Text | Content::Unknown => bail!(
            "{} has no header and is not a BASIC listing ({}). To add a machine-language \
             header, give --start-address (and optionally --run-address)",
            in_path.display(),
            content.describe()
        ),
        Content::Ce158Reserve | Content::Ce158Variables => bail!(
            "convert handles BASIC and machine language only; got {}",
            content.describe()
        ),
    };

    let name = in_path.file_stem().and_then(|s| s.to_str());
    if tokenizing {
        narrate(
            opts.verbose,
            format!(
                "Tokenizing with the {:?} keyword table, adding a {} BASIC header",
                opts.device,
                opts.device.header_name()
            ),
        );
    } else {
        narrate(opts.verbose, "De-tokenizing; keyword table taken from the header");
    }
    let outcome =
        crate::convert::convert_with(&raw, opts.device, name, true, opts.eol, SegmentMarker::Wire)?;
    let out_path = derive_convert_output(outfile, &in_path, target_ext)?;
    // Tokenized `PROG.BAS` (a PC-1600 disk's naming): its listing's default name is the
    // input itself, so the input moves aside to `PROG.bbas`, sde's name for it.
    let renamed = if !tokenizing && outfile.is_none() && same_file(&in_path, &out_path) {
        let aside = in_path.with_extension(TOKENIZED_EXT);
        if aside.exists() {
            bail!(
                "{} holds tokenized BASIC, and its listing would overwrite it; {} exists, so it \
                 can't be renamed there. Give an output file",
                in_path.display(),
                aside.display()
            );
        }
        narrate(
            opts.verbose,
            format!(
                "{} holds tokenized BASIC under .{ASCII_EXT} (as a PC-1600 disk stores it); renaming it \
                 to {} so the listing can take its name",
                in_path.display(),
                aside.display()
            ),
        );
        std::fs::rename(&in_path, &aside)
            .with_context(|| format!("cannot rename {} to {}", in_path.display(), aside.display()))?;
        Some(aside)
    } else {
        None
    };
    if let Err(e) = write_output(renamed.as_deref().unwrap_or(&in_path), &out_path, &outcome.bytes, opts.verbose) {
        if let Some(aside) = &renamed {
            let _ = std::fs::rename(aside, &in_path);
        }
        return Err(e);
    }

    Ok(if tokenizing {
        format!(
            "Converted {} -> {} (tokenized, {:?})",
            in_path.display(),
            out_path.display(),
            opts.device
        )
    } else {
        let note = renamed.map(|a| format!("; input renamed to {}", a.display())).unwrap_or_default();
        format!(
            "Converted {} -> {} (ASCII, {:?}{note})",
            in_path.display(),
            out_path.display(),
            outcome.device
        )
    })
}

/// Headerless input + `--start-address`: wrap it in a MACHINE header.
fn add_header(
    in_path: &Path,
    outfile: Option<&str>,
    raw: &[u8],
    content: Content,
    start: u32,
    opts: &ConvertOptions,
) -> Result<String> {
    if let Some(h) = header::find(raw) {
        bail!(
            "{} already has a {} header; convert it without --start-address first to strip it",
            in_path.display(),
            h.device.header_name()
        );
    }
    if content == Content::AsciiBasic {
        narrate(opts.verbose, "Input looks like a BASIC listing; treating it as machine code anyway (--start-address)");
    } else {
        narrate(opts.verbose, "No header; treating the input as machine code (--start-address)");
    }

    let device = opts.device;
    let name = base_stem(in_path).to_ascii_uppercase();
    let bytes =
        crate::convert::add_machine_header(raw, device, Some(&name), start, opts.run_address)?;
    let h = header::find(&bytes).expect("add_machine_header writes a header");
    let w = device.addr_hex_width();
    let run_note = if opts.run_address.is_some() { "" } else { " (no auto-start)" };
    narrate(
        opts.verbose,
        format!(
            "Adding {} MACHINE header ({} bytes): {}load=0x{:0w$X} run=0x{:0w$X}{run_note}, payload {} bytes",
            device.header_name(),
            h.header_len,
            if device == Device::Pc1500 { format!("name={name} ") } else { String::new() },
            h.start_addr,
            h.run_addr,
            raw.len(),
        ),
    );

    let tag = header_tag(device);
    let out_path = derive_machine_output(outfile, in_path, tag)?;
    write_output(in_path, &out_path, &bytes, opts.verbose)?;
    Ok(format!(
        "Converted {} -> {} ({} header added, load=0x{:0w$X})",
        in_path.display(),
        out_path.display(),
        device.header_name(),
        h.start_addr
    ))
}

/// Machine code with a header: write the bare payload.
fn strip_header(in_path: &Path, outfile: Option<&str>, raw: &[u8], opts: &ConvertOptions) -> Result<String> {
    let s = crate::convert::strip_machine_header(raw)?;
    let h = &s.header;
    let w = h.device.addr_hex_width();
    narrate(
        opts.verbose,
        format!(
            "Found {} MACHINE header at offset {} ({} bytes): {}load=0x{:0w$X} run=0x{:0w$X}, payload {} bytes",
            h.device.header_name(),
            h.offset,
            h.header_len,
            h.filename.as_ref().map(|n| format!("name={n} ")).unwrap_or_default(),
            h.start_addr,
            h.run_addr,
            h.length,
        ),
    );
    narrate(opts.verbose, "Stripping the header (load/run addresses are not kept)");
    if s.trailing > 0 {
        narrate(opts.verbose, format!("Dropping {} trailing bytes past the payload", s.trailing));
    }

    let out_path = derive_machine_output(outfile, in_path, PURE_TAG)?;
    write_output(in_path, &out_path, s.payload, opts.verbose)?;
    Ok(format!(
        "Converted {} -> {} ({} header stripped, load=0x{:0w$X} run=0x{:0w$X})",
        in_path.display(),
        out_path.display(),
        h.device.header_name(),
        h.start_addr,
        h.run_addr
    ))
}

fn write_output(in_path: &Path, out_path: &Path, bytes: &[u8], verbose: bool) -> Result<()> {
    if same_file(in_path, out_path) {
        bail!(
            "output {} would overwrite the input file; give a different output file",
            out_path.display()
        );
    }
    std::fs::write(out_path, bytes).with_context(|| format!("cannot write {}", out_path.display()))?;
    narrate(verbose, format!("Wrote {} bytes to {}", bytes.len(), out_path.display()));
    Ok(())
}

/// Write one file got from a disk image or tape (`label` names it there), or under
/// `dry_run` only say what would be written. Returns the line to report.
pub(crate) fn write_got_file(label: &str, path: &Path, bytes: &[u8], what: &str, dry_run: bool) -> Result<String> {
    if dry_run {
        return Ok(format!("Dry run: would write {} bytes to {} ({what})", bytes.len(), path.display()));
    }
    std::fs::write(path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(format!("{label} -> {} ({what}, {} bytes)", path.display(), bytes.len()))
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn header_tag(device: Device) -> &'static str {
    match device {
        Device::Pc1500 => CE158_TAG,
        Device::Pc1600 => PC1600_TAG,
    }
}

/// The input's file stem without a header tag this verb adds (`prog.pure.bin`,
/// `prog.ce158.bin` -> `prog`), so add/strip round-trips don't stack tags.
fn base_stem(p: &Path) -> &str {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    for tag in [PURE_TAG, CE158_TAG, PC1600_TAG] {
        if let Some((base, t)) = stem.rsplit_once('.') {
            if t.eq_ignore_ascii_case(tag) && !base.is_empty() {
                return base;
            }
        }
    }
    stem
}

/// Output for a machine-code add/strip: the given file (`.bin` appended if it has no
/// extension; `.bas`/`.bbas` rejected), else `<dir>/<base>.<tag>.bin`.
fn derive_machine_output(outfile: Option<&str>, in_path: &Path, tag: &str) -> Result<PathBuf> {
    match outfile {
        Some(given) => {
            let p = PathBuf::from(crate::filename::append_ext_if_missing(given, MACHINE_EXT));
            if let Some(e @ (ASCII_EXT | TOKENIZED_EXT)) = ext_of(&p).as_deref() {
                bail!("output {} has extension .{e}, which is for BASIC", p.display());
            }
            Ok(p)
        }
        None => Ok(in_path.with_file_name(format!("{}.{tag}.{MACHINE_EXT}", base_stem(in_path)))),
    }
}

/// If the final path segment has no `.`, append `.bas`.
fn append_bas_if_missing(file: &str) -> PathBuf {
    PathBuf::from(crate::filename::append_ext_if_missing(file, ASCII_EXT))
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase())
}

/// The content decides, not the extension; say so when `.bas` / `.bbas` disagrees with it.
fn note_extension_mismatch(in_path: &Path, content: Content, verbose: bool) {
    let tokenized = matches!(content, Content::Ce158Basic | Content::Pc1600Basic);
    let named = match ext_of(in_path).as_deref() {
        Some(TOKENIZED_EXT) if !tokenized => TOKENIZED_EXT,
        Some(ASCII_EXT) if content != Content::AsciiBasic => ASCII_EXT,
        _ => return,
    };
    narrate(
        verbose,
        format!("{} is named *.{named} but holds {}; going by its content", in_path.display(), content.describe()),
    );
}

fn derive_convert_output(outfile: Option<&str>, in_path: &Path, target_ext: &str) -> Result<PathBuf> {
    match outfile {
        Some(given) => {
            let p = Path::new(given);
            if p.file_name().and_then(|s| s.to_str()).is_some_and(|n| !n.contains('.')) {
                return Ok(PathBuf::from(crate::filename::append_ext_if_missing(given, target_ext)));
            }
            match ext_of(p).as_deref() {
                Some(e @ (ASCII_EXT | TOKENIZED_EXT)) if e != target_ext => bail!(
                    "output {} has extension .{e} but this conversion produces .{target_ext}",
                    p.display()
                ),
                _ => Ok(p.to_path_buf()),
            }
        }
        None => Ok(in_path.with_extension(target_ext)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_bas() {
        assert_eq!(append_bas_if_missing("prog"), Path::new("prog.bas"));
        assert_eq!(append_bas_if_missing("prog.bbas"), Path::new("prog.bbas"));
        assert_eq!(append_bas_if_missing("dir/prog"), Path::new("dir/prog.bas"));
    }

    #[test]
    fn derive_output() {
        let inp = Path::new("a/prog.bas");
        assert_eq!(derive_convert_output(None, inp, "bbas").unwrap(), Path::new("a/prog.bbas"));
        assert_eq!(
            derive_convert_output(Some("out"), inp, "bbas").unwrap(),
            Path::new("out.bbas")
        );
        assert_eq!(
            derive_convert_output(Some("out.x"), inp, "bbas").unwrap(),
            Path::new("out.x")
        );
        assert!(derive_convert_output(Some("out.bas"), inp, "bbas").is_err());
    }

    #[test]
    fn machine_output_names() {
        let tagged = |f: &str, tag| derive_machine_output(None, Path::new(f), tag).unwrap();
        assert_eq!(tagged("a/prog.bin", CE158_TAG), Path::new("a/prog.ce158.bin"));
        assert_eq!(tagged("prog.bin", PC1600_TAG), Path::new("prog.pc1600.bin"));
        assert_eq!(tagged("prog.ce158.bin", PURE_TAG), Path::new("prog.pure.bin"));
        assert_eq!(tagged("prog.PC1600.bin", PURE_TAG), Path::new("prog.pure.bin"));
        assert_eq!(tagged("prog.pure.bin", CE158_TAG), Path::new("prog.ce158.bin"));
        assert_eq!(tagged("prog", PURE_TAG), Path::new("prog.pure.bin"));
        let given = |f| derive_machine_output(Some(f), Path::new("prog.bin"), PURE_TAG);
        assert_eq!(given("out").unwrap(), Path::new("out.bin"));
        assert_eq!(given("out.x").unwrap(), Path::new("out.x"));
        assert!(given("out.bas").is_err());
        assert!(given("out.bbas").is_err());
    }

    fn opts(start_address: Option<u32>) -> ConvertOptions {
        ConvertOptions {
            device: Device::Pc1500,
            eol: LineEnding::Lf,
            start_address,
            run_address: None,
            verbose: false,
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sde-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn machine_header_add_then_strip_roundtrips() {
        let d = scratch_dir("roundtrip");
        let code = [0xFD, 0xA8, 0xB5, 0x41, 0x9A];
        let bin = d.join("prog.bin");
        std::fs::write(&bin, code).unwrap();

        run_convert(bin.to_str().unwrap(), None, &opts(Some(0x38C5))).unwrap();
        let wrapped = std::fs::read(d.join("prog.ce158.bin")).unwrap();
        let h = header::find(&wrapped).unwrap();
        assert_eq!((h.file_type, h.start_addr, h.run_addr), (header::FileType::Machine, 0x38C5, 0xFFFF));
        assert_eq!(h.filename.as_deref(), Some("PROG"));

        run_convert(d.join("prog.ce158.bin").to_str().unwrap(), None, &opts(None)).unwrap();
        assert_eq!(std::fs::read(d.join("prog.pure.bin")).unwrap(), code);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn machine_convert_errors() {
        let d = scratch_dir("errors");
        let bin = d.join("prog.bin");
        std::fs::write(&bin, [0xFD, 0xA8, 0x00]).unwrap();
        let bin = bin.to_str().unwrap();

        // Headerless without --start-address.
        let e = run_convert(bin, None, &opts(None)).unwrap_err().to_string();
        assert!(e.contains("--start-address"), "{e}");
        // Output naming the input file.
        assert!(run_convert(bin, Some(bin), &opts(Some(0x1000))).is_err());
        assert_eq!(std::fs::read(bin).unwrap(), [0xFD, 0xA8, 0x00]);
        // Header already present.
        run_convert(bin, None, &opts(Some(0x1000))).unwrap();
        let wrapped = d.join("prog.ce158.bin");
        assert!(run_convert(wrapped.to_str().unwrap(), None, &opts(Some(0x1000))).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tokenized_bas_moves_aside_for_its_listing() {
        let d = scratch_dir("aside");
        let listing = d.join("prog.bas");
        std::fs::write(&listing, "10 PRINT 1\n").unwrap();
        run_convert(listing.to_str().unwrap(), None, &opts(None)).unwrap();
        // Tokenized BASIC under .bas, as a PC-1600 disk names it (lower-case, so the
        // listing's default name is this file on case-sensitive file systems too).
        let tokenized = listing;
        std::fs::rename(d.join("prog.bbas"), &tokenized).unwrap();
        let tokens = std::fs::read(&tokenized).unwrap();

        let msg = run_convert(tokenized.to_str().unwrap(), None, &opts(None)).unwrap();
        assert!(msg.contains("input renamed to"), "{msg}");
        assert_eq!(std::fs::read(d.join("prog.bbas")).unwrap(), tokens);
        assert_eq!(std::fs::read_to_string(d.join("prog.bas")).unwrap(), "10 PRINT 1\n");

        // With the .bbas name taken, it is an error and nothing changes.
        std::fs::write(&tokenized, &tokens).unwrap();
        assert!(run_convert(tokenized.to_str().unwrap(), None, &opts(None)).is_err());
        assert_eq!(std::fs::read(&tokenized).unwrap(), tokens);
        let _ = std::fs::remove_dir_all(&d);
    }
}
