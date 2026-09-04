# Changelog

## [0.2.0] - Unreleased

### Added
- Support for source-only comments in ASCII BASIC files: lines starting with `//` or `#` at column 0 are recognised as documentation comments and stripped before the program is sent to the device, saving RAM that `REM` statements would otherwise consume.
- `--device pc1600emul`: talk to a PC-1600 emulator over a host pseudo-terminal. Uses the PC-1600 wire format but runs without hardware (RTS/CTS) flow control and paces the send like the PC-1500. The port must be given with `--port` (e.g. `/dev/ttys006`); emulator pseudo-terminals cannot be auto-detected.

### Fixed
- Serial port names are now accepted with surrounding whitespace or a trailing slash (a path such as `/dev/ttys006/` previously failed to open with `ENOTDIR`).
- Explicit device paths that the OS does not enumerate as serial ports (emulator pseudo-terminals, `/dev/ttysNNN`) are now opened directly instead of reported as "Could not open serial port".
- The send side now drains the output buffer before closing the port, so the tail of a transfer is no longer lost when the far end reads slowly (previously affected the PC-1600 path).
- Failed port opens now report the underlying OS error code.

### Changed

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
