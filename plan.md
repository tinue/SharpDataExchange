# SharpDataExchange - Implementation Plan

## Context

SharpCommunicator (the existing reference app) has a confusing CLI using `--in-file`/`--out-file` pairs with no clear direction, mixes disk-to-disk operations, supports stdin/stdout, and duplicates BASIC parsing logic now centralized in SharpBasicShared. This is a complete rewrite with clearer architecture, a simpler verb-based CLI, content-based format detection, ASCII-first defaults, and the Pocket PC always as a communication party.

---

## Step 1: Develop and Document the CLI

### Verb-based model

```
java -jar SharpDataExchange.jar get [options] <output-file>
java -jar SharpDataExchange.jar put [options] <input-file>
```

- `get` — Receive data from Pocket PC, write to `<output-file>` on disk
- `put` — Read `<input-file>` from disk, send to Pocket PC
- The Pocket PC serial port is always implicit; no disk-to-disk mode

### Full option set

```
Usage:
  get [options] <output-file>
      Receive data from the Pocket Computer and save it to <output-file>.

  put [options] <input-file>
      Read data from <input-file> and send it to the Pocket Computer.

Common options:
  -d, --device <device>      Device: pc1500 (default), pc1500a, pc1600
  -p, --port <port>          Serial port (auto-detected if omitted)
  -v, --verbose              Verbose logging
  -vv, --debug               Debug logging
  -V, --version              Print version and exit
  -h, --help                 Print this help

Options for `get`:
  -f, --format <format>      Output format: ascii (default), binary

Options for `put`:
  -f, --format <format>      Override detected input format: ascii, binary
      --start-address <hex>  Machine language load address (hex, e.g. 38C5)
      --run-address <hex>    Machine language auto-run address
      --add-utils            Prepend serial utility BASIC sub-program
```

### Design rationale

- `get`/`put` avoids the CSAVE/CLOAD vs. save/load ambiguity: from the PC's perspective it simply gets or puts data
- ASCII is the default output of `get` for all data types (BASIC, Reserve, Variables)
- For `put`, format is auto-detected from file content; `--format` is an override escape hatch
- No clipboard, no stdin/stdout; always a named file

---

## Step 2: Maven Project Setup

**Single-module project** (no multi-module overhead; one deliverable: a fat JAR)

```
SharpDataExchange/
├── pom.xml
└── src/
    ├── main/
    │   ├── java/ch/erzberger/sharppc/exchange/
    │   │   ├── SharpDataExchange.java        # entry point / orchestrator
    │   │   ├── cli/
    │   │   │   ├── CliArgs.java              # record DTO
    │   │   │   ├── CliParser.java            # commons-cli wiring
    │   │   │   ├── OutputFormat.java         # ASCII | BINARY
    │   │   │   └── PocketPcDevice.java       # PC1500 | PC1500A | PC1600
    │   │   ├── serial/
    │   │   │   ├── ByteProcessor.java        # port from SharpCommunicator
    │   │   │   ├── Watchdog.java             # port from SharpCommunicator
    │   │   │   ├── SerialPortWrapper.java    # port, add optional portName arg
    │   │   │   ├── DataReceiver.java         # rename of ReadFromPocketPc
    │   │   │   └── DataSender.java           # rename of SerialToDeviceSender
    │   │   ├── header/
    │   │   │   ├── SerialHeader.java         # port abstract base
    │   │   │   ├── Ce158Header.java          # port PC-1500 header
    │   │   │   └── Pc1600Header.java         # port PC-1600 header (fix TODO)
    │   │   ├── detect/
    │   │   │   └── ContentDetector.java      # content-based type detection
    │   │   ├── convert/
    │   │   │   ├── DataType.java             # enum: BINARY_BASIC, ASCII_BASIC,
    │   │   │   │                             #   BINARY_RESERVE, ASCII_RESERVE,
    │   │   │   │                             #   BINARY_VARS, ASCII_VARS,
    │   │   │   │                             #   MACHINE, UNKNOWN
    │   │   │   ├── AsciiBasicTokenizer.java     # thin wrapper on SharpBasicShared
    │   │   │   ├── ReserveAreaConverter.java    # NEW: binary <-> ASCII Reserve
    │   │   │   └── VariablesConverter.java      # NEW: binary <-> ASCII Variables
    │   │   │   # NOTE: BinaryBasicDetokenizer lives in SharpBasicShared library
    │   │   └── io/
    │   │       └── FileHandler.java          # port, drop clipboard support
    │   └── resources/
    │       └── logging.properties
    └── test/java/ch/erzberger/sharppc/exchange/
        ├── cli/CliParserTest.java
        ├── serial/WatchdogTest.java
        ├── header/Ce158HeaderTest.java
        ├── header/Pc1600HeaderTest.java
        ├── detect/ContentDetectorTest.java
        └── convert/
            ├── ReserveAreaConverterTest.java
            └── VariablesConverterTest.java
```

