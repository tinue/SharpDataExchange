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

### ASCII format: Reserve Area (SDAR)

```
; SDAR:1.0 pc1500
; Length: 48
; Filename: MYAPP
3A 00 FF 1A 00 00 00 00  00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00
```

- `; SDAR:1.0 <device>` — first line, signals type for detector
- `; Length: <n>` — byte count of actual data
- `; Filename: <name>` — from CE-158 header (16 chars max)
- Data: 16 hex bytes per line, space-separated, 2-space gap after byte 8
- Comment (`;`) and blank lines ignored by parser

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
- Numeric: BCD-decoded to decimal (up to 12 significant digits; scientific notation for large/small)
- String: double-quoted; internal quotes escaped as `\"`; control chars as `\xHH`
- Variable name type: trailing `$` = string, else numeric

BCD decoding: 8-byte PC-1500 packed BCD; sign and exponent nibbles per Sharp PC-1500 Technical Reference Manual variable storage format.

---

## Implementation Sequence

1. **Project scaffold**: `pom.xml`, `logging.properties`, stub `SharpDataExchange.java`
2. **CLI layer**: enums (`PocketPcDevice`, `OutputFormat`, `DataType`), `CliArgs` record, `CliParser`; write `CliParserTest`
3. **Headers + detection**: port `SerialHeader`/`Ce158Header`/`Pc1600Header` (resolve PC-1600 Reserve/Variables TODO); implement `ContentDetector`; write header and detector tests
4. **Conversion layer**: `AsciiBasicTokenizer`, `ReserveAreaConverter`, `VariablesConverter`; write conversion tests including round-trip
5. **Serial layer**: port `ByteProcessor`, `Watchdog`, `SerialPortWrapper`; implement `DataReceiver`, `DataSender`; write `WatchdogTest`
6. **Wire together**: implement `FileHandler` (no clipboard), complete `SharpDataExchange.main()` orchestration; manual integration test on hardware

---

## Critical Reference Files

| File | Purpose |
|---|---|
| `SharpCommunicator/src/.../serialhandler/SerialPortWrapper.java` | Port: auto-detection, baud, flow control |
| `SharpCommunicator/src/.../serialhandler/SerialToDeviceSender.java` | Port: timing-sensitive transmission |
| `SharpCommunicator/src/.../serialhandler/ReadFromPocketPc.java` | Port: watchdog-based receive |
| `SharpCommunicator/src/.../binaryfile/Ce158Header.java` | Port: type-char switch for detection |
| `SharpCommunicator/src/.../binaryfile/Pc1600Header.java` | Port: resolve Reserve/Variables type TODO |
| `SharpCommunicator/src/.../SharpCommunicator.java` | Reference for two-step flow |
| `SharpBasicShared/.../antlr/visitor/BinaryEncodingVisitor.java` | Inverse of BinaryBasicDetokenizer |
| `SharpBasicShared/.../core/keyword/KeywordRegistry.java` | Token code lookup for both directions |

---

## Verification

- **Unit**: `CliParserTest` — all flag combinations, error cases
- **Unit**: `ContentDetectorTest` — CE-158 headers of each type, ASCII heuristics, SDAR/SDAV
- **Unit**: `Ce158HeaderTest` / `Pc1600HeaderTest` — round-trip: construct -> serialize -> parse -> compare
- **Unit**: `WatchdogTest` — timeout fires; `reset()` postpones it
- **Unit**: `BinaryBasicDetokenizerTest` — in SharpBasicShared (see `task-detokenizer.md`); SharpDataExchange relies on library test coverage
- **Unit**: `ReserveAreaConverterTest` — binary->SDAR text; SDAR text->binary; length honored; comments/blanks ignored
- **Unit**: `VariablesConverterTest` — BCD decode; string decode; round-trip
- **Integration**: Manual test with PC-1500 / PC-1600 hardware for `get` (CSAVE + CSAVEa) and `put` (CLOAD)
