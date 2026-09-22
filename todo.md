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

## 5. Disk/card image access: `dir` / `get` / `put` / `del` on Calc-U-1600 images

**Floppy: done (0.2.3).** `sde dir/get/put/del` on CE-1600F `.floppy.yaml` images, the
`sde_disk_*` C ABI on one side, and `tools/e2e_floppy.sh` (manual) proving against the
PC-1600 ROM in Calc-U-1600 that files written by sde `LOAD`/`INPUT#`/`BLOAD` and files
saved by the ROM read back. Layering: container spec owned by Calc-U-1600
(`docs/Floppy-Image-Format.md` there; sde has its own strict reader/writer), filesystem
in `src/diskfs/` (Sharp's format), conversions in `src/transfer.rs` (shared with serial).

Settled along the way (see the ROM-observation notes in `PC-1600-Filesystem.md` §5.4):
year field 6 and seconds stored; attribute `20H`; the ROM keeps the FAT copy in sync
(so sde writes both); image byte order = ROM logical sector numbering; emulator-formatted
floppies have an all-zero boot sector (only the FAT's `F2` identifies a formatted side).

Still open:
- [ ] **RAM-disk cards** (CE-1601M, superRAM …): geometry from the `55 80` boot sector,
      `FF` end of chain; card instance files are spliced in place with comments preserved
      (Calc-U-1600 `BatteryCardInstance.hpp`) and the RAM-disk byte layout across card
      banks still needs pinning down. Program modules (`"P"`) and PC-1500 cards have no
      filesystem — fail cleanly.
- [ ] **Use it in Calc-U-1600**: refresh the vendored libsharpdx and, e.g., let a preset
      put a `.bas` file straight onto a disk (`sde_disk_put`).
- [ ] `#SEGMENT` programs on disk: sde stores the in-memory form (bare `FF`); confirm
      with a ROM `SAVE` of a segmented program.
- [ ] Machine code in a bank other than 0: sde writes run address `<bank>:FFFF` for "no
      auto-start"; the ROM was only observed with bank 0 (`00FFFF`).
- [ ] Maybe `format` (INIT) and `info`; raw `.img` import/export.
- Out of scope: real CE-1600F disks (no flux/sector-format tooling exists).

## 6. Tokenizer: line numbers after the first in `ON … GOTO/GOSUB`

Found while reading a ROM-saved disk (`GLOBUS.BAS` on Calc-U-1600's `dw.img`): the ROM
stored `ON V GOTO 310,330` (line 300) as `F1 9C 56 F1 92 1F 01 36 00 2C 33 33 30` — only
the first target is a binary line-number reference (`1F hi lo 00`), the following ones
stay ASCII digits. sde encodes every target as `1F hi lo 00`, so re-tokenizing that
program comes out 2 bytes longer than the ROM's (`BIO.BAS` on the same disk re-tokenizes
byte-identically). Check the PC-1500 behaviour too, then match the ROM.
