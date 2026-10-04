//! Shared filename helpers for `get`/`put`.

use std::path::Path;

use crate::header::FileType;
use crate::transfer::Format;

/// BASIC listing.
pub const BASIC_ASCII_EXT: &str = "bas";
/// Tokenized BASIC (on a PC-1600 disk it is still `.BAS`, as the device stores it).
pub const BASIC_BINARY_EXT: &str = "bbas";
/// Machine code, and data that is not recognized.
pub const MACHINE_EXT: &str = "bin";

/// The host extension for a file of `file_type` written in `format`: `.bas` / `.bbas`
/// for BASIC, `.sdar` / `.bsdar` for Reserve Area, `.sdav` / `.bsdav` for Variables,
/// `.bin` for machine code. Input files are recognized by content, never by these.
pub fn ext_for(file_type: FileType, format: Format) -> &'static str {
    match (file_type, format) {
        (FileType::Basic, Format::Ascii) => BASIC_ASCII_EXT,
        (FileType::Basic, Format::Binary) => BASIC_BINARY_EXT,
        (FileType::Machine, _) => MACHINE_EXT,
        (FileType::Reserve, Format::Ascii) => "sdar",
        (FileType::Reserve, Format::Binary) => "bsdar",
        (FileType::Variables, Format::Ascii) => "sdav",
        (FileType::Variables, Format::Binary) => "bsdav",
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
        let both = |t| (ext_for(t, Format::Ascii), ext_for(t, Format::Binary));
        assert_eq!(both(FileType::Basic), ("bas", "bbas"));
        assert_eq!(both(FileType::Machine), ("bin", "bin"));
        assert_eq!(both(FileType::Reserve), ("sdar", "bsdar"));
        assert_eq!(both(FileType::Variables), ("sdav", "bsdav"));
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
