//! Device-scoped keyword registry.
//!
//! Two vocabularies: PC-1500 family and PC-1600 family. Each is built from a static
//! pool (`keywords::PC1500_SET` / `PC1600_SET`) already in the intended lookup order,
//! so inserting in slice order with "later wins" reproduces the correct name/code
//! lookup semantics on collisions.

use std::collections::{HashMap, HashSet};

use crate::keywords::{self, Keyword};

/// Which machine family a conversion targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Pc1500,
    Pc1600,
}

/// `REM` token, hard-coded in both directions (`0xF1AB` in both the PC-1500 and
/// PC-1600 tables).
pub const REM_CODE: u16 = 0xF1AB;

pub struct Registry {
    device: Device,
    /// Name and every dotted abbreviation prefix (upper-case) -> keyword.
    by_name: HashMap<String, &'static Keyword>,
    /// 2-byte token code -> keyword.
    by_code: HashMap<u16, &'static Keyword>,
    /// Set of `code >> 8` over the pool: "is this byte the lead of a 2-byte token".
    high_bytes: HashSet<u8>,
    /// First ASCII letter of `name` -> keywords with that initial, sorted by name
    /// length descending (so a greedy first-match yields the longest keyword, e.g.
    /// `PEEK#` before `PEEK`). Used by the ROM-style scanner.
    by_first_letter: HashMap<u8, Vec<&'static Keyword>>,
}

impl Registry {
    pub fn for_device(device: Device) -> &'static Registry {
        match device {
            Device::Pc1500 => pc1500(),
            Device::Pc1600 => pc1600(),
        }
    }

    fn build(device: Device, pool: &'static [Keyword]) -> Registry {
        let mut by_name = HashMap::new();
        let mut by_code = HashMap::new();
        let mut high_bytes = HashSet::new();
        let mut by_first_letter: HashMap<u8, Vec<&'static Keyword>> = HashMap::new();
        // On a same-name collision the last pool entry wins, matching `by_name`'s
        // `HashMap::insert` "later wins" semantics. `by_first_letter` feeds the
        // ROM-style scanner via `longest_keyword_prefix`, which returns the first
        // list match -- so without shadowing here the scanner would disagree with
        // `lookup()`: e.g. PC-1600 `LCURSOR` would tokenize to the CE-150 token
        // 0xE683 instead of the native 0xF0A5 (pool order is Ce150 ++ Ce158 ++
        // Pc1600, so the native entry comes last).
        let mut last_idx: HashMap<&str, usize> = HashMap::new();
        for (idx, kw) in pool.iter().enumerate() {
            last_idx.insert(kw.name, idx);
        }
        for (idx, kw) in pool.iter().enumerate() {
            by_name.insert(kw.name.to_string(), kw);
            for abbr in all_abbreviations(kw) {
                by_name.insert(abbr, kw);
            }
            by_code.insert(kw.code, kw);
            high_bytes.insert((kw.code >> 8) as u8);
            if last_idx[kw.name] != idx {
                continue; // shadowed by a later entry with the same name
            }
            if let Some(&first) = kw.name.as_bytes().first() {
                by_first_letter.entry(first).or_default().push(kw);
            }
        }
        for list in by_first_letter.values_mut() {
            // Longest name first; stable within a length so pool order (later wins) is
            // preserved for same-name/same-length collisions.
            list.sort_by_key(|kw| std::cmp::Reverse(kw.name.len()));
        }
        Registry { device, by_name, by_code, high_bytes, by_first_letter }
    }

    /// Longest keyword whose `name` is a prefix of `rest` (an upper-case byte string),
    /// or `None`. Case-sensitive: the ROM only matches upper-case `A..=Z`.
    pub fn longest_keyword_prefix(&self, rest: &[u8]) -> Option<&'static Keyword> {
        let first = *rest.first()?;
        let candidates = self.by_first_letter.get(&first)?;
        candidates
            .iter()
            .copied()
            .find(|kw| rest.starts_with(kw.name.as_bytes()))
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// Look up by full name or dotted abbreviation, case-insensitively.
    pub fn lookup(&self, name_or_abbrev: &str) -> Option<&'static Keyword> {
        if name_or_abbrev.is_empty() {
            return None;
        }
        self.by_name.get(&name_or_abbrev.to_ascii_uppercase()).copied()
    }

    pub fn lookup_code(&self, code: u16) -> Option<&'static Keyword> {
        self.by_code.get(&code).copied()
    }

    /// True if `high_byte` starts at least one 2-byte token in this registry.
    pub fn is_two_byte_token_high_byte(&self, high_byte: u8) -> bool {
        self.high_bytes.contains(&high_byte)
    }
}

