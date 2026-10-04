//! `dir`, `del`, and the disk-image forms of `get`/`put`: files on a Calc-U-1600
//! CE-1600F floppy image (`<image>.floppy.yaml:<A|B>:<NAME.EXT>`).
//!
//! Glue only: the container is [`crate::floppy_image`], the filesystem [`crate::diskfs`],
//! and every conversion [`crate::transfer`] — the same rules serial `get`/`put` use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::detokenize::LineEnding;
use crate::diskfs::{DirEntry, DiskError, DosTimestamp, FileName, Pattern, Volume};
use crate::floppy_image::{self, FloppyImage, Side, FILE_SUFFIX};
use crate::header::{self, FileType};
use crate::registry::Device;
use crate::transfer::{self, DiskFileKind, Endpoint, Format, GetSpec, PutKind, PutSpec};
use crate::verbosity::narrate;

/// A parsed `<image>[:<side>[:<name>]]` argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageAddr {
    pub image: PathBuf,
    pub side: Option<Side>,
    /// File name or wildcard pattern; `None` if empty.
    pub name: Option<String>,
}

impl ImageAddr {
    fn require_side(&self, arg: &str) -> Result<Side> {
        self.side.ok_or_else(|| anyhow::anyhow!("{arg}: give the disk side, e.g. {}:A:", self.image.display()))
    }
}

/// Recognize an image address: the path up to and including `.floppy.yaml`
/// (case-insensitive), then optionally `:A`/`:B`, then optionally `:` and a name. Returns
/// `None` if `arg` is not an image address at all (e.g. a plain host file), and an error
/// if it is one but malformed. Windows drive letters (`C:\…`) are fine, since the image
/// part is found by its suffix, not by splitting on `:`.
pub fn parse_image_addr(arg: &str) -> Result<Option<ImageAddr>> {
    let lower = arg.to_ascii_lowercase();
    let Some(at) = lower.rfind(FILE_SUFFIX) else {
        return Ok(None);
    };
    let end = at + FILE_SUFFIX.len();
    let (image, rest) = arg.split_at(end);
    if rest.is_empty() {
        return Ok(Some(ImageAddr { image: image.into(), side: None, name: None }));
    }
    let Some(rest) = rest.strip_prefix(':') else {
        return Ok(None); // e.g. "x.floppy.yaml.bak"
    };
    let (side, name) = match rest.split_once(':') {
        Some((s, n)) => (s, n),
        None => (rest, ""),
    };
    let side = Side::from_str_ci(side)
        .ok_or_else(|| anyhow::anyhow!("{arg}: disk side must be A or B, got {side:?}"))?;
    let name = (!name.is_empty()).then(|| name.to_string());
    Ok(Some(ImageAddr { image: image.into(), side: Some(side), name }))
}

fn load(image: &Path) -> Result<FloppyImage> {
    floppy_image::read_path(image)
}

fn save(image: &Path, img: &FloppyImage) -> Result<()> {
    floppy_image::write_path_atomic(image, &floppy_image::format(img)?)
}

// ── dir ─────────────────────────────────────────────────────────────────

/// `sde dir <image>[:<side>[:<pattern>]]`
pub fn run_dir(target: &str) -> Result<String> {
    let addr = parse_image_addr(target)?
        .ok_or_else(|| anyhow::anyhow!("{target}: not a disk image (expected <file>{FILE_SUFFIX}[:A|:B])"))?;
    let img = load(&addr.image)?;
    let pattern = addr.name.as_deref().map(Pattern::parse).transpose()?;
    let sides: Vec<Side> = match addr.side {
        Some(s) => vec![s],
        None => Side::BOTH.to_vec(),
    };
    let mut out = String::new();
    for (i, side) in sides.into_iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("Disk \"{}\" ({}), side {side}\n", img.disk_name, addr.image.display()));
        let vol = match Volume::open_floppy_side(img.side(side)) {
            Ok(v) => v,
            Err(DiskError::NotFormatted) => {
                out.push_str("  not formatted\n");
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let entries: Vec<DirEntry> = match &pattern {
            Some(p) => vol.glob(p),
            None => vol.list(),
        };
        for e in &entries {
            let kind = match vol.read(e) {
                Ok(bytes) => describe_kind(transfer::classify_disk_file(&bytes)),
                Err(err) => format!("? ({err})"),
            };
            out.push_str(&format!(
                "  {:<12} {:>6}  {}  {}  {}\n",
                e.name.to_string(),
                e.size,
                e.timestamp(),
                e.attr_letters(),
                kind
            ));
        }
        out.push_str(&format!(
            "  {} file{}, {} bytes free\n",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" },
            vol.free_bytes()
        ));
    }
    Ok(out.trim_end().to_string())
}

