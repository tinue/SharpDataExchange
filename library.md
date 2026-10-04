# Embedding `libsharpdx`

`libsharpdx` is the pure core of [SharpDataExchange](README.md) with no file I/O,
packaged for embedding in another application — for example **Calc-U-1600**. It
tokenizes and de-tokenizes Sharp PC-1500 / PC-1600 BASIC, lists, reads, writes
and deletes files on a CE-1600F floppy side, and decodes and encodes cassette-tape
WAV files, entirely in memory.

Three consumers share one core verbatim:

| consumer | artifact | interface |
|---|---|---|
| Rust callers | `libsharpdx.rlib` (crate `sharpdx`) | [`src/lib.rs`](src/lib.rs) |
| C / C++ / Objective-C | `libsharpdx.a` (static) or `libsharpdx.{so,dylib,dll}` (shared) | [`include/sharpdx.h`](include/sharpdx.h) |
| Swift | same as C, plus `include/module.modulemap` | `import SharpDX` |

`include/sharpdx.h` and `include/module.modulemap` are generated from
[`src/ffi/`](src/ffi) by `build.rs` (via cbindgen) and are committed, so a C
or Swift embedder needs **no Rust toolchain** — only the prebuilt library and the
`include/` directory from a [release archive](../../releases).

## What's in a release archive

```
sharpdx-<platform>/
├── bin/
│   └── sde                 (or sde.exe)          the CLI, for convenience
├── lib/
│   ├── libsharpdx.a        (or sharpdx.lib)      static library
│   ├── libsharpdx.so       (.dylib / sharpdx.dll) shared library
│   └── sharpdx.dll.lib                           Windows import library
├── include/
│   ├── sharpdx.h
│   └── module.modulemap
├── LICENSE
├── README.md
└── library.md
```

