# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project may contain
breaking changes in every release, major or minor, until version 1.0.0 is reached
(the C ABI and the Rust API included); from 1.0.0 on it follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`bin/release` reads the section for the version being released and uses it as the
GitHub release notes, so keep entries user-facing.

## [Unreleased]

## [0.2.2] - WIP

## [0.2.1] - 2026-09-19

### Changed

- The libraries in release archives (`lib/`) are now built without the serial
  transport, so embedders no longer link `serialport` or, on Windows, its extra
  import libs (e.g. `windows.0.52.0.lib`). The C ABI is unchanged.
- New Cargo features: `serial` (`get`/`put` transport) and `cli` (the `sde`
  binary, implies `serial`), both on by default. Build a serial-free lib with
  `cargo build --release --lib --no-default-features`.
- Windows arm64 release builds now run natively on an arm64 runner and run the
  test suite before packaging.
- The GitHub repository is now `tinue/SharpDataExchange` (formerly
  `SharpDataExchangeRust`; old URLs redirect). Release, changelog and
  `library.md` links point to the new name.

## [0.2.0] - 2026-09-18

### Added

- `sde get` / `sde put`: Reserve Area (SDAR ASCII form, key-assignment layers)
  and Variables (SDAV ASCII form) transfer support for PC-1500/PC-1500A.
  Both are get/put-only and PC-1500-only — `convert` stays BASIC-only and the
  C ABI keeps collapsing them to `Unknown`. Adds a PC-1500 8-byte BCD numeric
  codec (`src/bcd.rs`) to encode/decode Variables' numeric records.
- `install.sh`: builds `sde` in release mode and installs it to
  `~/Applications` on macOS.

### Fixed