### pom.xml key dependencies

```xml
<groupId>ch.erzberger</groupId>
<artifactId>SharpDataExchange</artifactId>
<version>1.0.0-SNAPSHOT</version>
<java.version>21</java.version>

jSerialComm 2.11.2        (serial communication)
commons-cli 1.10.0        (CLI argument parsing)
sharp-basic-core          (KeywordRegistry, BasicKeyword, AbbreviationExpander)
sharp-basic-antlr         (BinaryBasicDetokenizer, BinaryEncodingVisitor, NormalizedTextVisitor, etc.)
lombok 1.18.x             (compile-scope only)
junit-jupiter 5.x         (test)
maven-shade-plugin        (fat JAR with Main-Class manifest)
```

SharpBasicShared modules must be `mvn install`-ed locally before building.

---

## Step 3: Architecture Design

### Two-step data flow (strict I/O / logic separation)

```
STEP 1 — READ ALL RAW BYTES
  get:  SerialPortWrapper + DataReceiver  ->  byte[]
  put:  FileHandler.readBinaryFile()      ->  byte[]

STEP 2 — DETECT -> CONVERT -> WRITE
  ContentDetector.detect(byte[])          ->  DataType

  get path:
    BINARY_BASIC   -> BinaryBasicDetokenizer (SharpBasicShared) -> ASCII lines -> FileHandler.writeText()
                   or                                           -> byte[]       -> FileHandler.writeBinary()
    BINARY_RESERVE -> ReserveAreaConverter  -> SDAR text        -> FileHandler.writeText()
    BINARY_VARS    -> VariablesConverter    -> SDAV text        -> FileHandler.writeText()
    ASCII_BASIC    -> write as-is
    MACHINE        -> FileHandler.writeBinary() (binary only)

  put path:
    ASCII_BASIC    -> AsciiBasicTokenizer (SharpBasicShared)  -> byte[] -> DataSender
    ASCII_RESERVE  -> ReserveAreaConverter.fromAscii()        -> byte[] -> DataSender
    ASCII_VARS     -> VariablesConverter.fromAscii()          -> byte[] -> DataSender
    BINARY_*       -> ensure header present                   -> DataSender
    MACHINE        -> prepend header if missing               -> DataSender
```

`DataSender`/`DataReceiver` know nothing about BASIC format — clean boundary.

### Content-based detection order (`ContentDetector`)

1. **CE-158 magic**: `data[0]==0x01 && data[2..4]=="COM"` — type from `data[1]`: `@`=BINARY_BASIC, `A`=BINARY_RESERVE, `B`=MACHINE, `H`=BINARY_VARS
2. **PC-1600 magic**: `data[0..3]=={0xFF,0x10,0x00,0x00}` — type from `data[4]`: `0x21`=BINARY_BASIC, `0x10`=MACHINE
3. **ASCII BASIC**: valid text, first 3-5 lines match `[0-9]+ .*` pattern
4. **ASCII Reserve**: first line matches `; SDAR:1.0 ...`
5. **ASCII Variables**: first line matches `; SDAV:1.0 ...`
6. **UNKNOWN** fallback

---

## Step 4: Serial Data Exchange Layer

