//! Per-user config file for `get`/`put` defaults.
//!
//! Modeled on `git config --global`, but simpler: a single optional dotfile
//! (`~/.sderc`, or `%USERPROFILE%\.sderc` on Windows) holding `key = value` lines.
//! Absent entirely, every default falls back to today's behavior (no configured port,
//! verbosity off). Read/write is done through `sde config get/set`, not by requiring
//! hand-editing, though the format is deliberately simple enough to hand-edit too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// The *directory* holding the `pc1600emul` pseudo-terminal socket, used when
/// `--port` is not given. The socket file itself is always named
/// [`PC1600EMUL_SOCKET_FILENAME`] inside that directory — this key configures where
/// that directory is, not the full port path.
pub const KEY_PC1600EMUL_PORT: &str = "pc1600emul.port";
/// Fixed filename of the `pc1600emul` pseudo-terminal socket within the configured
/// (or default) directory — matches the Calc-U-1600 emulator's own socket name.
pub const PC1600EMUL_SOCKET_FILENAME: &str = "calcu1600.serial";
/// Directory used for the `pc1600emul` socket when [`KEY_PC1600EMUL_PORT`] has never
/// been set.
pub const DEFAULT_PC1600EMUL_DIR: &str = "/tmp";
/// Default verbosity (`"true"` / `"false"`) when neither `-v` nor `-q` is given.
pub const KEY_VERBOSE: &str = "verbose";

pub struct Config {
    entries: BTreeMap<String, String>,
}

impl Config {
    /// Resolve the config file path from `$HOME` (or `%USERPROFILE%` on Windows).
    /// Does not require the file to exist.
    pub fn path() -> Result<PathBuf> {
        let home = if cfg!(windows) {
            std::env::var("USERPROFILE")
        } else {
            std::env::var("HOME")
        }
        .context("could not determine home directory ($HOME / %USERPROFILE% not set)")?;
        Ok(PathBuf::from(home).join(".sderc"))
    }

    /// Load the config file. Returns an empty config (not an error) if it doesn't exist.
    pub fn load() -> Result<Config> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Config { entries: parse(&text) }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Config { entries: BTreeMap::new() })
            }
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|s| s.as_str())
    }

    /// Build a `Config` directly from entries, bypassing the filesystem. Used by other
    /// modules' tests (e.g. `verbosity`) that need a config without touching disk.
    #[cfg(test)]
    pub fn from_entries_for_test(entries: BTreeMap<String, String>) -> Config {
        Config { entries }
    }

    /// Whether the config's `verbose` key is set to `"true"`. `None` if unset.
    pub fn verbose(&self) -> Option<bool> {
        self.get(KEY_VERBOSE).map(|v| v == "true")
    }

    /// Set `key = value` and write the file back to disk immediately.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        self.entries.insert(key.to_string(), value.to_string());
        let path = Self::path()?;
        std::fs::write(&path, format(&self.entries))
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

/// Parse `key = value` lines. `#`-comment and blank lines are ignored. Malformed lines
/// (no `=`) are ignored rather than rejected — this file is meant to be forgiving.
fn parse(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn format(entries: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (k, v) in entries {
        out.push_str(k);
        out.push_str(" = ");
        out.push_str(v);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_comments_and_blanks() {
        let text = "# comment\n\npc1600emul.port = /dev/ttys006\nverbose=true\n";
        let map = parse(text);
        assert_eq!(map.get(KEY_PC1600EMUL_PORT).map(String::as_str), Some("/dev/ttys006"));
        assert_eq!(map.get(KEY_VERBOSE).map(String::as_str), Some("true"));
    }

    #[test]
    fn parse_ignores_malformed_lines() {
        let map = parse("not a valid line\nkey = value\n");
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("key").map(String::as_str), Some("value"));
    }

    #[test]
    fn format_roundtrips_through_parse() {
        let mut entries = BTreeMap::new();
        entries.insert(KEY_PC1600EMUL_PORT.to_string(), "/dev/ttys006".to_string());
        entries.insert(KEY_VERBOSE.to_string(), "true".to_string());
        let text = format(&entries);
        let reparsed = parse(&text);
        assert_eq!(reparsed, entries);
    }

    #[test]
    fn config_get_verbose_helpers() {
        let cfg = Config { entries: parse("verbose = true\n") };
        assert_eq!(cfg.verbose(), Some(true));
        let cfg = Config { entries: parse("verbose = false\n") };
        assert_eq!(cfg.verbose(), Some(false));
        let cfg = Config { entries: BTreeMap::new() };
        assert_eq!(cfg.verbose(), None);
    }
}
