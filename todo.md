# TODO

## 1. Release-scaffolding loose ends

- `CHANGELOG.md` compare/tag links still say `OWNER` — change to `tinue`.
- Native Linux arm64 runners (`ubuntu-22.04-arm` / `ubuntu-24.04-arm`) are used;
  fall back to cross-compilation if those labels are ever unavailable.
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

## 4. Serial-free lib variant (reduce Calc-U-1600's dependencies)

Calc-U-1600 links `sharpdx.lib` only for tokenize/detokenize (the `convert` verb's
underlying calls) — it never touches `sde get`/`sde put`'s serial transport. It still
has to pull in `serialport` and its Windows backend regardless (including the
transitive `windows-sys` 0.52 import-lib baggage worked around in 0.2.0's release
packaging). Offer a serial-free build of the lib — e.g. a Cargo feature gating the
serial transport module (`serialport` dependency, `get`/`put` commands, C ABI transfer
entry points) off by default or behind a feature — so tokenize/detokenize-only
consumers can link a smaller artifact with fewer dependencies.

## 5. Manual follow-ups (trigger yourself)

- [ ] PC-1600 ROM dump using the Rust version; once successful, update the PC-1600 ROM
      repository.
- [ ] Archive/rename the SharpDataExchange project; rename SharpDataExchangeRust to
      SharpDataExchange.
- [ ] Full test with PC-1500, including reserve area and variables.
- [ ] Load assembly programs on both PC-1600 and PC-1500.
- [ ] Clean/fix the serial setup on PC-1600 for both emulation (no flow control) and real
      hardware; store this information in an easy-to-reach location (real hardware: S2-Card).