fn describe_kind(kind: DiskFileKind) -> String {
    match kind {
        DiskFileKind::Basic { .. } => "BASIC".into(),
        DiskFileKind::Machine { load, run, .. } => {
            if !transfer::is_autorun(run) {
                format!("machine, load {load:06X}")
            } else {
                format!("machine, load {load:06X}, run {run:06X}")
            }
        }
        DiskFileKind::AsciiBasic => "BASIC (ASCII)".into(),
        DiskFileKind::Text => "text".into(),
        DiskFileKind::Unknown => "unknown".into(),
    }
}

// ── get ─────────────────────────────────────────────────────────────────

pub struct DiskGetOptions {
    pub format: Option<Format>,
    /// `-f wav`: write each file as a cassette WAV (PC-1600 / CE-1600P format).
    pub tape: Option<crate::wav_cmd::TapeOptions>,
    pub skip_header: bool,
    pub raw: bool,
    pub eol: LineEnding,
    pub dry_run: bool,
    pub verbose: bool,
}

/// `sde get <image>:<side>:<name|pattern> [output]`
pub fn run_get_disk(arg: &str, addr: &ImageAddr, output: Option<&str>, o: &DiskGetOptions) -> Result<String> {
    let side = addr.require_side(arg)?;
    let Some(name) = addr.name.as_deref() else {
        bail!("{arg}: give the file to get, e.g. {}:{side}:PROG.BAS", addr.image.display());
    };
    let img = load(&addr.image)?;
    let vol = Volume::open_floppy_side(img.side(side))?;

    let wildcard = Pattern::is_wildcard(name);
    let entries = vol.matching(name)?;

    let out_dir = match output {
        Some(p) if Path::new(p).is_dir() => Some(PathBuf::from(p)),
        Some(p) if wildcard => bail!("{p}: with a wildcard the output must be an existing directory"),
        Some(_) => None,
        None => Some(PathBuf::from(".")),
    };

    let mut msgs = Vec::new();
    for e in &entries {
        let stored = vol.read(e)?;
        if let Some(tape) = &o.tape {
            let files = crate::wav_cmd::tape_files_from(
                &stored,
                &e.name.stem(),
                Device::Pc1600,
                None,
                None,
                tape,
            )
            .with_context(|| e.name.to_string())?;
            let path = match &out_dir {
                Some(dir) => dir.join(format!("{}.wav", e.name.stem())),
                None => crate::wav_cmd::wav_output(output, PathBuf::new()),
            };
            msgs.push(crate::wav_cmd::write_tape(
                &files, &path, tape, o.dry_run, o.verbose,
            )?);
            continue;
        }
        let (bytes, host_name, what) = if o.raw {
            (stored, e.name.to_string(), "unchanged".to_string())
        } else {
            let spec = GetSpec { format: o.format, skip_header: o.skip_header, eol: o.eol };
            let x = transfer::extract(&stored, &spec).with_context(|| e.name.to_string())?;
            for note in &x.notes {
                eprintln!("WARNING: {}: {note}", e.name);
            }
            let name = host_name_for(&e.name, &x);
            (x.bytes, name, x.content.describe().to_string())
        };
        let path = match &out_dir {
            Some(dir) => dir.join(&host_name),
            None => PathBuf::from(output.expect("out_dir is None only with an output file")),
        };
        narrate(o.verbose, format!("{}: {} bytes on disk, {what}", e.name, e.size));
        msgs.push(crate::paths::write_got_file(&e.name.to_string(), &path, &bytes, &what, o.dry_run)?);
    }
    Ok(msgs.join("\n"))
}

/// Host file name for a file got from the disk: the stored stem, and for BASIC the host
/// extension of what is written (`.bas` listing, `.bbas` tokenized; on the disk both are
/// `.BAS`), `.bin` for machine code without an extension; otherwise the stored name
/// unchanged.
fn host_name_for(name: &FileName, x: &transfer::Extracted) -> String {
    let file_type = x.header.as_ref().map(|h| h.file_type);
    let ext = match file_type {
        Some(FileType::Basic) => x.ext.to_string(),
        None if x.content == crate::detect::Content::AsciiBasic => x.ext.to_string(),
        Some(FileType::Machine) if name.ext().is_empty() => crate::filename::MACHINE_EXT.to_string(),
        _ => name.ext(),
    };
    if ext.is_empty() {
        name.stem()
    } else {
        format!("{}.{ext}", name.stem())
    }
}

