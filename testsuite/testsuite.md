# Manual test suite

Run these tests before each release, in addition to the unit tests.

**Notes:**

- `sde` is a shortcut for `java -jar ../target/SharpDataExchange.jar "$@"`
- All test programs are in the `testsuite/` directory alongside this file.
- SharpDataExchange detects file format from content automatically — no format flags needed for the common cases.
- `put` always sends to the Pocket Computer; `get` always receives from it. There is no disk-to-disk conversion mode.
- `get` detokenizes BINARY_BASIC to ASCII by default. Use `--format binary` to save raw bytes instead.
- Pocket Computer commands are shown in parentheses after the corresponding `sde` command.

---

## PC-1500

### BASIC — put (Pocket Computer receives)

**Put an ASCII BASIC program:**

```
sde put depreciation.bas
```
PC-1500: `CLOAD` — program should load and run correctly. Re-initialize serial with `Def-J` between tests if needed.

**Put an ASCII BASIC program with the serial utility sub-program prepended:**

```
sde put --add-utils depreciation.bas
```
PC-1500: `CLOAD` — program loads; the utility routines appear at the start of the program listing.

> Note: `--add-utils` is not yet implemented and will log a warning; the program is sent without utilities in the meantime.

**Put a binary BASIC program that already has a CE-158 header:**

```
sde put depreciation-tokenized-ce158header.bin
```
PC-1500: `CLOAD` — identical to the ASCII put above; SharpDataExchange detects the CE-158 header and sends the file as-is.

### BASIC — get (Pocket Computer sends)

**Get a BASIC program as ASCII (default):**

PC-1500: `CSAVE"DEPRECIATION"`, then:

```
sde get deptest.bas
```
Compare `deptest.bas` with `depreciation.bas`. Content should be identical; spacing after keywords may differ because the detokenizer uses a normalized form (one space after each keyword).

**Get a BASIC program as raw binary:**

PC-1500: `CSAVE"DEPRECIATION"`, then:

```
sde get --format binary deptest.bin
```
Compare `deptest.bin` with `depreciation-tokenized-ce158header.bin`. There will be minor differences in two header bytes that the PC-1500 writes as internal memory pointers (documented as "don't care" in the CE-158 manual) — this is expected.

### BASIC — line length test

Line 20 of `LineLengthTest.bas` is 217 characters long. Tokenized, it fits within the PC-1500's 254-byte line buffer; in ASCII, it does not.

**Verify the tokenized load succeeds:**

```
sde put LineLengthTest.bas
```
PC-1500: `CLOAD` — should succeed. `LIST 20` shows the full line.

**Verify that a raw ASCII load fails:**

PC-1500: `CLOADa`, then send the file from another tool in plain text (or type the line manually). The PC-1500 should reject it with `ERROR 67` (line too long).

> The point: SharpDataExchange tokenizes automatically when loading ASCII BASIC, so overlong lines that fit in binary are handled transparently.

### Reserve Area — round-trip

**Get Reserve Area data:**

PC-1500: program the reserve keys (e.g. define labels and key content), then `CSAVE"MYRESERVE",A`, then:

```
sde get reserve.sdar
```
`reserve.sdar` will be an SDAR text file showing the three layers of key labels and content with BASIC keywords detokenized.

**Edit and put back:**

Edit `reserve.sdar` (change a key label or content), then:

```
sde put reserve.sdar
```
PC-1500: `CLOAD,A` — the updated key definitions appear in the reserve area.

### Variables — round-trip

**Get Variables:**

PC-1500: set some variables and `CSAVE"MYVARS",V`, then:

```
sde get vars.sdav
```
`vars.sdav` will be an SDAV text file listing each variable value in order.

**Put Variables back:**

```
sde put vars.sdav
```
PC-1500: `CLOAD,V` (variables must be pre-DIM'd in the correct order if arrays are used) — values are restored.

---

## PC-1600

All PC-1600 commands require `--device pc1600` (or `-d pc1600`).

### BASIC — put

**Put an ASCII BASIC program:**

```
sde put --device pc1600 depreciation.bas
```
PC-1600: `LOAD "COM1:"` — program loads and runs correctly. Re-initialize the COM port with `Def-J` between tests if needed.

**Put a binary BASIC program with a PC-1600 header:**

```
sde put --device pc1600 depreciation-tokenized-pc1600header.bin
```
PC-1600: `LOAD "COM1:"` — identical result; SharpDataExchange detects the PC-1600 header and sends as-is.

### BASIC — get

**Get a BASIC program as ASCII:**

PC-1600: `Def-S` (or `SAVE "COM1:"`), then:

```
sde get --device pc1600 deptest.bas
```
Compare `deptest.bas` with `depreciation.bas` (content should match, spacing may differ).

**Get a BASIC program as raw binary:**

PC-1600: `SAVE "COM1:"`, then:

```
sde get --format binary --device pc1600 deptest.bin
```
Load it back immediately:

```
sde put --device pc1600 deptest.bin
```
PC-1600: `Def-L` — the same program loads back.

---

## Test programs

| File | Description |
|---|---|
| `depreciation.bas` | ASCII BASIC — Depreciation calculator (Sharp PC-1500 Applications Manual, Program P5-D-12). Last line number adjusted to avoid conflict with the utility sub-program. |
| `depreciation-tokenized-ce158header.bin` | Same program, tokenized, with CE-158 header (PC-1500 format). |
| `depreciation-tokenized-pc1600header.bin` | Same program, tokenized, with PC-1600 header. |
| `LineLengthTest.bas` | Two-line program where line 20 is 217 characters — fits when tokenized, too long for ASCII `CLOADa`. |

> **Not included:** `depreciation-tokenized-raw.bin` (tokenized without any header) is not usable with SharpDataExchange because content-based detection cannot identify a headerless binary blob; it would be reported as `UNKNOWN`.

---

## Dropped tests from SharpCommunicator

The following SharpCommunicator tests have no equivalent in SharpDataExchange:

| Old test | Reason dropped |
|---|---|
| Load tokenized binary without header | Format detected as `UNKNOWN`; headerless binary is not supported |
| Compact output to clipboard (`-o clip`) | Clipboard is not supported |
| Offline tokenization (`-i file.bas -o file.bin`) | No disk-to-disk conversion mode; `put` always sends to the Pocket Computer |
