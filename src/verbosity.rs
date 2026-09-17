//! `-v`/`-q`/config-file verbosity precedence, per `requirements-put-get.md` §7a.
//!
//! `-v`/`--verbose` and `-q`/`--quiet` are mutually exclusive on the command line
//! (enforced by clap's `conflicts_with` at the CLI layer); this module resolves the
//! remaining precedence: an explicit flag always wins over the config file's default,
//! and the baseline (nothing set anywhere) is quiet, matching today's behavior.

use crate::config::Config;

/// The `-v`/`-q` state as parsed from the command line, before consulting config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerbosityFlag {
    Verbose,
    Quiet,
    Unset,
}

/// Resolve effective verbosity: `-v` > `-q` > config `verbose` default > off.
pub fn resolve(flag: VerbosityFlag, config: &Config) -> bool {
    match flag {
        VerbosityFlag::Verbose => true,
        VerbosityFlag::Quiet => false,
        VerbosityFlag::Unset => config.verbose().unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config_with_verbose(value: Option<&str>) -> Config {
        // Config's fields are private; build via the public parse-through-load path
        // is filesystem-backed, so we instead exercise `resolve` against a config
        // built from `Config::set`-shaped data through the crate-internal test hook.
        let mut entries = BTreeMap::new();
        if let Some(v) = value {
            entries.insert(crate::config::KEY_VERBOSE.to_string(), v.to_string());
        }
        Config::from_entries_for_test(entries)
    }

    #[test]
    fn explicit_verbose_always_wins() {
        assert!(resolve(VerbosityFlag::Verbose, &config_with_verbose(Some("false"))));
        assert!(resolve(VerbosityFlag::Verbose, &config_with_verbose(None)));
    }

    #[test]
    fn explicit_quiet_always_wins() {
        assert!(!resolve(VerbosityFlag::Quiet, &config_with_verbose(Some("true"))));
        assert!(!resolve(VerbosityFlag::Quiet, &config_with_verbose(None)));
    }

    #[test]
    fn unset_falls_back_to_config_then_off() {
        assert!(resolve(VerbosityFlag::Unset, &config_with_verbose(Some("true"))));
        assert!(!resolve(VerbosityFlag::Unset, &config_with_verbose(Some("false"))));
        assert!(!resolve(VerbosityFlag::Unset, &config_with_verbose(None)));
    }
}
