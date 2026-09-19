# Embedding `libsharpdx`

`libsharpdx` is the pure `convert` core of [SharpDataExchange](README.md) with no
file I/O, packaged for embedding in another application — for example
**Calc-U-1600**. It tokenizes and de-tokenizes Sharp PC-1500 / PC-1600 BASIC
entirely in memory.

Three consumers share one core verbatim:

| consumer | artifact | interface |
|---|---|---|
| Rust callers | `libsharpdx.rlib` (crate `sharpdx`) | [`src/lib.rs`](src/lib.rs) |
| C / C++ / Objective-C | `libsharpdx.a` (static) or `libsharpdx.{so,dylib,dll}` (shared) | [`include/sharpdx.h`](include/sharpdx.h) |
| Swift | same as C, plus `include/module.modulemap` | `import SharpDX` |

`include/sharpdx.h` and `include/module.modulemap` are generated from
[`src/ffi.rs`](src/ffi.rs) by `build.rs` (via cbindgen) and are committed, so a C
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

The macOS archive is Apple Silicon (`arm64`) only; its `sde` and
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

### Entry points

```c
const char *sde_version(void);          /* static, NUL-terminated "x.y.z" */
const char *sde_last_error(void);       /* thread-local; never NULL       */
void        sde_buf_free(uint8_t *ptr, size_t len);

int32_t sde_detect(const uint8_t *in, size_t in_len, SdeContent *out_kind);

int32_t sde_tokenize(SdeDevice device, int with_header, const char *name,
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
  `SDE_DEVICE_PC1600`). Fails if the input is not ASCII BASIC.
* **`sde_detokenize`** — tokenized bytes → ASCII BASIC (UTF-8). A CE-158 /
  PC-1600 header is detected automatically and the device is then taken *from the
  header*; a bare headerless payload is decoded with the `device` argument.
* **`sde_convert`** — content-driven, mirrors the CLI: it inspects the input and
  picks the direction. `out_kind` (may be `NULL`) receives the detected input
  kind. This is the entry point most embedders want.
* **`sde_detect`** — classify a buffer without converting it.

`SdeDevice` is `SDE_DEVICE_PC1500` (0) or `SDE_DEVICE_PC1600` (1). `SdeContent` is
`UNKNOWN` / `ASCII_BASIC` / `CE158_BASIC` / `PC1600_BASIC`.

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
int32_t rc = sde_tokenize(SDE_DEVICE_PC1500, /*with_header=*/1, "SAMPLE",
                          bas, bas_len, &out, &out_len);
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
    sde_tokenize(SDE_DEVICE_PC1500, 1, name, ptr, len, &out, &outLen)
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
    Content::AsciiBasic  => { /* `input` was tokenized -> outcome.bytes is a listing */ }
    Content::Ce158Basic  => { /* `input` was a listing -> outcome.bytes is CE-158 tokenized */ }
    _ => {}
}
```

`convert` writes a de-tokenized listing with the host-default line ending (`\r\n`
on Windows, `\n` elsewhere); CR / CRLF input to a tokenize is always accepted.
Use `convert_with(&input, device, name, with_header, LineEnding::CrLf)` (or
`::Lf` / `::Cr` / `::Platform`) to force a specific terminator.

`sharpdx::VERSION` is the crate version string. `sharpdx::ffi::*` is the same C
ABI if you need it from Rust (the test suite exercises it that way).

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
