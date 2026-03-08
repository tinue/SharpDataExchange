# Changelog

## [0.1.0] - 2026-03-08

Initial release.

### Added
- Verb-based CLI: `get`, `put`, and experimental `terminal` mode.
- Support for Sharp PC-1500 and PC-1600.
- Automatic de-tokenization of BASIC programs to readable ASCII.
- Support for PC-1500 Reserve Area (SDAR) and Variables (SDAV).
- Remote execution wrapper `sder` to work around macOS USB/Serial bug.
- Automatic filename and extension resolution for `get`.
- Resource-safe serial port handling with AutoCloseable.
- Robust header detection and fallback handling.