The macOS archive is Apple Silicon (`arm64`) only and targets macOS 15.8+
(`MACOSX_DEPLOYMENT_TARGET=15.8`, set in `.cargo/config.toml`); its `sde` and
`libsharpdx.dylib` are signed with a Developer ID and notarized (hardened
runtime). A signed, notarized, stapled `.pkg` installer is published alongside it
— see the main [README](README.md#on-macos).

The libraries in `lib/` are built without the serial transport (`sde get`/`put`),
which the C ABI never exposes, so they carry no `serialport` dependency. To build
the same lib from source: `cargo build --release --lib --no-default-features`.

## C ABI contract

Every entry point:

* returns `int32_t` — `SDE_OK` (`0`) on success, a negative `SDE_ERR*` on failure;
* **never panics across the boundary** (`catch_unwind` inside; a caught panic
  becomes `SDE_ERR_PANIC`);
* on failure, leaves a human-readable message retrievable with `sde_last_error()`
  — a thread-local, NUL-terminated string valid until the next `sde_*` call **on
  the same thread**. It is never `NULL` (empty string when there is no error).

Functions that produce data write a **Rust-allocated** buffer through an
`(uint8_t **out, size_t *out_len)` pair. The caller owns it and must release it
with `sde_buf_free(out, out_len)` exactly once, passing back the same pointer and
length. Do not `free()` it, do not realloc it, do not free it twice.

Input buffers are borrowed for the duration of the call only; the library keeps
no reference to them.

`name` (the CE-158 header filename, max 16 chars, upper-cased by the encoder) is
`NULL` or a C string. Pass `NULL` when de-tokenizing.

### Error codes

| macro | value | meaning |
|---|---|---|
| `SDE_OK` | `0` | success |
| `SDE_ERR` | `-1` | conversion failed — see `sde_last_error()` |
| `SDE_ERR_PANIC` | `-2` | a panic was caught at the boundary (please report) |
| `SDE_ERR_ARGS` | `-3` | a required pointer argument was `NULL` / invalid |
| `SDE_ERR_NOT_FOUND` | `-4` | disk: no such file |
| `SDE_ERR_EXISTS` | `-5` | disk: the file exists (pass `SDE_DISK_FORCE` to replace it) |
| `SDE_ERR_DISK_FULL` | `-6` | disk: not enough free space |
| `SDE_ERR_NOT_FORMATTED` | `-7` | disk: the side has no CE-1600F filesystem |
| `SDE_ERR_PROTECTED` | `-8` | disk: the file is write-protected |
| `SDE_ERR_BAD_NAME` | `-9` | disk: not a valid 8.3 name |
| `SDE_ERR_CORRUPT` | `-10` | disk: broken FAT chain or directory |
| `SDE_ERR_DIRECTORY_FULL` | `-11` | disk: all 48 directory entries in use |
| `SDE_ERR_WAV_FORMAT` | `-12` | WAV: not a RIFF/WAVE file, or an encoding that can't be read |
| `SDE_ERR_WAV_NO_SIGNAL` | `-13` | WAV: no PC-1500 / PC-1600 tape signal on it |
| `SDE_ERR_WAV_CORRUPT` | `-14` | WAV: a file was found but failed its checksums |
| `SDE_ERR_WAV_UNSUPPORTED` | `-15` | WAV: content that can't be taken off / put on a tape |

### Entry points

```c
const char *sde_version(void);          /* static, NUL-terminated "x.y.z" */
const char *sde_last_error(void);       /* thread-local; never NULL       */
void        sde_buf_free(uint8_t *ptr, size_t len);

int32_t sde_detect(const uint8_t *in, size_t in_len, SdeContent *out_kind);
int32_t sde_file_kind(const uint8_t *in, size_t in_len, const char **out_kind);
int32_t sde_file_info(const uint8_t *in, size_t in_len, SdeFileInfo *out);

int32_t sde_tokenize(SdeDevice device, int with_header,
                     SdeSegmentMarker segment_marker, const char *name,
                     const uint8_t *in, size_t in_len,
                     uint8_t **out, size_t *out_len);

int32_t sde_detokenize(SdeDevice device,
                       const uint8_t *in, size_t in_len,
                       SdeLineEnding line_ending,
                       uint8_t **out, size_t *out_len);

int32_t sde_convert(SdeDevice device, const char *name,
                    const uint8_t *in, size_t in_len,
                    SdeLineEnding line_ending,
                    uint8_t **out, size_t *out_len, SdeContent *out_kind);
```

* **`sde_tokenize`** — ASCII BASIC → tokenized payload. `with_header != 0`
  prepends the serial header (CE-158 for `SDE_DEVICE_PC1500`, PC-1600 header for
  `SDE_DEVICE_PC1600`). Fails if the input is not ASCII BASIC. `segment_marker`
  picks how a `#SEGMENT` line is stored: `SDE_SEGMENT_MARKER_WIRE` (`FF 00 00`, what
  `SAVE "COM1:"` sends) or `SDE_SEGMENT_MARKER_MEMORY` (the bare `FF` the ROM keeps in
  its program area — for poking a program straight into RAM).
* **`sde_detokenize`** — tokenized bytes → ASCII BASIC (UTF-8). A CE-158 /
  PC-1600 header is detected automatically and the device is then taken *from the
  header*; a bare headerless payload is decoded with the `device` argument.
* **`sde_convert`** — content-driven, mirrors the CLI: it inspects the input and
  picks the direction. `out_kind` (may be `NULL`) receives the detected input
  kind. This is the entry point most embedders want.
* **`sde_detect`** — classify a buffer without converting it (BASIC and text only;
  for everything else use `sde_file_kind`).
* **`sde_file_kind`** / **`sde_file_info`** — what a file holds, as a one-word token,
  plus where its payload is and where it goes. See
  [File kind (program loaders)](#file-kind-program-loaders).

`SdeDevice` is `SDE_DEVICE_PC1500` (0) or `SDE_DEVICE_PC1600` (1). `SdeContent` is
`UNKNOWN` / `ASCII_BASIC` / `CE158_BASIC` / `PC1600_BASIC` / `TEXT` (plain text that is
not a BASIC listing; before 0.2.3 such input was reported as `UNKNOWN`).

### File kind (program loaders)

`sde_file_kind` writes a static, NUL-terminated one-word token to `*out_kind` (do
not free it). The tokens are part of the stable API and are never renamed:

| token | the buffer holds |
|---|---|
| `basic-ascii` | an ASCII BASIC listing |
| `basic-pc1500` | tokenized BASIC behind a CE-158 header |
| `basic-pc1600` | tokenized BASIC behind a PC-1600 header |
| `ml-lh5801` | machine code behind a CE-158 header |
| `ml-z80` | machine code behind a PC-1600 header |
| `raw-lh5801` / `raw-z80` | headerless binary whose code looks like that CPU's (a **heuristic guess**) |
| `raw` | headerless binary, CPU not recognized |
| `reserve` / `reserve-text` | Reserve Area behind a CE-158 header / as SDAR text |
| `variables` / `variables-text` | Variables behind a CE-158 header / as SDAV text |
| `text` | plain text |
| `empty` | nothing (`in_len == 0`; `in` may then be `NULL`) |
| `wav-pc1500` / `wav-pc1600` | a cassette WAV with PC-1500 (CE-150) / PC-1600 (CE-1600P) files |
| `wav` | a WAV file with no decodable PC-1500 / PC-1600 tape on it |
| `damaged` | a file with a fatal problem (below) |

Headers don't record the CPU: a CE-158 header is taken as LH5801 code (PC-1500
family), a PC-1600 header as Z80 code (the PC-1600's `BSAVE` runs on its Z80). For a
headerless binary the CPU is guessed from the code (see `sde info` in the README).

`sde_file_info` fills an `SdeFileInfo`. Nothing in it needs freeing:

| field | meaning |
|---|---|
| `kind` | the token, as above, but never `damaged`: check `problems` |
| `problems` | `SDE_PROBLEM_*` bits |
| `payload_offset` | first payload byte in `in`: after the header, `0` if headerless |
| `payload_len` | payload bytes present in `in` (never past its end, even when truncated) |
| `load_addr`, `run_addr` | `ml-*` only: load and run address (PC-1600: bank in bits 16–23); else `0` |
| `autorun` | `ml-*` only: `1` if `run_addr` really starts the program (its low 16 bits are not `FFFF`) |
| `name` | CE-158 header filename or the SDAR/SDAV name; UTF-8, NUL-terminated, `""` if none |

| `SDE_PROBLEM_*` | value | meaning |
|---|---|---|
| `TRUNCATED` | `1` | the payload is shorter than the header says |
| `TRAILING` | `2` | bytes follow the payload |
| `LEADING_NOISE` | `4` | `00` bytes precede the header |
| `HEADER_CUT` | `8` | header magic, but no complete header with a known type |
| `BAD_PAYLOAD` | `16` | tokenized BASIC that doesn't de-tokenize, Reserve/Variables that don't decode |
| `FATAL` | `25` | `TRUNCATED \| HEADER_CUT \| BAD_PAYLOAD`: don't load the file |

A loader:

```c
SdeFileInfo fi;
if (sde_file_info(buf, len, &fi) != SDE_OK || (fi.problems & SDE_PROBLEM_FATAL)) {
    return reject("unusable file");
}
if (strcmp(fi.kind, "ml-lh5801") == 0) {
    memcpy(&pc1500_ram[fi.load_addr], buf + fi.payload_offset, fi.payload_len);
    if (fi.autorun) start_at(fi.run_addr);
} else if (strcmp(fi.kind, "basic-pc1500") == 0) {
    load_basic(buf + fi.payload_offset, fi.payload_len);
} else {
    return reject(fi.kind);
}
```

### Line endings

Tokenizing **always** accepts `CR` (`\r`) or `CRLF` (`\r\n`) line endings in the
input listing, on any platform — they are normalized before the scanner runs.

When de-tokenizing, `line_ending` chooses the terminator written into the
listing:

| `SdeLineEnding` | value | terminator |
|---|---|---|
| `SDE_LINE_ENDING_PLATFORM` | `0` | host default — `\r\n` on Windows, `\n` elsewhere |
| `SDE_LINE_ENDING_LF` | `1` | `\n` |
| `SDE_LINE_ENDING_CR_LF` | `2` | `\r\n` |
| `SDE_LINE_ENDING_CR` | `3` | `\r` (the PC-1500's own line terminator) |

Pass `SDE_LINE_ENDING_PLATFORM` for the default behaviour. It is ignored when the
input is ASCII BASIC (i.e. when `sde_convert` tokenizes).

### C / C++ example

```c
#include "sharpdx.h"

uint8_t *out = NULL; size_t out_len = 0;
int32_t rc = sde_tokenize(SDE_DEVICE_PC1500, /*with_header=*/1, SDE_SEGMENT_MARKER_WIRE,
                          "SAMPLE", bas, bas_len, &out, &out_len);
if (rc != SDE_OK) {
    fprintf(stderr, "tokenize: %s\n", sde_last_error());
    return 1;
}
/* ... use out[0 .. out_len] ... */
sde_buf_free(out, out_len);
```

Build:

```
cc app.c -Iinclude path/to/libsharpdx.a -o app                 # static
cc app.c -Iinclude -Lpath/to/lib -lsharpdx -o app              # shared
```

On macOS a static link also needs the platform libs the Rust std pulls in; the
linker resolves them from the SDK automatically with `cc`. For a shared link add
`-Wl,-rpath,path/to/lib` (or install the dylib on the loader path). A full
runnable version is [`examples/embed_c.c`](examples/embed_c.c).

## Swift

`include/module.modulemap` exposes the header as `module SharpDX`.

```swift
import SharpDX

var out: UnsafeMutablePointer<UInt8>? = nil
var outLen = 0
let rc = "SAMPLE".withCString { name in
    sde_tokenize(SDE_DEVICE_PC1500, 1, SDE_SEGMENT_MARKER_WIRE, name, ptr, len, &out, &outLen)
}
guard rc == SDE_OK, let out else {
    throw SharpDXError(String(cString: sde_last_error()))
}
defer { sde_buf_free(out, outLen) }
let data = Data(bytes: out, count: outLen)
```

```
swiftc app.swift -I include -L path/to/lib -lsharpdx -o app
```

For an Xcode / SwiftPM target: add `include/` as a header search path, link
`libsharpdx.a`, and either vendor the modulemap or wrap it in a system-library
module target. See [`examples/embed_swift.swift`](examples/embed_swift.swift).

## Rust

```toml
[dependencies]
sharpdx = { git = "https://github.com/tinue/SharpDataExchange", default-features = false }
```

`default-features = false` drops the `cli` feature (the `sde` binary's `clap`) and
the `serial` feature (the `get`/`put` transport modules and `serialport`); enable
`features = ["serial"]` if you want those modules.

The pure core is safe Rust — no need to go through the C ABI:

```rust
use sharpdx::{convert, Content, Device};

let outcome = convert(&input, Device::Pc1500, Some("SAMPLE"), /*with_header=*/ true)?;
match outcome.content {
    Content::AsciiBasic  => { /* `input` was a listing -> outcome.bytes is CE-158 tokenized */ }
    Content::Ce158Basic  => { /* `input` was tokenized -> outcome.bytes is a listing */ }
    _ => {}
}
```

`convert` writes a de-tokenized listing with the host-default line ending (`\r\n`
on Windows, `\n` elsewhere); CR / CRLF input to a tokenize is always accepted.
Use `convert_with(&input, device, name, with_header, LineEnding::CrLf,
SegmentMarker::Wire)` (or `::Lf` / `::Cr` / `::Platform`) to force a specific
terminator; the last argument is the `#SEGMENT` form, as for `sde_tokenize`.

The disk modules are plain Rust too: `sharpdx::floppy_image` reads and writes
`.floppy.yaml` files, `sharpdx::diskfs::Volume` works on one side, and
`sharpdx::transfer` holds the conversions (`build_put` with `Endpoint::Disk`,
`extract`, `classify_disk_file`).

`sharpdx::VERSION` is the crate version string. `sharpdx::ffi::*` is the same C
ABI if you need it from Rust (the test suite exercises it that way).

## Disk sides (CE-1600F floppy)

```c
int32_t sde_disk_list  (const uint8_t *side, size_t side_len,
                        SdeDirEntry *entries, size_t capacity,
                        size_t *out_count, uint32_t *out_free_bytes);
int32_t sde_disk_get   (const uint8_t *side, size_t side_len, const char *name,
                        SdeDiskGetMode mode, SdeLineEnding line_ending,
                        uint8_t **out, size_t *out_len, SdeDiskKind *out_kind);
int32_t sde_disk_put   (uint8_t *side, size_t side_len, const char *name,
                        const uint8_t *in, size_t in_len, SdeDiskPutMode mode,
                        uint32_t start_addr, uint32_t run_addr, uint32_t flags,
                        const SdeDiskTime *when);
int32_t sde_disk_delete(uint8_t *side, size_t side_len, const char *name_or_pattern,
                        uint32_t flags, size_t *out_deleted);
```

The caller owns the container — for Calc-U-1600, the in-memory disk image — and passes
**one side** of it: `SDE_DISK_SIDE_SIZE` (65536) bytes, side A at offset 0 and side B at
offset 65536 of the 128 KB image. The library never sees the `.floppy.yaml` file.
`sde_disk_put` and `sde_disk_delete` change the buffer in place, and only on success.

* **`sde_disk_list`** fills up to `capacity` `SdeDirEntry` records (name `"NAME.EXT"`,
  attribute, month/day/hour/minute/second, size, `SdeDiskKind`, and for machine code
  the 24-bit load/run address). `SDE_DISK_MAX_ENTRIES` (48) always suffices;
  `*out_count` is the number of files.
* **`sde_disk_get`** — `SDE_DISK_GET_MODE_AUTO` gives the natural host form: BASIC as
  a UTF-8 listing, ASCII files as UTF-8 text with `line_ending`, machine code and
  unknown data unchanged. `BINARY` keeps BASIC tokenized with its header, `PAYLOAD`
  drops the header, `RAW` is the stored bytes.
* **`sde_disk_put`** — `SDE_DISK_PUT_MODE_AUTO` tokenizes a BASIC listing (PC-1600
  keywords), stores text as a PC-1600 ASCII file (CP437, CRLF, `1A`), and a file that
  already has a PC-1600 header unchanged; anything else fails. `ASCII_LISTING` stores a
  listing as an ASCII program, `MACHINE` wraps headerless code in a header with
  `start_addr` / `run_addr` (`SDE_DISK_NO_RUN` = no auto-start), `RAW` stores the bytes
  as given, `TOKENIZE` tokenizes the input as a BASIC listing even if detection took
  it for plain text (fails if it has no numbered lines). `flags` = `SDE_DISK_FORCE` replaces an existing file. `when` sets the
  directory time stamp (the PC-1600 clock has no year); `NULL` uses the current UTC time.
* **`sde_disk_delete`** takes a name or a `*` / `?` pattern; `*out_deleted` is the
  number of files removed.

These are exactly the rules the `sde` CLI applies (see the README's disk-image table).

## Cassette WAV files

```c
int32_t sde_wav_count (const uint8_t *in, size_t in_len,
                       size_t *out_count, size_t *out_issues);
int32_t sde_wav_decode(const uint8_t *in, size_t in_len, size_t index,
                       uint8_t **out, size_t *out_len, SdeWavFile *out_file);
int32_t sde_wav_encode(const uint8_t *in, size_t in_len, uint32_t sample_rate,
                       uint32_t leader_ms, const char *name,
                       uint8_t **out, size_t *out_len);
```

A tape file crosses the boundary as its **serial image** — CE-158 or PC-1600 header
plus payload — the same bytes `sde_file_info`, `sde_detokenize` and a program loader
already take. So a loader that handles `.bin` / `.bbin` files handles a WAV with one
extra call: `sde_file_kind` reports `wav-pc1500` / `wav-pc1600` (with `sde_file_info`
describing the first file on the tape), and `sde_wav_decode` turns the file into an
image.

* **`sde_wav_count`** — how many files on the tape decode safely; `*out_issues` (may be
  `NULL`) how many were found but are damaged or unsupported.
  `SDE_ERR_WAV_NO_SIGNAL` if there is no tape on it at all.
* **`sde_wav_decode`** — file `index` (0-based, in tape order) as a serial image
  (free with `sde_buf_free`); `SdeWavFile` (may be `NULL`) gives its tape format
  (`SDE_TAPE_FORMAT_PC1500_CE150` / `…_PC1600_CE1600P`), kind, name, load / run address,
  payload size, position on the tape and measured speed. A tape whose only file is
  damaged fails with `SDE_ERR_WAV_CORRUPT` and says where.
* **`sde_wav_encode`** — a serial image (BASIC, machine code, or PC-1500 Reserve Area)
  as a 16-bit mono WAV at `sample_rate` (at least 16000). The tape format follows the header (CE-158 →
  CE-150, PC-1600 → CE-1600P); `name` (may be `NULL`) overrides the name on the tape.
  `leader_ms` is the lead-in tone: `SDE_WAV_LEADER_DEFAULT` (about 2 s / 3 s),
  `SDE_WAV_LEADER_ROM` (the ROMs' own, about 8 s / 3.3 s) or a length in milliseconds.

Decoding accepts any PCM (8–32 bit) or float WAV from 5 kHz up, mono or stereo, and
tolerates speed error (±25 %), wow and flutter, low level and noise; it returns a file
only when all of its checksums match. Each call decodes the whole WAV (a few
milliseconds per minute of audio), so decode once and keep the image.

In Rust the same is `sharpdx::wav` (`decode`, `encode`, `to_image`, `from_image`).

## Threading

The core holds no global mutable state; independent conversions on separate
threads do not interfere. The only per-thread state is the `sde_last_error()`
buffer, which is thread-local by design — read it on the same thread that made
the failing call.

## Versioning and ABI stability

This project may contain breaking changes in every release, major or minor, until
version 1.0.0 is reached — that includes the C ABI (`sde_*` symbol set, `Sde*`
enum values) and the Rust API. `sde_version()` reports the build you linked; pin
an exact release. Once the project reaches `1.0.0` it follows semver and the
`sde_*` ABI becomes stable.

## License

[Polyform Noncommercial License 1.0.0](LICENSE). Embedding in a noncommercial
application (Calc-U-1600 included) is a permitted purpose. Ship a copy of the
`LICENSE` file with anything you distribute that includes this library.