- Windows release archives: `sharpdx.lib` is now shipped alongside any
  transitively-linked import lib that isn't a standard Windows SDK/CRT lib
  (e.g. an older `windows-sys`'s classic import lib, pulled in by a
  dependency's Windows backend). Without it, a downstream consumer linking
  the raw static lib directly hit "cannot open input file" for a name
  `native-static-libs` listed but never supplied.

## [0.1.5] - 2026-09-18

### Added

- `sde get` / `sde put`: real serial transfer of BASIC and machine-language
  programs to/from a real or emulated Sharp PC-1500 / PC-1500A / PC-1600, on top
  of the existing offline `convert` verb. Supports all four transport targets
  (`pc1500`/`pc1500a`/`pc1600`/`pc1600emul`), `--raw`, `--dry-run`, and
  header auto-add/ambiguity handling. Reserve Area and Variables transfer remain
  out of scope.
- `sde config`: reads an optional `~/.sderc` (`key = value`) for two defaults --
  the `pc1600emul` socket directory (fixed filename `calcu1600.serial`, default
  `/tmp`) and default verbosity -- plus `-v`/`-q` CLI overrides shared by `get`
  and `put`.
- PC-1600 patches a constant `GOTO`/`GOSUB`/bare-`THEN` jump target into a
  compact binary form (`0x1F [hi] [lo] 0x00`) instead of leaving it as ASCII
  digits (PC-1500 always uses ASCII digits). Confirmed against real
  PC-1500/PC-1600 memory dumps; `scanner::tokenize` reproduces this on PC-1600
  only, `detokenize::detokenize` decodes it on both devices.
- The device supports multiple `GOSUB "LABEL"`-addressable program segments per
  save, separated on the wire by `0xFF 0x00 0x00`. Confirmed on real PC-1600
  hardware. The reserved `#SEGMENT` marker line now round-trips this boundary in
  both directions; `sde_tokenize` (and `convert_with`/`scanner::tokenize`) take
  a `SdeSegmentMarker` option controlling how it tokenizes: `Wire`
  (`0xFF 0x00 0x00`, what `SAVE "COM1:"` actually transmits, the previous and
  default behavior) or `Memory` (bare `0xFF`, what the ROM's serial receiver
  actually stores into the program area). A caller poking tokenized bytes
  directly into RAM -- bypassing the serial protocol entirely -- should use
  `Memory`, since including the two wire-only pacing bytes made `BASPRG_END`
  come out 2 bytes too high per marker.

### Fixed

- `header::build_pc1600`'s end-of-header marker was `{0x00, 0xF0}`; a real
  PC-1600 capture confirms it should be `{0x00, 0x0F}`. Fixed, with the
  `depreciation-tokenized-pc1600header.bin` fixture regenerated to match.

## [0.1.4] - 2026-09-16

### Fixed

- PC-1600 keyword table: added native `PEEK`, `PEEK#`, `POKE`, and `CALL` tokens
  (0xF26D, 0xF26E, 0xF28C, 0xF282), distinct from `XPEEK`/`XPEEK#`/`XPOKE`/`XCALL`,
  which tokenize the PC-1500's codes for those same names.
- PC-1600 keyword table: removed the spurious `PROTOCOL` entry, which is a CE-158
  extension keyword, not a native PC-1600 one.
- PC-1600 keyword table: listed `DEV$` and `COM$` explicitly (same codes as
  CE-158's), since they are native to the PC-1600's built-in serial port.

## [0.1.3] - 2026-09-11

### Fixed

- `bin/release --release` crashed with `MAIN_BRANCH: unbound variable` while
  tagging the release commit: this bash mis-parses a bare `$VAR` immediately
  followed by a multi-byte UTF-8 character (here the `…` right after
  `$MAIN_BRANCH`) as part of the variable name. Braced `${MAIN_BRANCH}` fixes
  it; the release commit and tag still land correctly, only the final "no
  rebuild" promotion step was interrupted, so `v0.1.2`'s promotion had to be
  finished by hand.
- The tokenizer discarded a `'` (the `REM` shorthand) instead of keeping it,
  then went on to parse the rest of the line as code — a comment like
  `10 'X1 = 1.POSITION ON STACK` had `POSITION`/`ON` mistaken for keywords.
  `'` now behaves exactly like `REM`: it is kept, and everything after it on
  the line is copied through verbatim. The dotted-abbreviation expander had
  the same gap (`'CALL P. HERE` was corrupted into `'CALL PRINT HERE` before
  the scanner ever saw it) and is fixed the same way.
- `bin/release`'s `publish` job 403'd downloading its own build artifacts
  ("Failed to ListArtifacts") — a side effect of the earlier
  `download-artifact@v4->v7` bump: v5+ lists/downloads artifacts via the
  Actions REST API, which needs `actions: read`, but the job's `permissions:`
  block only granted `contents: write`. Added `actions: read`.

## [0.1.2] - 2026-09-11

### Fixed

- `lib/native-libs-windows.txt` (added in 0.1.1) was corrupted by a stray ANSI
  color-reset escape (`CARGO_TERM_COLOR=always` in CI coloring a note captured
  from a redirected, non-tty stderr) and its doc comment wrongly described the
  content as GCC `-lname` syntax; it's actually MSVC linker tokens (bare
  `name.lib` filenames and the occasional `/defaultlib:x` flag). Packaging now
  captures it with `CARGO_TERM_COLOR=never` and strips escapes defensively.

## [0.1.1] - 2026-09-11

### Added

- Windows release archives now include `lib/native-libs-windows.txt`: the Windows
  system import libs (`ws2_32`, `userenv`, `bcrypt`, ...) a non-cargo consumer
  linking the raw `sharpdx.lib` needs to supply itself, captured straight from
  `rustc --print=native-static-libs` for that build rather than left for every
  downstream consumer to guess.
- Platform-aware line endings for de-tokenized listings. The output now uses the
  host convention by default — `CRLF` on Windows, `LF` on macOS / Linux — and the
  terminator is overridable: `--eol auto|lf|crlf|cr` on the CLI,
  `sharpdx::convert_with(.., LineEnding)` in the crate, and a new
  `SdeLineEnding line_ending` argument on `sde_detokenize` / `sde_convert` in the
  C ABI.
- macOS releases are signed with a Developer ID and notarized (hardened runtime),
  so they run without a Gatekeeper warning. The `sde` / `libsharpdx.dylib` in the
  `.tar.gz` are notarized; a new **`SharpDataExchange-<version>.pkg`** installer is
  published alongside it — signed, notarized, and stapled — with a choice between
  an all-users install (`/usr/local/bin`) and a per-user one (`~/.local/bin`,
  added to `~/.zshrc`).

### Changed

- Tokenizing now accepts `CR` and `CRLF` line endings in the input listing on
  every platform (previously a real Windows `.bas` file with `CRLF` endings could
  leave a stray `0x0D` in the payload).
- **C ABI break:** `sde_detokenize` and `sde_convert` take an extra
  `SdeLineEnding line_ending` parameter (pass `SDE_LINE_ENDING_PLATFORM` for the
  previous-style host default). `sde_tokenize` is unchanged.

## [0.1.0] - 2026-09-08

### Added

- ROM-style linear scanner (`src/scanner.rs`) that tokenizes Sharp PC-1500 /
  PC-1600 BASIC in one left-to-right pass with a 3-state (normal / string /
  verbatim) model, instead of a grammar.
- `sde convert` CLI: content-detected direction, `-d/--device`
  `pc1500|pc1500a|pc1600|pc1600emul`, file or stdin/stdout I/O, output-path
  derivation.
- `libsharpdx` C ABI (`sde_convert`, `sde_tokenize`, `sde_detokenize`,
  `sde_detect`, `sde_version`, `sde_last_error`, `sde_buf_free`) with a
  panic-free boundary and caller-freed buffers. Committed C header and Swift
  module map generated from `src/ffi.rs`.
- `sharpdx` Rust crate exposing the pure `convert` core.
- Byte-parity test suite against the Java `SharpDataExchange` `convert` fixtures,
  plus round-trip and C-ABI tests.
- Release archives for macOS (Apple Silicon), Linux x86-64 / arm64, and Windows
  x64 / arm64, each bundling the CLI, the static and shared libraries, and the
  `include/` directory.

### Known limitations

- `convert` only — no `get` / `put` / `terminal`, serial I/O, Variables /
  Reserve-Area conversion, or cassette headers.
- PC-1600-only token values are limited to what `Pc1600Keywords.java` lists;
  unknown `>= 0xE0` byte pairs pass through opaquely.
- The CE-158 header filename field is upper-cased (the historical fixture stored
  it lower-case); `testsuite.md` marks those bytes "don't care". Payloads match
  the Java output exactly.

[Unreleased]: https://github.com/tinue/SharpDataExchange/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/tinue/SharpDataExchange/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/tinue/SharpDataExchange/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/tinue/SharpDataExchange/compare/v0.1.5...v0.2.0
[0.1.5]: https://github.com/tinue/SharpDataExchange/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/tinue/SharpDataExchange/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/tinue/SharpDataExchange/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/tinue/SharpDataExchange/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/tinue/SharpDataExchange/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tinue/SharpDataExchange/releases/tag/v0.1.0
