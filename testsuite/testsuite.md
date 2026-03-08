# Manual test suite

Run these tests before each release, in addition to the unit tests.

**Notes:**

- `sde` is a shortcut for `java -jar ~/Applications/PocketPc/SharpDataExchange.jar "$@"`
- All test programs are in the `testsuite/` directory alongside this file.
- SharpDataExchange detects file format from content automatically — no format flags needed for the common cases.
- `put` always sends to the Pocket Computer; `get` always receives from it. There is no disk-to-disk conversion mode.
- `get` detokenizes BINARY_BASIC to ASCII by default. Use `--format binary` to save raw bytes instead.
- For `put`: start the Pocket Computer command first (so it is ready to receive), then run `sde put`.
- For `get`: run `sde get` first (so it is listening), then trigger the Pocket Computer to send.

---

## PC-1500

### BASIC — put (Pocket Computer receives)

**Put an ASCII BASIC program:**

PC-1500: `CLOAD`

```
sde put depreciation.bas
```
Program should load and run correctly. Re-initialize serial with `Def-J` between tests if needed.

**Put an ASCII BASIC program with the serial utility sub-program prepended:**

PC-1500: `CLOAD`

```
sde put --add-utils depreciation.bas
```
Program loads; the utility routines appear at the end of the program listing (line numbers 61000+).

**Put a binary BASIC program that already has a CE-158 header:**

PC-1500: `CLOAD`

```
sde put depreciation-tokenized-ce158header.bin
```
Identical result to the ASCII put above; SharpDataExchange detects the CE-158 header and sends the file as-is.

### BASIC — get (Pocket Computer sends)

**Get a BASIC program as ASCII (default):**

```
sde get deptest.bas
```
PC-1500: `CSAVE"DEPRECIATION"`

Compare `deptest.bas` with `depreciation.bas`. Content should be identical; spacing after keywords may differ because the detokenizer uses a normalized form (one space after each keyword).

**Get a BASIC program as raw binary:**

```
sde get --format binary deptest.bin
```
PC-1500: `CSAVE"DEPRECIATION"`

Compare `deptest.bin` with `depreciation-tokenized-ce158header.bin`. There will be minor differences in two header bytes that the PC-1500 writes as internal memory pointers (documented as "don't care" in the CE-158 manual) — this is expected.

### BASIC — line length test

Line 20 of `LineLengthTest.bas` is 217 characters long. Tokenized, it fits within the PC-1500's 254-byte line buffer; in ASCII, it does not.

**Verify the tokenized load succeeds:**

PC-1500: `CLOAD`

```
sde put LineLengthTest.bas
```
Should succeed. `LIST 20` on the PC-1500 shows the full line.

**Verify that a raw ASCII load fails:**

PC-1500: `CLOADa`, then send the file from another tool in plain text (or type the line manually). The PC-1500 should reject it with `ERROR 67` (line too long).

> The point: SharpDataExchange tokenizes automatically when loading ASCII BASIC, so overlong lines that fit in binary are handled transparently.

### Reserve Area — round-trip

**Get Reserve Area data:**

```
sde get reserve.sdar
```
PC-1500: `CSAVE"MYRESERVE",A`

`reserve.sdar` will be an SDAR text file showing the three layers of key labels and content with BASIC keywords detokenized.

**Edit and put back:**

Edit `reserve.sdar` (change a key label or content), then:

PC-1500: `CLOAD,A`

```
sde put reserve.sdar
```
The updated key definitions appear in the reserve area.

### Variables — round-trip

**Get Variables:**

```
sde get vars.sdav
```
PC-1500: `CSAVE"MYVARS",V`

`vars.sdav` will be an SDAV text file listing each variable value in order.

**Put Variables back:**

PC-1500: `CLOAD,V` (variables must be pre-DIM'd in the correct order if arrays are used)

```
sde put vars.sdav
```
Values are restored.

---

## PC-1600

All PC-1600 commands require `--device pc1600` (or `-d pc1600`).

### BASIC — put (Pocket Computer receives)

**Put an ASCII BASIC program:**

PC-1600: `LOAD "COM1:"`

```
sde put --device pc1600 depreciation.bas
```
Program loads and runs correctly. Re-initialize the COM port with `Def-J` between tests if needed.

**Put a binary BASIC program with a PC-1600 header:**

PC-1600: `LOAD "COM1:"`

```
sde put --device pc1600 depreciation-tokenized-pc1600header.bin
```
Identical result; SharpDataExchange detects the PC-1600 header and sends as-is.

### BASIC — get (Pocket Computer sends)

**Get a BASIC program as ASCII:**

```
sde get --device pc1600 deptest.bas
```
PC-1600: `Def-S` (or `SAVE "COM1:"`)

Compare `deptest.bas` with `depreciation.bas` (content should match, spacing may differ).

**Get a BASIC program as raw binary, then load it back:**

```
sde get --format binary --device pc1600 deptest.bin
```
PC-1600: `SAVE "COM1:"`

Then load it back:

PC-1600: `Def-L`

```
sde put --device pc1600 deptest.bin
```
The same program loads back.

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
