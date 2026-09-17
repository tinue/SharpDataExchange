//! Shared filename helpers for `get`/`put`, per `requirements-put-get.md` §3/§4.

use std::path::Path;

use crate::header::FileType;

/// Extension appended when an output filename (from the CLI or a header) has none:
/// `.bas` for BASIC, `.bin` for machine language.
pub fn ext_for(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Basic => "bas",
        FileType::Machine => "bin",
    }
}

/// Append `ext` to `name` if it has no extension (no `.` in the final path segment).
pub fn append_ext_if_missing(name: &str, ext: &str) -> String {
    let has_ext = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.contains('.'))
        .unwrap_or(false);
    if has_ext {
        name.to_string()
    } else {
        format!("{name}.{ext}")
    }
}

/// Derive the filename for a synthesized header from a `put` input file path: base name
/// (no extension), upper-cased, truncated to 16 characters — per requirements §3.
pub fn synth_basename(input_path: &str) -> String {
    let stem = Path::new(input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    upper.chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_for_types() {
        assert_eq!(ext_for(FileType::Basic), "bas");
        assert_eq!(ext_for(FileType::Machine), "bin");
    }

    #[test]
    fn append_ext_only_if_missing() {
        assert_eq!(append_ext_if_missing("prog", "bas"), "prog.bas");
        assert_eq!(append_ext_if_missing("prog.bin", "bas"), "prog.bin");
        assert_eq!(append_ext_if_missing("dir/prog", "bin"), "dir/prog.bin");
    }

    #[test]
    fn synth_basename_truncates_and_uppercases() {
        assert_eq!(synth_basename("myprogram.bin"), "MYPROGRAM");
        assert_eq!(synth_basename("dir/prog"), "PROG");
        assert_eq!(synth_basename("a-very-long-file-name.bin"), "A-VERY-LONG-FILE");
        assert_eq!(synth_basename(""), "");
    }
}