Port from SharpCommunicator (`sharp-pocket-computer/SharpCommunicator/src/main/java/ch/erzberger/serialhandler/`):

| New class | Source | Changes |
|---|---|---|
| `ByteProcessor` | port as-is | none |
| `Watchdog` | port as-is | none |
| `SerialPortWrapper` | port | add optional `portName` constructor arg |
| `DataReceiver` | `ReadFromPocketPc` | rename; PC-1500 timeout=5000ms, PC-1600=500ms |
| `DataSender` | `SerialToDeviceSender` | rename; PC-1500: 1ms/byte delay, 300ms+500ms pauses; PC-1600: full speed |

Device baud rates: PC-1500/PC-1500A=19200, PC-1600=9600. Flow control: PC-1500=none, PC-1600=RTS/CTS.

---

## Step 5: Application Implementation

### De-tokenization — lives in SharpBasicShared

`BinaryBasicDetokenizer` is implemented in the SharpBasicShared library (see `SharpBasicShared/task-detokenizer.md`). SharpDataExchange uses it as a library call, the same way it uses `BinaryEncodingVisitor` for the forward direction.

In SharpDataExchange, the `get` path for `BINARY_BASIC` calls:

```java
BinaryBasicDetokenizer detokenizer = new BinaryBasicDetokenizer(registry);
List<String> asciiLines = detokenizer.detokenize(payloadBytes);
```

### Binary format: Reserve Area (CE-158 payload, type 'A')

Source: Sharp PC-1500 Technical Reference Manual §5-3-6.

The payload is **187 bytes** (confirmed by hardware dump), starting at memory address
4008H (the 8-byte ROM status block at 4000H–4007H is machine-specific configuration and
is not included — the CE-158 start-address field for RESERVE type contains 0x0008,
the offset into the reserve area, confirming the 4008H start):

| Payload offset | Memory address | Size | Content |
|---|---|---|---|
| 0x000 | 4008H | 26 bytes | Key symbol (label) for layer I — null-padded 7-bit CP437 string |
| 0x01A | 4022H | 26 bytes | Key symbol (label) for layer II |
| 0x034 | 403CH | 26 bytes | Key symbol (label) for layer III |
| 0x04E | 4056H | 110 bytes | Key contents pool |

**Key symbol format**: 26 bytes; the label string in 7-bit CP437, null-terminated and
padded with 00H to fill the 26 bytes. Example: `"SIN COS PRI ABC DEF IFK"` followed by
three 00H bytes. Filename is null-padded per §13 of the Technical Reference Manual
("blank portions will be padded with NULL codes (00 hex)"); `trim()` strips them on read.

**Key contents pool** (111 bytes): a flat stream of entries, one final 00H terminator:

```
[key_code] [content_bytes...] [key_code] [content_bytes...] ... [00H]
```

- **Key code byte** identifies which layer and key slot the entry belongs to:

  | Key | Layer I | Layer II | Layer III |
  |---|---|---|---|
  | F1 | 01H | 11H | 09H |
  | F2 | 02H | 12H | 0AH |
  | F3 | 03H | 13H | 0BH |
  | F4 | 04H | 14H | 0CH |
  | F5 | 05H | 15H | 0DH |
  | F6 | 06H | 16H | 0EH |

- **Content bytes**: BASIC keywords stored as standard PC-1500 BASIC tokens (two bytes:
  F0H+xx or F1H+xx); plain characters stored as 7-bit CP437 codes (20H–7FH). Example:
  `GOTO` = `F1 92`; `@` = `40H`; `GOTO@` = `F1 92 40`.
- **Parsing**: content bytes are always ≥ 20H or start with F0H/F1H (all > 16H); key
  codes are always in 01H–16H. No ambiguity — the parser can reliably distinguish them.
- **Entry order**: registration order (not sorted by key code). On re-registration, the
  old entry is deleted and the new one appended.
- **00H** terminates the entire pool. Unused bytes in the 111-byte pool are 00H.
- **Size limit**: total pool content (all entries + final 00H) must not exceed 110 bytes.
  `ReserveAreaConverter` must check this when converting SDAR→binary.

