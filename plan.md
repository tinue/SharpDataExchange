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
  -f, --format <format>      Output format: ascii (default), asciicompact, binary

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
    │   │   │   ├── OutputFormat.java         # ASCII | ASCIICOMPACT | BINARY
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

```
; SDAV:1.0 pc1500
; Count: 4
; Filename: MYAPP
A=3.14159265
B$="HELLO"
C=0
D$=""
```

- `; SDAV:1.0 <device>` — first line, signals type for detector
- `; Count: <n>` — number of variables (for validation)
- Variables: `NAME=<decimal>` or `NAME$="<string>"`
- Numeric: decoded to decimal (up to 10 significant digits; scientific notation for large/small)
- String: double-quoted; internal quotes escaped as `\"`; control chars as `\xHH`
- Variable name type: trailing `$` = string, else numeric

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
- Longer names (AA$, etc.) and DIM'd string arrays likely use the D0H pointer record.
- In-memory strings are therefore stored either directly at a fixed location (A$–Z$) or via
  a pointer to a separate string buffer (AA$, DIM arrays).

**CE-158 tape format for variables:**

The tape payload is a **sequential list of variable values**, with no variable names and no
memory addresses. Behaviour confirmed by hardware convention:

- `CSAVE"x",V,A,C` saves variables A, B, C as 3 consecutive records.
- `CLOAD"x",V,E` loads them positionally into E, F, G (ignoring the 4th and 5th saved variables).
- `CLOAD"x",V,E,H` loads E←A, F←B, G←C, and zeroes H (file shorter than target range).

The exact binary encoding of each record in the tape file — especially for string variables —
is **still unknown** and requires hardware dumps to determine. Candidates:

- Raw 8-byte D0H record + appended string buffer (raw memory approach)
- Actual string characters inline (length-prefixed or fixed-size)
- Some other normalised form

**Decoding strategy for `VariablesConverter` (pending file format confirmation from dumps):**
- Hardware dump `pc1500-vars-strings.bin` will reveal the string record format.
- For numerics (confirmed from §5-3-1/§5-3-2):
  - If `bytes[4] == 0xB2`: integer; value = signed16(`bytes[5]`, `bytes[6]`)
  - Otherwise: decimal float; exponent = signed8(`bytes[0]`), sign = `bytes[1]`, BCD mantissa = bytes 2–6

---

## Implementation Sequence

1. ✅ **Project scaffold**: `pom.xml`, `logging.properties`, stub `SharpDataExchange.java`
2. ✅ **CLI layer**: enums (`PocketPcDevice`, `OutputFormat`, `DataType`), `CliArgs` record, `CliParser`; write `CliParserTest` — 31 tests passing
3. ✅ **Headers + detection**: port `SerialHeader`/`Ce158Header`/`Pc1600Header`; implement `ContentDetector`; write header and detector tests — 27 tests passing
   - PC-1600 `getHeader()` bug fixed: now uses correct 3-byte little-endian encoding (original used 2-byte big-endian)
   - PC-1600 end marker `0x000F` added to `getHeader()` output
   - PC-1600 RESERVE/VARIABLES type bytes remain unknown; constructor throws `UnsupportedOperationException` with a clear message pending hardware research
4. **Hardware dumps** *(user action — required before step 5)*: capture raw binary files
   from real PC-1500 hardware to confirm the Reserve Area payload start address and to
   fully spec the Variables binary format. Use `SETDEV U1,CI,CO` first, then **SharpCommunicator**
   `--out-file <file> --out-format binary` to write the raw binary including the CE-158 header.

   All dump files go in `src/test/resources/dumps/` and are committed to the repository
   so they serve as both format-confirmation evidence and permanent test fixtures.

   **Reserve Area (`CSAVE"x",A`)** — ✅ done: `src/test/resources/dumps/pc1500-reserve.bin`
   - Payload confirmed: 188 bytes = 3×26-byte labels + 110-byte pool (no checksum)
   - Payload starts at 4008H (ROM status block 4000H–4007H not included) ✓
   - All three layers present; pool structure and token encoding verified
   - File structure: `[27-byte CE-158 header][188-byte payload]` = 215 bytes
   - CE-158 length field = 187 = 188−1 (capacity−1 encoding confirmed from §13)

   **Variables** — three dumps to reveal block structure and all data types:
   Numeric value encoding (BCD decimal and B2H integer) is fully spec'd from §5-3-1/§5-3-2.
   The dumps are needed to determine: the string variable tape encoding (the §5-3-3 D0H
   pointer record is in-RAM layout only — the tape format is unknown), and the CE-158
   file block structure (confirmed sequential/positional, no variable names in the file).

   - `src/test/resources/dumps/pc1500-vars-numeric.bin`
     Store several numeric variables and save them: a positive float, a negative float,
     π (3.14159265), a very small number, zero, and a small integer (to verify whether
     B2H encoding appears in practice or only BCD floats are used).
   - `src/test/resources/dumps/pc1500-vars-strings.bin`
     Store single-letter string variables (e.g. A$, B$, C$) and save them:
     a short string, an empty string, and a string with a double-quote character.
     Single-letter strings (A$–Z$) have fixed memory locations — this dump will show
     their tape encoding directly.
   - `src/test/resources/dumps/pc1500-vars-mixed.bin`
     A mix of numerics and strings (e.g. A=1.5, B$="HI", C=0, D$="") to reveal
     how numeric and string records interleave in the CE-158 payload, and to confirm
     the purely positional (no-name) structure.

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
