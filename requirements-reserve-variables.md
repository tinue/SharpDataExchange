# Requirements: Reserve Area & Variables transfer (PC-1500)

Status: proposed, not yet implemented.

## Scope

Add support for the two remaining PC-1500/1500A CE-158 payload types that `sde`
currently rejects: **Reserve Area** (`CSAVEr`/`CLOADr`, CE-158 type char `'A'`) and
**Variables** (`CSAVEv`/`CLOADv` — SDAV, CE-158 type char `'H'`). Both are
implemented by porting the format from the sister Java project
([`tinue/SharpDataExchange`](https://github.com/tinue/SharpDataExchange),
`ReserveAreaConverter.java` / `VariablesConverter.java`) byte-for-byte and
text-format-for-text-format — no new design, no deviation from the Java ASCII
representation (SDAR / SDAV) unless a bug is found in the Java side.

PC-1500/1500A only. The PC-1600 header format has no recognized type byte for
either payload (confirmed in the Java `Pc1600Header`, which only maps
BASIC/MACHINE); `--device pc1600`/`pc1600emul` for these types stays an error,
same as today.

Permanently out of scope (explicit user decision, do not revisit):
- `terminal` mode — the PC-1500/1600 serial link isn't full-duplex-capable
  (only input or output can be redirected at a time), so an interactive
  terminal has little value.
- `--add-utils` (serial utility BASIC sub-program injection on `put`).
- Any additional `--dry-run` support beyond what `get`/`put` already have
  (`convert --dry-run` stays unimplemented).
- Windows serial-port auto-detection — Windows machines routinely enumerate
  many COM ports (Bluetooth, virtual devices, etc.), making auto-detection
  unreliable; `--port` stays mandatory on Windows.

These four items should be moved from "not implemented" to a permanent
"won't implement" note in the README's Scope/Known Limitations section once
this doc is acted on.

## 1. Reserve Area (SDAR)

### 1.1 Binary format (wire payload, CE-158 header stripped)

188-byte fixed payload:

| Offset | Size | Content |
|---|---|---|
| 0 | 26 | Layer 1 label (CP437, null-padded) |
| 26 | 26 | Layer 2 label |
| 52 | 26 | Layer 3 label |
| 78 | 110 | Key-content pool (see below) |

Total: `3×26 + 110 = 188` bytes. CE-158 header: type char `'A'`, filename as
usual, `startAddr`/`runAddr` unused (0), `length` = 188 (stored on the wire as
`length - 1`, per the existing CE-158 length convention already implemented in
`src/header.rs`).

**Key-content pool**: a sequence of `[key code][tokenized content bytes]...`
entries, terminated by `0x00`. Key codes identify (layer, key) pairs:

| Layer | Keys 1–6 (key codes) |
|---|---|
| 1 | `0x01`–`0x06` |
| 2 | `0x11`–`0x16` |
| 3 | `0x09`–`0x0E` |

(Note layer 2 and 3 codes are not consecutive with layer 1 — port the table
exactly as in Java `ReserveAreaConverter.KEY_CODE`, do not re-derive it.)

Content bytes for a key run until the next key code (`0x01`–`0x06`,
`0x09`–`0x0E`, `0x11`–`0x16`) or a `0x00` terminator. Within content, `0xF0`
and `0xF1` are token-code prefix bytes — each introduces a 2-byte BASIC
keyword token (looked up against the PC-1500 keyword registry, i.e.
`sde`'s existing `src/keywords.rs`/`src/registry.rs`); all other bytes are
literal CP437 characters. A key with no content is simply absent from the
pool (no code, no bytes).

If the encoded pool exceeds 110 bytes, encoding to binary must fail with a
clear error (mirrors Java's `IllegalArgumentException` on pool overflow) —
this is a real hardware limit, not a soft guideline.

### 1.2 ASCII format (SDAR text)

```
; SDAR:1.0 pc1500
; Filename: MYAPP

[layer 1]
label: <label text>
key 1: <content, keywords as plain text>
key 2: <content>
key 3: <content>
key 4: <content>
key 5: <content>
key 6: <content>

[layer 2]
label: ...
key 1: ...
...

[layer 3]
...
```

- First line is always `; SDAR:1.0 pc1500` (device is always `pc1500` for
  this format — Reserve Area doesn't exist on PC-1600).
- `; Filename: <name>` is emitted only when a filename is known (from the
  CE-158 header on `get`); omitted otherwise.
- Three `[layer N]` sections in fixed order, each with `label:` then six
  `key N:` lines (always all six, even when empty — content after the colon
  can be blank).
- Content is the de-tokenized form: keyword tokens render as their full
  keyword name (e.g. `PRINT`), exactly like BASIC de-tokenization elsewhere
  in `sde`; everything else is the literal CP437-decoded character.
- Parsing (ASCII → binary) must accept the same shape; leading `;` comment
  lines besides the two above are ignored, blank lines between sections are
  ignored.
- `.sdar` is the file extension for this ASCII form (both for `get`'s
  auto-appended extension and for content/extension-agreement rules, matching
  how `.bas`/`.bbin`/`.bin` are already handled).

### 1.3 CLI integration

- `get`: content detection recognizes a CE-158 header with type char `'A'`
  (extend `src/header.rs`'s `FileType` and the header-type-byte match table,
  currently `'A'`/`'H'` are explicitly called out as "recognized on the wire
  but out of scope") → `Content::Ce158Reserve` (name TBD to match existing
  `Ce158Basic`/`Ce158Machine` style) in `src/detect.rs`. `--format binary`
  saves the payload as-is (header included by default, `--skip-header`
  behaves as it does today); default format converts to SDAR text.
- `put`: SDAR text input is detected the same way ASCII BASIC is today —
  first non-blank line matches `; SDAR:` — and encoded to binary + a fresh
  CE-158 Reserve header. A file that already carries a CE-158 Reserve header
  is sent as-is, same pattern as BASIC/machine language.
- `convert` is **not** extended — per the existing README scope statement,
  `convert` stays BASIC-only; Reserve/Variables only move over the wire via
  `get`/`put`, matching the Java tool's own CLI wiring (Reserve/Variables
  never go through Java's `runConvert` either).
- Round-trip (`get` producing SDAR, then `put` sending that SDAR back)
  must reproduce the original 188-byte payload exactly.

### 1.4 Test plan

- Port byte-parity fixture: Java's `src/test/resources/dumps/pc1500-reserve.bin`
  (or an equivalent captured dump) becomes a Rust test fixture under
  `tests/fixtures/`; assert `sde`'s SDAR text output matches Java's output
  (same rules as the existing `tests/byte_parity.rs` convert-parity tests).
- Round-trip test: binary → SDAR → binary, byte-identical to the input
  payload.
- Overflow test: pool content that exceeds 110 bytes is rejected with an
  error, not silently truncated.
- Empty-key test: a key with no content round-trips as an absent pool entry.

## 2. Variables (SDAV)

### 2.1 Binary format (wire payload, CE-158 header stripped)

Sequential records, no fixed total length, no trailing terminator — parsing
stops when the payload is exhausted:

```
[0x00][4-byte prefix][data bytes] [0x00][4-byte prefix][data bytes] ...
```

Each record:
- `0x00` separator byte.
- 4-byte prefix: `byte0` = total record length minus 1 (i.e. `4 + len(data) - 1`),
  `byte1` = dimension discriminator (`0x00` for a scalar, otherwise the array's
  max index, i.e. `DIM(dimMax)` was declared), `byte2` = always `0x00`,
  `byte3` = type byte: `0x88` = numeric, otherwise = string max length in
  bytes (`0x10`/16 for a scalar string).
- Data bytes: length = `(byte0 + 1) - 4`.

Four record shapes, from `byte1`/`byte3`:

| `byte1` (discriminator) | `byte3` (type) | Meaning | Data layout |
|---|---|---|---|
| `0x00` | `0x88` | Numeric scalar | 8-byte BCD value |
| `0x00` | length `L` | String scalar | `L` bytes (16 for a plain scalar) |
| `dimMax` (>0) | `0x88` | Numeric array `DIM(dimMax)` | `(dimMax+1) × 8` bytes, 8-byte BCD elements |
| `dimMax` (>0) | `L` | String array `DIM$(dimMax)*L` | `(dimMax+1) × L` bytes, `L`-byte slots |

**BCD numeric codec — new, does not exist yet in `sde`.** `src/` has no
BCD/numeric module today; this must be ported from
`ch.erzberger.sharpbasic.core.numeric.Pc1500NumericCodec` in the
`SharpBasicShared`/`sharp-basic-core` library (a separate sister repo, not
`SharpDataExchange` itself — used by the Java tool as a dependency). Format
(PC-1500 Technical Reference Manual §5-3-1/§5-3-2), 8 bytes total:

| Byte | Content |
|---|---|
| 0 | Exponent, signed 8-bit two's complement, range −99..+99 |
| 1 | Sign: `0x00` positive/zero, `0x80` negative |
| 2–6 | Mantissa, 5 bytes packed BCD = 10 digits, decimal point after digit 1 |
| 7 | Always `0x00` |

Value = `sign × d.ddddddddd × 10^exponent`. All-zero 8 bytes decodes to `"0"`.

Also recognize on decode (but never produce on encode — SDAV never needs to
round-trip it, Java notes no real CSAVE dump has ever produced one) a
secondary binary-integer variant: when byte 4 is `0xB2`, bytes 5–6 are a
big-endian signed 16-bit integer and the rest of the record is don't-care.

Decode formatting: strip trailing zeros; plain decimal for exponents in
`-4..9`; scientific notation (`<mantissa>E<+/-><exp>`) outside that range —
port Java's `formatDecimal` behavior exactly, since SDAV text is compared for
parity against the Java tool's output.

Encode: round the input to 10 significant digits (half-up) before packing,
and reject (error, not silent clamp) values whose resulting exponent falls
outside −99..+99 — this is a real hardware range limit.

### 2.2 ASCII format (SDAV text)

```
; SDAV:1.0 pc1500
; Count: <N>
; Filename: MYAPP
<value-line>
<value-line>
...
```

- `; SDAV:1.0 pc1500` first line, `; Count: <N>` second line (N = number of
  top-level records, i.e. one count per scalar or per whole array, not per
  array element), `; Filename:` third line when known.
- Value lines, one per record in wire order:
  - Numeric scalar → plain decimal number (e.g. `3.14`).
  - String scalar → double-quoted string, with `\\`, `\"`, and non-printable/
    non-ASCII bytes (`< 0x20` or `> 0x7E`) escaped as `\xHH`.
  - Numeric array → a `DIM (<dimMax>)` header line followed by `dimMax+1`
    plain-decimal value lines.
  - String array → a `DIM $(<dimMax>)*<maxLen>` header line followed by
    `dimMax+1` quoted/escaped string lines.
- Parsing (ASCII → binary) must recompute the record count and error out if
  it disagrees with a declared `; Count:` line (mirrors Java's hard
  mismatch check) rather than silently trusting one or the other.
- `.sdav` is the file extension for this ASCII form.

### 2.3 CLI integration

Same pattern as Reserve Area §1.3: extend `FileType`/`Content` for CE-158
type char `'H'`, wire into `get` (binary/SDAV text output) and `put` (SDAV
text input, or pass-through of an already-headered binary file). Not wired
into `convert`. Note from the Java header handling: the CE-158 length field
for a Variables header is not meaningful (Java always writes it as `1`
regardless of actual payload size) — port that exact quirk rather than trying
to compute a "correct" length, since the real device apparently ignores it
for this type.

### 2.4 Test plan

- Round-trip test: binary → SDAV → binary, byte-identical, across all four
  record shapes (numeric scalar, string scalar, numeric array, string array)
  and mixed sequences of them.
- Count-mismatch test: a hand-edited SDAV with a wrong `; Count:` is rejected.
- Escaping test: strings containing `\`, `"`, control bytes, and high-CP437
  bytes round-trip through the `\xHH` escape correctly.
- Port Java's `VariablesConverterTest` fixtures/cases where practical, for
  cross-implementation parity the same way `pc1500-reserve.bin` is used for
  Reserve Area.

## 3. Shared groundwork

- `src/header.rs`: extend `FileType` with `Reserve`/`Variables` variants,
  remove the current explicit "out of scope" carve-out for type chars `'A'`/
  `'H'`, and wire the length-field quirks noted above (Reserve: real length;
  Variables: always encode as `1`).
- `src/detect.rs`: extend `Content` with the two new binary variants (CE-158
  only — no PC-1600 equivalent) and the two new ASCII markers (`; SDAR:`,
  `; SDAV:`), following the same detection-priority order as Java's
  `ContentDetector` (header match first, then ASCII marker, before falling
  through to the ASCII-BASIC heuristic — SDAR/SDAV must be checked before the
  BASIC line-number heuristic to avoid misdetection).
- New modules, e.g. `src/reserve.rs` and `src/variables.rs`, mirroring the
  Java `ReserveAreaConverter`/`VariablesConverter` split, plus a new
  `src/numeric.rs` (or similar) porting `Pc1500NumericCodec` — this one has
  no Rust counterpart yet and is a prerequisite for Variables encode/decode.
- `get_cmd.rs`/`put_cmd.rs`: extend the dispatch matches to route the two new
  content types, and `filename.rs`/extension logic for `.sdar`/`.sdav`.
- README: add both formats to "Data Formats", remove them from "Scope /
  Known Limitations", and add the four permanently-skipped items (terminal
  mode, `--add-utils`, extra `--dry-run`, Windows auto-detect) to a
  "Won't implement" note instead of leaving them as open gaps.