> **CE-158 length field encoding (§13 of Technical Reference Manual, confirmed by dump):**
> The length field stores **capacity − 1** (i.e., `actual_payload_length − 1`). When
> reading: `length = wire_value + 1`. When writing: `wire_value = length − 1`.
> File structure is simply `[27-byte header][length bytes]` — no checksum byte.
> Confirmed: dump file is 215 bytes = 27 header + 188 payload; wire length field = 0x00BB = 187 = 188−1.
> `Ce158Header.java` implements this encoding. There is no CE-158 checksum in the file.

### ASCII format: Reserve Area (SDAR)

The PC-1500 Reserve Area stores three layers of key definitions for the six reserve keys.
Each layer has a label (typically a short string that identifies the keys on that layer,
e.g. `ABS FOR SIN COS TAN ATN`) and six key slots. Key content may include BASIC keywords,
which are de-tokenized when writing and re-tokenized when reading back.

```
; SDAR:1.0 pc1500
; Filename: MYAPP

[layer 1]
label: ABS FOR SIN COS TAN ATN
key 1: ABS(
key 2: FOR
key 3: SIN(
key 4: COS(
key 5: TAN(
key 6: ATN(

[layer 2]
label: IO
key 1: LOAD "
key 2: SAVE "
key 3: NEW
key 4:
key 5:
key 6:

[layer 3]
label:
key 1:
key 2:
key 3:
key 4:
key 5:
key 6:
```

