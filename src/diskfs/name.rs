//! 8.3 file names and DOS-style wildcard patterns as stored in a directory entry.

use super::DiskError;

/// Characters allowed in a stored name besides `A-Z` and `0-9` (MS-DOS's set, minus
/// anything the PC-1600's `"X:NAME.EXT"` syntax would trip over).
const EXTRA_CHARS: &[u8] = b"!#$%&'()-@^_`{}~";

/// A file name as stored in a directory entry: 8 + 3 bytes, upper-case, space-padded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileName {
    stem: [u8; 8],
    ext: [u8; 3],
}

impl FileName {
    /// Parse `NAME` or `NAME.EXT` (case-insensitive; stored upper-case). The stem must be
    /// 1–8 characters, the extension 0–3.
    pub fn parse(s: &str) -> Result<FileName, DiskError> {
        let bad = |why: &str| DiskError::BadName(format!("{s:?}: {why}"));
        let (stem, ext) = match s.rsplit_once('.') {
            Some((stem, ext)) => (stem, ext),
            None => (s, ""),
        };
        if stem.is_empty() {
            return Err(bad("empty name"));
        }
        if stem.len() > 8 {
            return Err(bad("name longer than 8 characters"));
        }
        if ext.len() > 3 {
            return Err(bad("extension longer than 3 characters"));
        }
        let mut name = FileName { stem: [b' '; 8], ext: [b' '; 3] };
        for (dst, src) in [(&mut name.stem[..], stem), (&mut name.ext[..], ext)] {
            for (i, c) in src.bytes().enumerate() {
                let c = c.to_ascii_uppercase();
                if !(c.is_ascii_uppercase() || c.is_ascii_digit() || EXTRA_CHARS.contains(&c)) {
                    return Err(bad(&format!("character {:?} is not allowed", c as char)));
                }
                dst[i] = c;
            }
        }
        Ok(name)
    }

    /// From the 11 raw bytes of a directory entry (not validated: whatever the ROM wrote).
    pub fn from_raw(raw: &[u8]) -> FileName {
        let mut name = FileName { stem: [b' '; 8], ext: [b' '; 3] };
        name.stem.copy_from_slice(&raw[0..8]);
        name.ext.copy_from_slice(&raw[8..11]);
        name
    }

    pub fn raw(&self) -> [u8; 11] {
        let mut out = [0u8; 11];
        out[..8].copy_from_slice(&self.stem);
        out[8..].copy_from_slice(&self.ext);
        out
    }

    pub fn stem(&self) -> String {
        trim(&self.stem)
    }

    pub fn ext(&self) -> String {
        trim(&self.ext)
    }
}

fn trim(b: &[u8]) -> String {
    crate::cp437::decode(b).trim_end_matches(' ').to_string()
}

impl std::fmt::Display for FileName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ext = self.ext();
        if ext.is_empty() {
            write!(f, "{}", self.stem())
        } else {
            write!(f, "{}.{ext}", self.stem())
        }
    }
}

/// A DOS-style name pattern: `*` matches the rest of the name or extension part, `?` one
/// character (or the padding after a short name). `*` alone means every file; a pattern
/// without `.` only matches names without an extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    stem: [u8; 8],
    ext: [u8; 3],
}

impl Pattern {
    pub fn parse(s: &str) -> Result<Pattern, DiskError> {
        let bad = |why: &str| DiskError::BadName(format!("{s:?}: {why}"));
        let (stem, ext) = match s.rsplit_once('.') {
            Some((stem, ext)) => (stem, ext),
            None if s == "*" => ("*", "*"),
            None => (s, ""),
        };
        let mut p = Pattern { stem: [b' '; 8], ext: [b' '; 3] };
        for (dst, src, what) in [(&mut p.stem[..], stem, "name"), (&mut p.ext[..], ext, "extension")] {
            let mut i = 0;
            for c in src.bytes() {
                if i == dst.len() {
                    return Err(bad(&format!("{what} too long")));
                }
                let c = c.to_ascii_uppercase();
                if c == b'*' {
                    dst[i..].fill(b'?');
                    i = dst.len();
                    continue;
                }
                if !(c == b'?' || c.is_ascii_uppercase() || c.is_ascii_digit() || EXTRA_CHARS.contains(&c)) {
                    return Err(bad(&format!("character {:?} is not allowed", c as char)));
                }
                dst[i] = c;
                i += 1;
            }
        }
        if p.stem == [b' '; 8] {
            return Err(bad("empty name"));
        }
        Ok(p)
    }

    /// True if the text contains a wildcard (so it should be treated as a pattern).
    pub fn is_wildcard(s: &str) -> bool {
        s.contains(['*', '?'])
    }

    pub fn matches(&self, name: &FileName) -> bool {
        let raw = name.raw();
        self.stem.iter().chain(self.ext.iter()).zip(raw.iter()).all(|(&p, &c)| p == b'?' || p == c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_names() {
        let n = FileName::parse("globus.bas").unwrap();
        assert_eq!(&n.raw(), b"GLOBUS  BAS");
        assert_eq!(n.to_string(), "GLOBUS.BAS");
        assert_eq!(FileName::parse("MC").unwrap().to_string(), "MC");
        assert_eq!(FileName::parse("A-1_$.C~").unwrap().to_string(), "A-1_$.C~");
        for bad in ["", ".BAS", "TOOLONGNAME.BAS", "A.BASX", "A B.BAS", "A*.BAS", "A/B", "Ä.BAS", "A.B.C"] {
            assert!(FileName::parse(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn patterns() {
        let name = |s| FileName::parse(s).unwrap();
        let pat = |s| Pattern::parse(s).unwrap();
        assert!(pat("*").matches(&name("GLOBUS.BAS")));
        assert!(pat("*").matches(&name("MC")));
        assert!(pat("*.*").matches(&name("MC")));
        assert!(pat("*.BAS").matches(&name("bio.bas")));
        assert!(!pat("*.BAS").matches(&name("BIO.ASM")));
        assert!(pat("G*.*").matches(&name("GLOBUS.BAS")));
        assert!(!pat("G*.*").matches(&name("BIO.BAS")));
        assert!(pat("B?O.BAS").matches(&name("BIO.BAS")));
        assert!(pat("BIO?.BAS").matches(&name("BIO.BAS"))); // ? matches padding
        assert!(!pat("BIO").matches(&name("BIO.BAS"))); // no '.': extension must be empty
        assert!(pat("BIO").matches(&name("BIO")));
        assert!(pat("bio.bas").matches(&name("BIO.BAS")));
        assert!(Pattern::parse("*.TOOLONG").is_err());
        assert!(Pattern::is_wildcard("*.BAS") && !Pattern::is_wildcard("A.BAS"));
    }
}