// ── put ─────────────────────────────────────────────────────────────────

pub struct DiskPutOptions {
    pub format: Option<Format>,
    pub start_address: Option<u32>,
    pub run_address: Option<u32>,
    pub raw: bool,
    pub force: bool,
    pub dry_run: bool,
    pub verbose: bool,
}

/// `sde put <file>... <image>:<side>:[NAME.EXT]`
pub fn run_put_disk(inputs: &[String], arg: &str, addr: &ImageAddr, o: &DiskPutOptions) -> Result<String> {
    let side = addr.require_side(arg)?;
    if addr.name.is_some() && inputs.len() > 1 {
        bail!("{arg}: a target file name can only be given for a single input file");
    }
    if let Some(n) = &addr.name {
        if Pattern::is_wildcard(n) {
            bail!("{arg}: wildcards are not allowed in a put target");
        }
    }
    let hint = "name the file on the disk explicitly, e.g. <image>:A:NAME.EXT";
    let puts = prepare_disk_puts(inputs, addr.name.as_deref(), hint, o)?;
    let mut img = load(&addr.image)?;
    let mut side_bytes = img.side(side).to_vec();
    let mut vol = Volume::open_floppy_side(&mut side_bytes[..])?;
    let ts = now_timestamp();

    let mut msgs = Vec::new();
    for p in &puts {
        vol.write(&p.name, &p.out.bytes, ts, o.force)?;
        msgs.push(p.report());
    }
    let free = vol.free_bytes();

    if o.dry_run {
        return Ok(format!("{}\nDry run: {} not changed", msgs.join("\n"), addr.image.display()));
    }
    *img.side_mut(side) = side_bytes;
    save(&addr.image, &img)?;
    msgs.push(format!("{} side {side}: {free} bytes free", addr.image.display()));
    Ok(msgs.join("\n"))
}

/// `sde put <file>... <dir>`: a host folder used as a PC-1600 disk, such as Calc-U-1600's
/// host drive (`S3:`). Files are stored as on a floppy (8.3 names, BASIC tokenized as
/// `NAME.BAS`); new names are created upper-case, an existing file keeps its host
/// spelling.
pub fn run_put_dir(inputs: &[String], dir: &Path, o: &DiskPutOptions) -> Result<String> {
    let puts = prepare_disk_puts(inputs, None, "rename it to an 8.3 name (NAME.EXT) first", o)?;
    let mut msgs = Vec::new();
    for p in &puts {
        let path = host_disk_path(dir, &p.name, o.force)?;
        if !o.dry_run {
            if path.exists() {
                // --force replaces a write-protected file too, as on a floppy.
                std::fs::remove_file(&path).with_context(|| format!("cannot replace {}", path.display()))?;
            }
            std::fs::write(&path, &p.out.bytes).with_context(|| format!("cannot write {}", path.display()))?;
        }
        msgs.push(p.report());
    }
    if o.dry_run {
        msgs.push(format!("Dry run: {} not changed", dir.display()));
    }
    Ok(msgs.join("\n"))
}

/// Where `name` goes in `dir`: an existing file whose name matches it ignoring case (an
/// error without `force`), else `dir/NAME.EXT`.
fn host_disk_path(dir: &Path, name: &FileName, force: bool) -> Result<PathBuf> {
    let wanted = name.to_string();
    let mut existing: Option<PathBuf> = None;
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(n) = file_name.to_str() else { continue };
        if n.eq_ignore_ascii_case(&wanted) && entry.file_type()?.is_file() {
            // The exact upper-case spelling wins, as on Calc-U-1600's host drive.
            if existing.is_none() || n == wanted {
                existing = Some(entry.path());
            }
        }
    }
    let Some(path) = existing else { return Ok(dir.join(wanted)) };
    if !force {
        let protected = std::fs::metadata(&path)?.permissions().readonly();
        let e = if protected { DiskError::Protected(wanted) } else { DiskError::Exists(wanted) };
        return Err(anyhow::Error::from(e).context(path.display().to_string()));
    }
    Ok(path)
}

/// One input file, converted for a PC-1600 disk.
struct DiskPut {
    input: String,
    name: FileName,
    out: transfer::PutBytes,
}

impl DiskPut {
    fn report(&self) -> String {
        format!("{} -> {} ({}, {} bytes)", self.input, self.name, describe_put(self.out.kind), self.out.bytes.len())
    }
}

