# TODO

## 1. Release-scaffolding loose ends

- Native arm64 runners (`ubuntu-22.04-arm` / `ubuntu-24.04-arm` / `windows-11-arm`)
  are used; fall back to cross-compilation if those labels are ever unavailable.
- `Swatinem/rust-cache@v2` still triggers a Node 20 deprecation warning; revisit
  when a Node 24 major ships.

## 2. Abbreviations via keyword ordering (not an explicit abbreviation list)

The ROM does not store a list of valid abbreviations. It stores an **ordered list of
keywords per initial letter** and matches greedily: `P.` resolves to whatever the first
`P*` entry in that list is (`PRINT`), because the table is ordered so the intended
keyword comes first. This currently relies on the explicit `abbrev` field extracted from
the upstream keyword tables; replace that with ROM-order-driven resolution.

Research needed:
- **a) What is the keyword order?** Recover the per-letter ordering of the PC-1500 ROM
  keyword table (linked lists at `$C020` / `$C054` in the disassembly). This is the
  authority for which keyword a bare `X.` abbreviation expands to.
- **b) How does the order interact with CE-150 and/or CE-158?** Peripheral keywords live
  in their own token ranges (`0xE6xx`–`0xE8xx`) — determine where/whether they are
  inserted into the per-letter match order when the peripheral is attached, and how that
  affects abbreviation resolution (e.g. does `L.` change meaning with a CE-150 present?).

## 3. PC-1600 abbreviations + keyword ordering

`Pc1600Keywords.java` (the upstream keyword source) defines no abbreviations ("not
documented in the manuals"), so our PC-1600 path currently expands nothing. But the
PC-1600 **does**
support abbreviations. Same research as item 2, for the PC-1600:
- Recover the PC-1600 keyword table order (no PC-1600 ROM source in the corpus — needs a
  PC-1600 ROM dump / PockEmul, or hardware observation).
- Determine the CE-150 / CE-158 interaction for the PC-1600 match order.

## 4. Manual follow-ups (trigger yourself)

- [ ] PC-1600 ROM dump using the Rust version; once successful, update the PC-1600 ROM
      repository.
- [ ] Full test with PC-1500, including reserve area and variables.
- [ ] Load assembly programs on both PC-1600 and PC-1500.
- [ ] Clean/fix the serial setup on PC-1600 for both emulation (no flow control) and real
      hardware; store this information in an easy-to-reach location (real hardware: S2-Card).

## 5. Disk/card images: open items

Floppy images (CE-1600F `.floppy.yaml`) are supported since 0.2.3; layering and the
settled ROM facts are in `src/diskfs/` and `PC-1600-Filesystem.md` §5.4.

- [ ] **RAM-disk cards** (CE-1601M, superRAM …): geometry from the `55 80` boot sector,
      `FF` end of chain; card instance files are spliced in place with comments preserved
      (Calc-U-1600 `BatteryCardInstance.hpp`) and the RAM-disk byte layout across card
      banks still needs pinning down. Program modules (`"P"`) and PC-1500 cards have no
      filesystem — fail cleanly.
- [ ] **Use it in Calc-U-1600**: refresh the vendored libsharpdx and, e.g., let a preset
      put a `.bas` file straight onto a disk (`sde_disk_put`). Nothing in Calc-U-1600
      calls the `sde_disk_*` functions yet (also tracked in its `TODO.md`).
- [ ] `#SEGMENT` programs on disk: sde stores the in-memory form (bare `FF`); confirm
      with a ROM `SAVE` of a segmented program.
- [ ] Machine code in a bank other than 0: sde writes run address `<bank>:FFFF` for "no
      auto-start"; the ROM was only observed with bank 0 (`00FFFF`).
- [ ] Maybe `format` (INIT) and `info`; raw `.img` import/export.
- Out of scope: real CE-1600F disks (no flux/sector-format tooling exists).

## 6. Tokenizer: line numbers after the first in `ON … GOTO/GOSUB`

Conflicting evidence: `scanner.rs` binary-encodes every target and cites a memory dump
(`10 ON A GOSUB 10,20,30` → `1F 00 0A 00 2C 1F 00 14 00 2C 1F 00 1E 00 0D`). The
disk observation below shows the opposite. Find out which one is right (typed in
directly vs. loaded/saved? PC-1600 ROM version?).

Found while reading a ROM-saved disk (`GLOBUS.BAS` on Calc-U-1600's `dw.img`): the ROM
stored `ON V GOTO 310,330` (line 300) as `F1 9C 56 F1 92 1F 01 36 00 2C 33 33 30` — only
the first target is a binary line-number reference (`1F hi lo 00`), the following ones
stay ASCII digits. sde encodes every target as `1F hi lo 00`, so re-tokenizing that
program comes out 2 bytes longer than the ROM's (`BIO.BAS` on the same disk re-tokenizes
byte-identically). Check the PC-1500 behaviour too, then match the ROM.

## 7. Code-quality follow-ups (skipped by the 2026-10-04 `/simplify` pass)

The first full-codebase simplify (`30f34d6`, tag `simplified`) left these alone. Each one
would change behaviour or is too large for a cleanup pass. Grouped by why it was skipped.

### 7a. Would change behaviour: decide first, then do

- [ ] **One machine-header builder.** Three places wrap headerless machine code:
      `convert::add_machine_header`, serial `transfer::build_put` (the `--start-address`
      branch) and `transfer::build_disk_put`. They have drifted:
      - Default run address: serial uses a flat `0xFFFF`, while convert and disk use
        `(start & 0xFF0000) | PC1600_NO_AUTORUN` on the PC-1600 (keeping the bank).
      - Address-range checks: convert checks 16/24 bits per device, disk checks 24 bits
        only, and serial checks nothing.
      - Payload-length limit: in convert, and again in `wav::from_image`.

      Fix: put the per-device facts on `registry::Device` (`max_addr()`,
      `no_autorun_run(start)`, `max_payload()`) and have both `transfer` paths call
      `add_machine_header`. Needs a decision on serial PC-1600's default (keep the bank or
      not) and on which error texts survive.
