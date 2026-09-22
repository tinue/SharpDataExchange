# TODO

## 1. Release-scaffolding loose ends

- `CHANGELOG.md` compare/tag links still say `OWNER` — change to `tinue`.
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
- [x] Archive/rename the SharpDataExchange project; rename SharpDataExchangeRust to
      SharpDataExchange.
- [ ] Full test with PC-1500, including reserve area and variables.
- [ ] Load assembly programs on both PC-1600 and PC-1500.
- [ ] Clean/fix the serial setup on PC-1600 for both emulation (no flow control) and real
      hardware; store this information in an easy-to-reach location (real hardware: S2-Card).

## 5. Disk/card image access: `dir` / `get` / `put` on Calc-U-1600 images

Idea (from a feasibility discussion, 2026-09-22): an imgtool-like
(<https://docs.mamedev.org/tools/imgtool.html>) way to read and write files on
Calc-U-1600's CE-1600F floppy images (`.floppy.yaml`) and PC-1600 RAM-disk card images
(CE-1601M etc.). Better as an sde extension than a separate tool: sde already owns the
16-byte file header and (de)tokenizing, and `get`/`put` already mean "move a file between
host and pocket computer" — here the other end is an image instead of the serial port.

**Feasibility: high.** The filesystem is small and documented in
`SharpPC1500Reference/PC-1600/PC-1600-Filesystem.md` §5: boot sector (`55 80` signature,
geometry at `08H..1AH`), a one-byte FAT (≤254 clusters, `FF` = end of chain) plus a copy,
32-byte MS-DOS-style directory entries. The floppy always uses the fixed F2H (64 KB)
layout, one volume per side, 48 directory entries. RAM-disk media read their geometry
from the boot sector, so a "block device + boot-sector geometry" driver covers both
(including superRAM 512K's hand-patched FB header). The directory timestamps are plain
FAT words with year and seconds = 0 (`hour<<11 | min<<5`, `month<<5 | day`) — corrected
in the reference doc from clean Systemhandbuch text.

Proposed scope:
- `dir <image>` — list files (name, type from the header, size, date/time).
- `get <image>:<vol>:<NAME.EXT> [out]` — extract raw, or de-tokenize a BASIC file to a
  listing (same output options as serial `get`).
- `put <file> <image>:<vol>:` — store; tokenize a `.bas` listing on the way in.
- Maybe `del`. Later, if wanted: `format`, `info`.
- Put the filesystem code in the library (serial-free feature) so Calc-U-1600 gets it via
  the vendored libsharpdx too — e.g. a preset that puts a `.bas` file straight onto a disk.

Things to settle:
- [ ] **Format ownership.** `.floppy.yaml` and the card YAML are Calc-U-1600 formats; sde
      has no YAML dependency. The needed subset (flat header + `addressed-hex` blocks) is
      small enough for a hand-written reader, but the spec now lives in two repos — honour
      `format-version` strictly and reject unknown versions.
- [ ] **Floppy first.** Floppy files are always written whole. Card instance files are
      spliced in place with comments preserved (Calc-U-1600 `BatteryCardInstance.hpp`) —
      the writer must match that. The RAM-disk byte layout across card banks (slot/bank
      order, superRAM's 32K `S0:` carve) lives in Calc-U-1600's card model and still needs
      pinning down in the YAML.
- [ ] **Not every card has a filesystem.** Only RAM-disk media (`INIT … "F"` / `"M"`)
      do; program modules (`"P"`) and PC-1500 cards don't. Detect the `55 80` boot sector
      and fail cleanly with "no filesystem".
- [ ] **Addressing syntax.** Give image targets an explicit form rather than guessing from
      the argument; each floppy side is its own volume, so the side is part of the path
      (e.g. `disk.floppy.yaml:A:PROG.BAS`, `card.yaml:S2:`).
- [ ] **Clash with a running emulator.** If the image is mounted in Calc-U-1600, its next
      save overwrites sde's edits. Document "eject first", or have the app watch the file.
- [ ] **SAVE quirks to mirror.** The ROM only writes FAT bytes 0–122 (the rest of the
      sector is leftover buffer RAM) and does not update the FAT copy on SAVE — trust the
      first FAT and don't flag copy mismatches as corruption.
- [ ] **Open details.** Attribute bit 1 (`"I"`) is unknown — pass it through, write `20H`
      on new files. Confirm the image's flat 64 KB-per-side order equals the ROM's logical
      sector numbering (track×8+sector). Verify the timestamp packing once by SAVEing at
      a known `TIME` in the emulator. Round-trip test: files written by sde must `FILES`/
      `LOAD` in the headless harness, and vice versa.
- Out of scope: real CE-1600F disks (no flux/sector-format tooling exists).