/// All valid dotted forms, shortest to longest, excluding the bare full name.
/// `PRINT`/`P` -> `["P.", "PR.", "PRI.", "PRIN."]`; no abbreviation -> empty.
/// Mirrors `BasicKeyword.getAllAbbreviations()`.
fn all_abbreviations(kw: &Keyword) -> Vec<String> {
    let name = kw.name;
    let abbr = kw.abbrev;
    if abbr.len() >= name.len() {
        return Vec::new();
    }
    (abbr.len()..name.len())
        .map(|len| format!("{}.", &name[..len]))
        .collect()
}

macro_rules! lazy_registry {
    ($fn_name:ident, $device:expr, $pool:expr) => {
        pub fn $fn_name() -> &'static Registry {
            use std::sync::OnceLock;
            static CELL: OnceLock<Registry> = OnceLock::new();
            CELL.get_or_init(|| Registry::build($device, $pool))
        }
    };
}

lazy_registry!(pc1500, Device::Pc1500, keywords::PC1500_SET);
lazy_registry!(pc1600, Device::Pc1600, keywords::PC1600_SET);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_lookups() {
        let r = pc1500();
        assert_eq!(r.lookup("PRINT").unwrap().code, 0xF097);
        assert_eq!(r.lookup("print").unwrap().code, 0xF097);
        assert_eq!(r.lookup("P.").unwrap().code, 0xF097);
        assert_eq!(r.lookup("PR.").unwrap().code, 0xF097);
        assert_eq!(r.lookup("PRIN.").unwrap().code, 0xF097);
        assert!(r.lookup("PRINT.").is_none()); // full-length dotted form is not registered
        assert_eq!(r.lookup_code(0xF097).unwrap().name, "PRINT");
        assert_eq!(r.lookup_code(REM_CODE).unwrap().name, "REM");
    }

    #[test]
    fn high_byte_set_is_data_driven() {
        let r = pc1500();
        assert!(r.is_two_byte_token_high_byte(0xF0));
        assert!(r.is_two_byte_token_high_byte(0xF1));
        assert!(r.is_two_byte_token_high_byte(0xE6)); // CE-150
        assert!(r.is_two_byte_token_high_byte(0xE8)); // CE-158
        assert!(!r.is_two_byte_token_high_byte(0x41)); // 'A'
        assert!(!r.is_two_byte_token_high_byte(0xF2)); // PC-1600 only
    }

    #[test]
    fn pc1600_collisions_resolve_to_native() {
        let r = pc1600();
        // CE-150 LCURSOR 0xE683 is overridden by PC-1600 native 0xF0A5 (pool order).
        assert_eq!(r.lookup("LCURSOR").unwrap().code, 0xF0A5);
        assert_eq!(r.lookup("LINE").unwrap().code, 0xF099);
        assert_eq!(r.lookup_code(0xF0B7).unwrap().name, "LLINE");
        assert!(r.is_two_byte_token_high_byte(0xF2));
        assert!(r.is_two_byte_token_high_byte(0xE3)); // PAPER 0xE381
    }

    #[test]
    fn pc1600_has_no_abbreviations() {
        let r = pc1600();
        assert!(r.lookup("P.").is_none());
        assert_eq!(r.lookup("PRINT").unwrap().code, 0xF097);
    }

    #[test]
    fn scanner_prefix_match_agrees_with_lookup_on_collisions() {
        // The ROM-style scanner path (`longest_keyword_prefix`) must resolve a
        // same-name collision to the same keyword `lookup()` does -- the later
        // pool entry. Regression: PC-1600 `LCURSOR` / `LINE` used to match the
        // earlier CE-150 entry here while `lookup()` returned the native one.
        let r = pc1600();
        for name in ["LCURSOR", "LINE", "GLCURSOR", "COLOR", "SORGN"] {
            let via_scan = r.longest_keyword_prefix(name.as_bytes()).unwrap().code;
            let via_name = r.lookup(name).unwrap().code;
            assert_eq!(via_scan, via_name, "{name}: scanner {via_scan:#06X} != lookup {via_name:#06X}");
        }
        assert_eq!(r.longest_keyword_prefix(b"LCURSOR").unwrap().code, 0xF0A5);
        assert_eq!(r.longest_keyword_prefix(b"LINE").unwrap().code, 0xF099);
    }
}