- `; SDAR:1.0 <device>` — first line, signals type for detector
- `; Filename: <name>` — optional; carries CE-158 filename for round-trip fidelity
- `[layer N]` — section header; exactly three layers (1, 2, 3), in order
- `label:` — one per layer; free-form text the user chooses to identify the keys;
  typically something like `ABS FOR SIN COS TAN ATN`; empty is allowed;
  taken as-is from the file (alignment with keys is the user's responsibility)
- `key 1:` … `key 6:` — all six must be present in each layer, even if empty;
  value is the de-tokenized key content; BASIC keywords appear as full keyword names
- BASIC keywords de-tokenized on write (binary→SDAR), re-tokenized on read (SDAR→binary)
- Comment (`;`) and blank lines ignored anywhere in the file
- **Size limit**: the `ReserveAreaConverter` must verify that the total tokenized binary
  pool (all entries + 00H terminator) fits within 110 bytes; reject with a clear error if not

### ASCII format: Variables (SDAV)

Variable names are **not stored in the binary format** and are therefore **not present in
the SDAV format either**. The tape is purely positional; the type and structure of each
record is self-evident from line syntax.

**Example — mixed scalars and arrays:**

```
; SDAV:1.0 pc1500
; Count: 6
; Filename: MYAPP
3.14159265
"HELLO"
0
""
DIM $(1)*80
""
"long string here"
DIM $(2)*40
""
"foo"
"bar"
```

**Rules:**

- `; SDAV:1.0 <device>` — first line, signals type for detector
- `; Count: <n>` — number of binary records (for validation). Each scalar = 1 record.
  Each `DIM` array = 1 record regardless of element count.
- `; Filename: <name>` — optional; carries CE-158 filename for round-trip fidelity
- Comment (`;`) and blank lines ignored anywhere in the file
- `<decimal>` — numeric scalar; up to 10 significant digits; scientific notation for
  very large/small values (e.g. `1E-9`)
- `"<string>"` — simple string scalar (16-byte slot); see string escaping below
- `DIM $(<dim_max>)*<max_len>` — introduces one DIM'd **string** array record; `*<max_len>`
  is mandatory in SDAV (optional in PC-1500 BASIC, but required here to reconstruct the
  binary slot size); must be followed by exactly `dim_max + 1` quoted string lines
  (elements 0 to `dim_max`)
- `DIM (<dim_max>)` — introduces one DIM'd **numeric** array record; no `$`, no `*L`
  (forbidden); must be followed by exactly `dim_max + 1` decimal lines
- `dim_max` may be 0 — a single-element array loaded into e.g. a variable declared with
  `DIM A(0)`; elements are always indexed from 0
- Quoted string lines after a `DIM $` — array elements in index order; value is
  null-padded to `max_len` in binary (no null terminator if value fills the slot exactly)
- Decimal lines after a `DIM (` — numeric array elements in index order

**String escaping** (applies to all quoted strings — scalars and array elements):

| Sequence | Meaning |
|---|---|
| `\\` | Literal backslash |
| `\"` | Literal double-quote (e.g. entered on Sharp via `CHR$(34)`) |
| `\xHH` | Byte with hex value HH (two uppercase hex digits) |

Any other character is stored as-is (7-bit CP437). On `get`, only characters that require
escaping (`\`, `"`, and bytes < 0x20 or > 0x7E) are escaped; all others are written
literally.

### Binary format: numeric variable value (8 bytes, §5-3-1 / §5-3-2)

Two possible encodings — distinguished by byte 4:

**Decimal (floating point)** — byte 4 ≠ B2H:

| Byte | Content |
|---|---|
| 0 | Exponent — signed 8-bit two's complement, range −99 to +99 |
| 1 | Mantissa sign — 00H = positive, 80H = negative |
| 2–6 | Mantissa — 5 bytes packed BCD, 10 digits; implicit decimal point after first digit |
| 7 | Always 00H |

Value = sign × (BCD mantissa as 1.xxxxxxxxx) × 10^exponent

Examples: `03 00 15 00 00 00 00 00` = 1500; `FD 00 12 34 56 78 90 00` = 0.001234567890;
`08 80 12 34 00 00 00 00` = −1.234×10⁸

**Binary integer** — byte 4 == B2H:

| Byte | Content |
|---|---|
| 0–3 | Don't care |
| 4 | B2H (type marker) |
| 5–6 | 16-bit two's complement integer, big-endian, range −32768 to +32767 |
| 7 | Don't care |

Examples: `xx xx xx xx B2 05 DC xx` = 1500; `xx xx xx xx B2 FF FB xx` = −5

**String variable — memory layout only (§5-3-3):**

The D0H / pointer / string-buffer structure described in §5-3-3 is the **in-RAM layout**,
not the tape format. Key points for context:

- Single-letter strings (A$–Z$) have **fixed length and fixed memory address** — no pointer needed.
- DIM'd string arrays use a separate slot-based tape format (see array prefix below).
- In-memory strings are therefore stored either directly at a fixed location (A$–Z$) or via
  a pointer to a separate string buffer (DIM arrays).

**CE-158 tape format for variables — confirmed from hardware dumps:**

Four dumps captured and committed to `src/test/resources/dumps/`:

| File | Contents |
|---|---|
| `pc1500-vars-numeric.bin` | 5 numeric variables: +1.7534, −1.7534, π, 1×10⁻⁹, 0 |
| `pc1500-vars-strings.bin` | 4 string variables: "Hi there!", "B$", "", "last string" |
| `pc1500-vars-mixed.bin` | 2 numeric + 2 string interleaved: 1.5, "HI", 0, (string) |
| `pc1500-vars-longstrings-arrays.bin` | 2 DIM'd string arrays: `AA$(1)*80` (1 element, max 80 chars), `BB$(2)*40` (2 elements, max 40 chars) |
| `pc1500-vars-mixed-arrays.bin` | 2 numeric arrays (`DIM A(5)` with 6 elements, `DIM A(0)` with 1 element) + 1 string array (`DIM A$(1)*16`) — confirms numeric array prefix format |

The tape payload is a **sequential list of variable records**, one per saved variable (or one
per DIM'd array), with no variable names — purely positional (confirmed). Each record is
preceded by a `0x00` separator byte (including the first). The positional `INPUT#` behaviour
is confirmed.

**Payload structure:**

```
[0x00] [4-byte prefix] [data]  ← repeated for each variable / array
```

The 4-byte prefix is self-describing. **Discriminators: prefix byte 1 and byte 3.**

| byte 1 | byte 3 | Interpretation |
|---|---|---|
| `0x00` | `0x88` | Simple numeric scalar — OR `DIM A(0)`, binary identical |
| `0x00` | `0x10` | Simple string scalar (16-byte slot) — OR `DIM A$(0)*16`, binary identical |
| `0x00` | other | `DIM A$(0)*L` with max_len = byte 3 |
| non-zero | `0x88` | Numeric array: dim_max = byte 1 |
| non-zero | other | String array: dim_max = byte 1, max_len = byte 3 |

**Simple variable prefix** (byte 1 = `0x00`):

| Byte | Content |
|---|---|
| 0–1 | Total record length − 1, little-endian (record = prefix + data) |
| 2 | Always `0x00` |
| 3 | Type: `0x88` = float/BCD, `0x10` = string |

No ambiguity: simple-var records are at most 20 bytes, so byte 1 of the LE16 len field is
always `0x00`, matching the discriminator rule.

**Numeric record** — prefix `0B 00 00 88`, data 8 bytes, total record 12 bytes:

Same 8-byte BCD value as documented above (exponent, sign, 5-byte BCD, 0x00).

**String record** — prefix `13 00 00 10`, data 16 bytes, total record 20 bytes:

The string content stored left-aligned, null-padded to exactly 16 bytes. No length prefix.
Maximum observable string length is 16 characters; the 16-byte buffer is the fixed buffer
size for single-letter string variables (A$–Z$) in the Variable Area.

**DIM'd string array prefix** (byte 1 ≠ `0x00`):

| Byte | Content |
|---|---|
| 0 | Total record length − 1 (single byte; fits because arrays can be at most 256+4 bytes for reasonable DIM sizes) |
| 1 | `dim_max`: N from `DIM X$(N)*L` (the maximum subscript; elements run from 0 to N) |
| 2 | Always `0x00` |
| 3 | `max_len`: L from `DIM X$(N)*L` (maximum string length per element) |

**DIM'd string array data:**

`(dim_max + 1)` sequential slots of exactly `max_len` bytes each, from index 0 to
`dim_max`. Each slot contains the string value null-padded to `max_len` bytes. If the
string value exactly fills the slot there is no null terminator. Total record length =
`4 + (dim_max + 1) × max_len`; confirmed by both array records in the dump:
- `AA$(1)*80`: `4 + 2×80 = 164`, len_minus_1 = 163 = `0xa3` ✓
- `BB$(2)*40`: `4 + 3×40 = 124`, len_minus_1 = 123 = `0x7b` ✓

Variable name is **not stored** in the array record. The format is purely positional —
on `INPUT#` the PC-1500 must have the variables and arrays pre-DIM'd with matching
**types** and **dimensions** in the correct order (names do not matter).

If fewer records arrive than expected, the remaining variables are filled with zeroes/empty
strings; if more records arrive, the extra data is ignored. confirmed.

**CE-158 length field:** Always `0x0000` raw (parsed as 1 — meaningless) for VARIABLES saves.
The actual payload length cannot be read from the header. `VariablesConverter` must read
records until EOF; there is no trailing terminator byte in the payload.

**Binary → SDAV (`get`) strategy for `VariablesConverter`:**

For each record:
- Read `0x00` separator; if EOF, stop.
- Read 4-byte prefix.
- If prefix byte 1 = `0x00` (simple variable):
  - `len_minus_1` = LE16(bytes 0–1), `type` = byte 3.
  - Data length = `len_minus_1 + 1 − 4`.
  - If `type == 0x88`: numeric — decode 8-byte BCD value → emit `<decimal>`.
    - If `data[4] == 0xB2`: B2H integer (not observed in dumps, keep for completeness).
    - Otherwise: BCD float; exponent = signed8(`data[0]`), sign = `data[1]`, mantissa = `data[2..6]`.
  - If `type == 0x10`: string — read 16-byte buffer; trim trailing nulls → emit `"<escaped>"`.
- If prefix byte 1 ≠ `0x00` (DIM'd array):
  - `dim_max` = byte 1, type indicator = byte 3.
  - If byte 3 = `0x88` (numeric array): emit `DIM (<dim_max>)`; read `dim_max + 1`
    8-byte BCD values; for each → emit `<decimal>`.
  - Otherwise (string array): max_len = byte 3; emit `DIM $(<dim_max>)*<max_len>`;
    read `dim_max + 1` slots of `max_len` bytes; trim trailing nulls → emit `"<escaped>"`.
- Loop.

**SDAV → binary (`put`) strategy for `VariablesConverter`:**

Parse line by line, skipping blanks and `;` comments:
- `<decimal>` → numeric scalar record: encode BCD, write `00 0B 00 00 88` + 8 bytes.
- `"<string>"` (outside a DIM context) → string scalar record: unescape, encode to 16 bytes,
  write `00 13 00 00 10` + 16 bytes.
- `DIM $(<dim_max>)*<max_len>` → read next `dim_max + 1` non-blank non-comment quoted
  string lines; unescape each; write `00 <len_minus_1> <dim_max> 00 <max_len>` +
  `(dim_max + 1)` slots of `max_len` bytes each (null-padded, no null terminator if full).
- `DIM (<dim_max>)` → read next `dim_max + 1` non-blank non-comment decimal lines;
  encode each as 8-byte BCD; write `00 <len_minus_1> <dim_max> 00 88` +
  `(dim_max + 1)` × 8 bytes.
- Validate final record count against `; Count:` header; reject with a clear error if mismatch.

**Note on B2H integer encoding:** Not observed in any hardware dump. Appears to be an
in-RAM computation format only; unlikely to appear in `PRINT#` output. Keep decode support
for correctness.

**Note on numeric arrays and named non-array variables (AA$, BB$, ...):** Only DIM'd
string arrays have been confirmed from hardware. Numeric arrays (`DIM A(N)`) and named
non-DIM'd string scalars (AA$, BB$, ...) may use different formats — not yet captured.

---

## Implementation Sequence

1. ✅ **Project scaffold**: `pom.xml`, `logging.properties`, stub `SharpDataExchange.java`
2. ✅ **CLI layer**: enums (`PocketPcDevice`, `OutputFormat`, `DataType`), `CliArgs` record, `CliParser`; write `CliParserTest` — 31 tests passing
3. ✅ **Headers + detection**: port `SerialHeader`/`Ce158Header`/`Pc1600Header`; implement `ContentDetector`; write header and detector tests — 27 tests passing
   - PC-1600 `getHeader()` bug fixed: now uses correct 3-byte little-endian encoding (original used 2-byte big-endian)
   - PC-1600 end marker `0x000F` added to `getHeader()` output
   - PC-1600 RESERVE (0x41) and VARIABLES (0x48) type bytes assumed to match PC-1500, pending hardware verification
   4. ✅ **Hardware dumps**: capture raw binary files   from real PC-1500 hardware to confirm the Reserve Area payload start address and to
   fully spec the Variables binary format. Use `SETDEV U1,CI,CO` first, then **SharpCommunicator**
   `--out-file <file> --out-format binary` to write the raw binary including the CE-158 header.

   All dump files go in `src/test/resources/dumps/` and are committed to the repository
   so they serve as both format-confirmation evidence and permanent test fixtures.

   **Reserve Area (`CSAVEr"x"`)** — ✅ done: `src/test/resources/dumps/pc1500-reserve.bin`
   - Payload confirmed: 188 bytes = 3×26-byte labels + 110-byte pool (no checksum)
   - Payload starts at 4008H (ROM status block 4000H–4007H not included) ✓
   - All three layers present; pool structure and token encoding verified
   - File structure: `[27-byte CE-158 header][188-byte payload]` = 215 bytes
   - CE-158 length field = 187 = 188−1 (capacity−1 encoding confirmed from §13)

   **Variables** — ✅ done: four dumps captured and analysed.

   - `src/test/resources/dumps/pc1500-vars-numeric.bin` ✅
     5 numeric variables (floats + zero). B2H integer encoding not observed; all BCD.
   - `src/test/resources/dumps/pc1500-vars-strings.bin` ✅
     4 string variables (short, literal "B$", empty, long). 16-byte fixed buffer confirmed.
   - `src/test/resources/dumps/pc1500-vars-mixed.bin` ✅
     2 numeric + 2 string interleaved. Positional structure and record separator confirmed.
   - `src/test/resources/dumps/pc1500-vars-longstrings-arrays.bin` ✅
     `DIM AA$(1)*80` (1 element, max 80 chars) + `DIM BB$(2)*40` (2 elements, max 40 chars).
     New prefix format confirmed: byte 1 = dim_max, byte 3 = max_len; data = sequential
     max_len-byte slots for each element (index 0 to dim_max); no variable name stored.
   - `src/test/resources/dumps/pc1500-vars-mixed-arrays.bin` ✅
     `DIM A(5)` (6 numeric elements) + `DIM A(0)` (1 numeric element) + `DIM A$(1)*16`
     (2 string elements). Numeric array prefix confirmed: byte 3 = `0x88`, data =
     (dim_max+1) × 8-byte BCD. `DIM A(0)` binary-identical to simple numeric scalar
     confirmed. File has 2 leading `0x00` bytes before the CE-158 header (capture
     artifact); ContentDetector should scan for magic rather than assuming offset 0.

   Format fully specified — see "CE-158 tape format for variables" section above.

   **Machine language** — no dump needed; format is already fully understood.

5. **Conversion layer**: `AsciiBasicTokenizer`, `ReserveAreaConverter`, `VariablesConverter`; write conversion tests including round-trip
6. **Serial layer**: port `ByteProcessor`, `Watchdog`, `SerialPortWrapper`; implement `DataReceiver`, `DataSender`; write `WatchdogTest`
7. **Wire together**: implement `FileHandler` (no clipboard), complete `SharpDataExchange.main()` orchestration; manual integration test on hardware

---

## Critical Reference Files

| File | Purpose |
|---|---|
| `SharpCommunicator/src/.../serialhandler/SerialPortWrapper.java` | Port: auto-detection, baud, flow control |
| `SharpCommunicator/src/.../serialhandler/SerialToDeviceSender.java` | Port: timing-sensitive transmission |
| `SharpCommunicator/src/.../serialhandler/ReadFromPocketPc.java` | Port: watchdog-based receive |
| `SharpCommunicator/src/.../binaryfile/Ce158Header.java` | ✅ Ported to `header/Ce158Header.java` |
| `SharpCommunicator/src/.../binaryfile/Pc1600Header.java` | ✅ Ported to `header/Pc1600Header.java`; encoding fixed; RESERVE/VARIABLES TODO remains open |
| `SharpCommunicator/src/.../SharpCommunicator.java` | Reference for two-step flow |
| `SharpBasicShared/.../antlr/visitor/BinaryEncodingVisitor.java` | Inverse of BinaryBasicDetokenizer |
| `SharpBasicShared/.../core/keyword/KeywordRegistry.java` | Token code lookup for both directions |

---

## Verification

- ✅ **Unit**: `CliParserTest` — all flag combinations, error cases (31 tests)
- ✅ **Unit**: `ContentDetectorTest` — CE-158 headers of each type, ASCII heuristics, SDAR/SDAV (14 tests)
- ✅ **Unit**: `Ce158HeaderTest` / `Pc1600HeaderTest` — round-trip: construct -> serialize -> parse -> compare (14 + 13 tests)
- **Unit**: `WatchdogTest` — timeout fires; `reset()` postpones it
- **Unit**: `BinaryBasicDetokenizerTest` — in SharpBasicShared (see `task-detokenizer.md`); SharpDataExchange relies on library test coverage
- **Unit**: `ReserveAreaConverterTest` — binary->SDAR text; SDAR text->binary; length honored; comments/blanks ignored
- **Unit**: `VariablesConverterTest` — BCD decode; string decode; round-trip
- **Integration**: Manual test with PC-1500 / PC-1600 hardware for `get` (CSAVE + CSAVEa) and `put` (CLOAD)