- [ ] **PC-1600 tape polarity.** `wav/pc1600.rs` decodes the whole signal in both
      polarities and keeps the better one. Choosing the polarity from the leader
      (`find_steady`) would halve the work, but could change which recordings decode;
      re-verify against the tape matrix (`dev/tape-matrix` in Calc-U-1600) first.
- [ ] **C ABI error text.** `ffi/mod.rs::finish_bytes` and `ffi/wav.rs::fail` set the
      message with `to_string()` (top-level error only), while `ffi/disk.rs::fail` uses
      `{e:#}` (the full context chain). One `fail_with(e, code_fn)` would unify them, but
      the message text that C callers see would change.
- [ ] **Serial `put --format ascii`.** `transfer.rs` claims to be the one place that
      decides what `--format` means, but for serial the "headerless ASCII BASIC +
      `--format ascii` → send line by line" rule lives in `put_cmd::run_put`, and
      `build_put` relies on the caller having intercepted it. Disk already models this as
      `PutKind::AsciiListing`. Fix: a `PutKind` for serial line-by-line sending, with
      `run_put` dispatching on `kind`.
- [ ] **"Can this go on tape?" policy.** `wav_cmd::tape_files_from` rejects
      Text/Raw/AsciiListing/Variables itself; `wav::from_image` partly does too. Letting
      `from_image` own it would change the error wording.

### 7b. Too large for a cleanup pass

- [ ] **C ABI WAV handle.** `sde_wav_count`, `sde_wav_decode` (per index) and
      `sde_file_info`/`sde_file_kind` (via `info::classify`) each run a full `wav::decode`
      on the same buffer. A host that counts, fetches every file and asks for info runs
      N+2 full decodes, which takes seconds and hundreds of MB on a long recording. Fix:
      `sde_wav_open` → handle, `sde_wav_get(h, i)`, `sde_wav_free(h)` (or one call that
      returns every file). This is a C API addition, so Calc-U-1600 needs updating too.
- [ ] **Decoder passes.** `wav/pc1500.rs` and `wav/pc1600.rs` each build their own
      edge-difference vectors (pc1600 twice, once per polarity, plus strided copies), and
      the two decoders run one after the other. Since samples stream (no copy of the
      recording), these vectors are most of the remaining decode memory: a 24 KB
      PC-1500 tape (30 min, 171 MB mono WAV) peaks at 466 MB, about 300 MB of it edges. Fix: compute `d` once (a PC-1600 cycle is
      `d[i] + d[i+1]`), and optionally run the sweeps under `std::thread::scope`.