/// Convert every input for a PC-1600 disk (floppy side or host folder) and pick its name
/// there: `name` if given (one input only), else the host name per
/// [`transfer::disk_ext_for`]. `name_hint` says how to fix a host name that isn't 8.3.
fn prepare_disk_puts(
    inputs: &[String],
    name: Option<&str>,
    name_hint: &str,
    o: &DiskPutOptions,
) -> Result<Vec<DiskPut>> {
    let mut puts: Vec<DiskPut> = Vec::new();
    for input in inputs {
        let (raw, tape_name) = crate::wav_cmd::read_put_input(input, o.verbose)?;
        let input = tape_name.unwrap_or_else(|| input.clone());
        let h = header::find(&raw);
        let content = crate::detect::detect_from_header(h.as_ref(), &raw);
        narrate(o.verbose, format!("{input}: detected {}", content.describe()));
        let spec = PutSpec {
            source_name: &input,
            device: Device::Pc1600,
            format: o.format,
            start_address: o.start_address,
            run_address: o.run_address,
            raw: o.raw,
            endpoint: Endpoint::Disk,
        };
        let out = transfer::build_put(&raw, h.as_ref(), content, &spec).with_context(|| input.clone())?;
        let name = match name {
            Some(n) => FileName::parse(n)?,
            None => default_disk_name(&input, out.kind, &out.bytes, name_hint)?,
        };
        if puts.iter().any(|p| p.name == name) {
            bail!("{input}: {name} is the target of more than one input file");
        }
        let tokenized_basic = match out.kind {
            PutKind::TokenizedBasic => true,
            PutKind::AsIs => matches!(transfer::classify_disk_file(&out.bytes), DiskFileKind::Basic { .. }),
            _ => false,
        };
        if tokenized_basic {
            narrate(o.verbose, format!("{name} holds tokenized BASIC; the PC-1600 names BASIC .BAS in both forms"));
        }
        puts.push(DiskPut { input, name, out });
    }
    Ok(puts)
}

fn describe_put(kind: PutKind) -> &'static str {
    match kind {
        PutKind::AsIs => "stored as-is",
        PutKind::TokenizedBasic => "tokenized BASIC",
        PutKind::AsciiListing => "BASIC as ASCII",
        PutKind::MachineWrapped => "machine language, header added",
        PutKind::Text => "text",
        PutKind::Raw => "raw",
        PutKind::Reserve | PutKind::Variables => "PC-1500 data",
    }
}

/// Host stem upper-cased, plus [`transfer::disk_ext_for`].
fn default_disk_name(input: &str, kind: PutKind, bytes: &[u8], hint: &str) -> Result<FileName> {
    let p = Path::new(input);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let host_ext = p.extension().and_then(|s| s.to_str());
    let ext = transfer::disk_ext_for(kind, bytes, host_ext);
    let name = if ext.is_empty() { stem.to_string() } else { format!("{stem}.{ext}") };
    FileName::parse(&name).map_err(|e| anyhow::anyhow!("{input}: {e}; {hint}"))
}

fn now_timestamp() -> DosTimestamp {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    DosTimestamp::new(now.month() as u8, now.day() as u8, now.hour() as u8, now.minute() as u8, now.second() as u8)
}

// ── del ─────────────────────────────────────────────────────────────────

