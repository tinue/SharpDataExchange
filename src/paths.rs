//! CLI-only filesystem glue for the `convert` verb: extension rules, output-path
//! derivation, file read/write. Ported from `SharpDataExchange.runConvert` /
//! `deriveConvertOutput` / `appendBasIfMissing`. Not reachable from the library.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::detect::Content;
use crate::detokenize::LineEnding;
use crate::registry::Device;
use crate::scanner::SegmentMarker;

const ASCII_EXT: &str = "bas";
const TOKENIZED_EXT: &str = "bbin";

/// Run one `convert`: read `infile`, convert, write the derived (or given) output file.
/// Returns a one-line human summary. `eol` sets the line ending of a de-tokenized
/// listing (ignored when tokenizing).
pub fn run_convert(
    infile: &str,
    outfile: Option<&str>,
    device: Device,
    eol: LineEnding,
) -> Result<String> {
    let in_path = append_bas_if_missing(infile);
    let raw = std::fs::read(&in_path)
        .with_context(|| format!("cannot read {}", in_path.display()))?;
    if raw.is_empty() {
        bail!("{} is empty", in_path.display());
    }

    let content = crate::detect::detect(&raw);
    check_extension_matches_content(&in_path, content)?;

    let (target_ext, tokenizing) = match content {
        Content::AsciiBasic => (TOKENIZED_EXT, true),
        Content::Ce158Basic | Content::Pc1600Basic => (ASCII_EXT, false),
        Content::Ce158Machine
        | Content::Pc1600Machine
        | Content::Ce158Reserve
        | Content::Ce158Variables
        | Content::Unknown => bail!(
            "convert only handles BASIC; got {}. A tokenized file must include a CE-158 or PC-1600 header.",
            content.describe()
        ),
    };

    let name = in_path.file_stem().and_then(|s| s.to_str());
    let outcome = crate::convert::convert_with(&raw, device, name, true, eol, SegmentMarker::Wire)?;
    let out_path = derive_convert_output(outfile, &in_path, target_ext)?;
    std::fs::write(&out_path, &outcome.bytes)
        .with_context(|| format!("cannot write {}", out_path.display()))?;

    Ok(if tokenizing {
        format!(
            "Converted {} -> {} (tokenized, {:?})",
            in_path.display(),
            out_path.display(),
            device
        )
    } else {
        format!(
            "Converted {} -> {} (ASCII, {:?})",
            in_path.display(),
            out_path.display(),
            outcome.device
        )
    })
}

/// If the final path segment has no `.`, append `.bas`.
fn append_bas_if_missing(file: &str) -> PathBuf {
    PathBuf::from(crate::filename::append_ext_if_missing(file, ASCII_EXT))
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase())
}

/// The input extension must agree with its actual content: `.bbin` requires tokenized
/// BASIC, `.bas` requires an ASCII listing. Any other extension is unconstrained.
fn check_extension_matches_content(in_path: &Path, content: Content) -> Result<()> {
    match ext_of(in_path).as_deref() {
        Some(TOKENIZED_EXT) if !matches!(content, Content::Ce158Basic | Content::Pc1600Basic) => {
            bail!(
                "{} is named *.{TOKENIZED_EXT} but its content is {}",
                in_path.display(),
                content.describe()
            )
        }
        Some(ASCII_EXT) if content != Content::AsciiBasic => {
            bail!(
                "{} is named *.{ASCII_EXT} but its content is {}",
                in_path.display(),
                content.describe()
            )
        }
        _ => Ok(()),
    }
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
        assert_eq!(append_bas_if_missing("prog.bbin"), Path::new("prog.bbin"));
        assert_eq!(append_bas_if_missing("dir/prog"), Path::new("dir/prog.bas"));
    }

    #[test]
    fn derive_output() {
        let inp = Path::new("a/prog.bas");
        assert_eq!(derive_convert_output(None, inp, "bbin").unwrap(), Path::new("a/prog.bbin"));
        assert_eq!(
            derive_convert_output(Some("out"), inp, "bbin").unwrap(),
            Path::new("out.bbin")
        );
        assert_eq!(
            derive_convert_output(Some("out.x"), inp, "bbin").unwrap(),
            Path::new("out.x")
        );
        assert!(derive_convert_output(Some("out.bas"), inp, "bbin").is_err());
    }
}
