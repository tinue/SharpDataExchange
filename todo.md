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