/// `sde del <image>:<side>:<name|pattern>...` — several targets, several images.
pub fn run_del(targets: &[String], force: bool, dry_run: bool, verbose: bool) -> Result<String> {
    // Group by image (keeping order) so each image is read and written once.
    let mut by_image: BTreeMap<PathBuf, Vec<(String, Side, String)>> = BTreeMap::new();
    for t in targets {
        let addr = parse_image_addr(t)?
            .ok_or_else(|| anyhow::anyhow!("{t}: not a disk image address (<file>{FILE_SUFFIX}:A:NAME)"))?;
        let side = addr.require_side(t)?;
        let name = addr.name.clone().ok_or_else(|| anyhow::anyhow!("{t}: give the file(s) to delete"))?;
        by_image.entry(addr.image).or_default().push((t.clone(), side, name));
    }

    let mut msgs = Vec::new();
    for (image, items) in by_image {
        let mut img = load(&image)?;
        for (arg, side, name) in items {
            let mut bytes = img.side(side).to_vec();
            let mut vol = Volume::open_floppy_side(&mut bytes[..])?;
            let entries = match vol.matching(&name) {
                Err(e @ DiskError::NotFound(_)) => bail!("{arg}: {e}"),
                r => r?,
            };
            for e in &entries {
                vol.delete(e, force)?;
                narrate(verbose, format!("{}: freed {} bytes", e.name, e.size));
                msgs.push(format!("Deleted {} from {} side {side}", e.name, image.display()));
            }
            *img.side_mut(side) = bytes;
        }
        if !dry_run {
            save(&image, &img)?;
        }
    }
    if dry_run {
        msgs.push("Dry run: no image changed".into());
    }
    Ok(msgs.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(image: &str, side: Option<Side>, name: Option<&str>) -> Option<ImageAddr> {
        Some(ImageAddr { image: image.into(), side, name: name.map(str::to_string) })
    }

    #[test]
    fn image_addresses() {
        let p = |s| parse_image_addr(s).unwrap();
        assert_eq!(p("d.floppy.yaml"), addr("d.floppy.yaml", None, None));
        assert_eq!(p("d.floppy.yaml:a"), addr("d.floppy.yaml", Some(Side::A), None));
        assert_eq!(p("d.floppy.yaml:B:"), addr("d.floppy.yaml", Some(Side::B), None));
        assert_eq!(p("dir/D.Floppy.YAML:A:prog.bas"), addr("dir/D.Floppy.YAML", Some(Side::A), Some("prog.bas")));
        assert_eq!(
            p(r"C:\disks\d.floppy.yaml:A:*.BAS"),
            addr(r"C:\disks\d.floppy.yaml", Some(Side::A), Some("*.BAS"))
        );
        assert_eq!(p("prog.bas"), None);
        assert_eq!(p("C:\\prog.bas"), None);
        assert_eq!(p("d.floppy.yaml.bak"), None);
        assert!(parse_image_addr("d.floppy.yaml:C:X").is_err());
    }

    #[test]
    fn host_names() {
        let n = |s| FileName::parse(s).unwrap();
        let x = |bytes: &[u8], format| {
            transfer::extract(bytes, &GetSpec { format, skip_header: false, eol: LineEnding::Lf }).unwrap()
        };
        let mut basic = header::build(Device::Pc1600, None, 4);
        basic.extend_from_slice(&[0x00, 0x0A, 0x01, 0x0D]);
        assert_eq!(host_name_for(&n("GLOBUS.BAS"), &x(&basic, None)), "GLOBUS.bas");
        assert_eq!(host_name_for(&n("GLOBUS.BAS"), &x(&basic, Some(Format::Binary))), "GLOBUS.bbas");
        assert_eq!(host_name_for(&n("SYS.CFG"), &x(b"A=1\r\n\x1A", None)), "SYS.CFG");
        assert_eq!(host_name_for(&n("README"), &x(b"hello\r\n\x1A", None)), "README");
        assert_eq!(host_name_for(&n("OLD"), &x(b"10 PRINT\r\n\x1A", None)), "OLD.bas");
        assert_eq!(host_name_for(&n("OLD"), &x(b"10 PRINT\r\n\x1A", Some(Format::Binary))), "OLD.bas");
    }

    #[test]
    fn put_to_a_host_folder() {
        let d = std::env::temp_dir().join(format!("sde-put-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s3 = d.join("s3");
        std::fs::create_dir_all(&s3).unwrap();
        let src = d.join("prog.bas");
        std::fs::write(&src, "10 PRINT \"HI\"\n").unwrap();
        let inputs = [src.to_string_lossy().to_string()];
        let o = |format, force, dry_run| DiskPutOptions {
            format,
            start_address: None,
            run_address: None,
            raw: false,
            force,
            dry_run,
            verbose: false,
        };

        // Dry run writes nothing; then a new file is tokenized, upper-case.
        run_put_dir(&inputs, &s3, &o(None, false, true)).unwrap();
        assert_eq!(std::fs::read_dir(&s3).unwrap().count(), 0);
        run_put_dir(&inputs, &s3, &o(None, false, false)).unwrap();
        let stored = std::fs::read(s3.join("PROG.BAS")).unwrap();
        assert!(matches!(transfer::classify_disk_file(&stored), DiskFileKind::Basic { .. }));

        // An existing file (any case) needs --force and keeps its host spelling.
        std::fs::rename(s3.join("PROG.BAS"), s3.join("prog.bas")).unwrap();
        let e = run_put_dir(&inputs, &s3, &o(None, false, false)).unwrap_err();
        assert!(matches!(e.downcast_ref::<DiskError>(), Some(DiskError::Exists(_))), "{e:#}");
        run_put_dir(&inputs, &s3, &o(Some(Format::Ascii), true, false)).unwrap();
        let names: Vec<_> = std::fs::read_dir(&s3).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["prog.bas"]);
        // -f ascii stores the listing (CP437, CRLF, 1A).
        assert_eq!(std::fs::read(s3.join("prog.bas")).unwrap(), b"10 PRINT \"HI\"\r\n\x1A");
        let _ = std::fs::remove_dir_all(&d);
    }
}