- [ ] **Channel selection.** `wav/riff.rs` picks the loudest channel with
      2·channels + 1 passes and re-matches the sample format per sample. Fix: one pass
      accumulating sum and sum-of-squares for all channels, with the format `match`
      hoisted out of the loop.
- [ ] **Encoding cost.** `wav/encode.rs` grows its `Vec<f32>` without reserving, calls
      `sin` + `tanh` per sample, and `riff::write_pcm16` then copies everything into a
      second buffer (about 6 bytes resident per sample, ~1.3 GB peak for a 64 KB PC-1500
      file). Fix: a one-cycle lookup table for the shaped wave, capacity reserved from
      the exact duration, and `i16` written straight into the WAV buffer (or streamed to
      the file). Output must stay byte-identical, or be re-verified with CLOAD.
- [ ] **Dry runs that synthesize the tape.** `wav_cmd::write_tape` encodes the whole WAV
      even with `--dry-run` just to report seconds, and `run_play`'s dry run calls
      `encode_samples` for the same reason. Fix: a duration-only mode in `Synth` (advance
      `t` without pushing samples), then return early on dry run.
- [ ] **`put -f wav` with a WAV input reads the file twice.** `run_play` runs
      `decode_source` (read and full decode, for the description and warnings), then
      reads and parses it again via `wav::read_samples`. Fix: decode from one parsed
      `Pcm` that keeps its samples.
- [ ] **One `-f wav` decision for `get`.** Serial and disk `get` carry
      `format: Option<Format>` and `tape: Option<TapeOptions>` side by side and branch
      internally; `put` and `convert` branch in `main.rs`. Fix:
      `enum GetOutput { Host(GetSpec), Tape(TapeOptions) }` built once in `main.rs`, plus
      a shared `wav_cmd::write_image_as_tape(...)` for the near-identical serial and disk
      tape branches. This also removes the hand-written `--skip-header`/`--raw` +
      `-f wav` checks.
- [ ] **Parallel file classifiers.** `detect::Content`, `info::FileKind`,
      `transfer::DiskFileKind` and the C ABI's `SdeContent` each classify files, and
      `Content` is a content × device cross-product (hence the `unreachable!` in
      `detect.rs`). Unifying them is an architectural change.
- [ ] **Header magic and type-byte tables.** `info::header_magic` re-implements
      `header::find`'s magic matching (`01 ?? "COM"`, `FF 10 00 00`, the leading-`00`
      skip), and each file-type ↔ byte mapping is written once per direction (header.rs
      `ce158_file_type`/`build_ce158`, `pc1600_file_type`/`build_pc1600`, and again in
      both tape codecs). Fix: `header::magic_at(data)` plus one `const` table per format,
      searched both ways. These tables are format-critical, so this needs careful
      round-trip tests.

### 7c. Not worth it on its own (pick up when touching the code anyway)

- [ ] `info::describe` analyzes a tape's first file twice (`wav_summary` → `analyze`,
      then `tape_file_details` → `describe` → `analyze`). It is cheap (≤ 64 KB) but needs
      the image and `Analysis` passed down.
- [ ] The 16-bit byte-sum checksum is inlined in `get_cmd` and exists as
      `wav::pc1500::sum16`. It is one line, and sharing it couples serial and tape code.
- [ ] `fixture()` / `fixture_dir()` / `sde(dir, args)` are copied across the integration
      tests. A `tests/common/mod.rs` would need `#[allow(dead_code)]`, and each helper is
      about 3 lines.
- [ ] The tokenize pipeline copies the listing about 4 times (`text::decode_bas_listing`,
      `normalize`, trim, `convert::expand_all`). Inputs are ≤ 64 KB.
- [ ] `floppy_image.rs` clones each side's ~200 KB hex block when parsing and builds
      ~4096 small `String`s per side in `format_addressed_hex` when writing. This could be
      `&str` plus writing into `out` directly.
- [ ] `detect.rs` decodes the whole file to CP437 twice in order to examine at most 5
      lines. It could walk lines over the bytes lazily.
- Kept on purpose: `PocketDevice::supports_flow_control` (an alias of
  `is_pc1600_family`, but it states intent at the call site).
