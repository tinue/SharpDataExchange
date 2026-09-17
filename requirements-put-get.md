# Requirements: `put` and `get` (BASIC + machine language only)

Extracted from the Java sister project
(`/Users/me/Development/sharp/SharpDataExchange`) as the basis for adding `put`
and `get` to this Rust port. Scope is **BASIC programs and machine-language
(assembly) programs only** — Reserve Area and Variables transfer are explicitly
out of scope for this pass (see "Out of scope" below).

Tokenizing/de-tokenizing already exists in this project (`convert`) and is
**not** being re-specified here; it's fine for it to differ from the Java
implementation's internals as long as the wire format it produces/consumes
matches what's described below.

## 1. Concepts

- Two verbs, from the PC's point of view:
  - **`get`** — PC receives data from the Pocket Computer over serial and
    writes it to a file. Corresponds to a `CSAVE`/`SAVE "COM1:"` on the device.
  - **`put`** — PC reads a file and sends it to the Pocket Computer.
    Corresponds to a `CLOAD`/`LOAD "COM1:"` on the device.
- Ordering matters: for `get`, start the PC command first, then trigger the
  save on the device. For `put`, issue the load command on the device first,
  then start the PC command. The device does not buffer outgoing data.
- Content/type is detected from the byte stream itself, never from file name
  or extension.

## 2. Devices

Four device targets, each selecting serial parameters and header flavor:

| Device | Baud | Flow control | Header | Send pacing |
|---|---|---|---|---|
| `pc1500` (default) | 19200 | none | CE-158 (27 bytes) | paced (1 ms/byte + fixed pauses) |
| `pc1500a` | 19200 | none | CE-158 (27 bytes) | paced |
| `pc1600` | 9600 | RTS/CTS hardware | PC-1600 (16 bytes) | none (hardware handshake throttles) |
| `pc1600emul` | 9600 | none (pseudo-terminal has no handshake lines) | PC-1600 (16 bytes) | paced, same scheme as PC-1500 |

Serial port config in all cases: 8 data bits, no parity, 1 stop bit, no
exclusive lock (needed so a pseudo-terminal peer can already hold it open).

Port selection: explicit `--port`, or auto-detect. Auto-detect looks for
`cu.usb*` (macOS) / `ttyACM*`, `ttyUSB*` (Linux); succeeds only if exactly one
candidate matches. `pc1600emul` ports (pseudo-terminals) are never enumerated
this way and must be given explicitly, either via `--port` or via the config
file's default (see §2a) — the Java tool hardcodes a fixed emulator socket
path as its default; the Rust port replaces that hardcoded default with the
configurable one below.

### 2a. Config file for defaults

A per-user config file holds default values, read the way `git` reads its
global config: optional (nothing breaks if it's absent), lives in a
conventional per-user location (a dotfile, e.g. under the OS's config-home /
`$HOME`), and is read/written through subcommands rather than hand-editing
being required — analogous to `git config --global <key> <value>` — except
only the equivalent of "global" scope is needed here (no per-repo/local
config, since this tool has no repo concept).

- Two defaults needed for this pass:
  1. The serial port to use for `--device pc1600emul` when `--port` is not
     given on the command line.
  2. The default verbosity (§7a) — whether narration is on or off when
     neither `-v`/`--verbose` nor `-q`/`--quiet` is given on the command
     line. Absent this config setting (or absent the whole config file),
     verbose narration defaults to off, same as today.
- Needs a way to set each (`sde config set <key> <value>` or similar), read
  it back (`sde config get <key>`), and have `get`/`put` consult both
  silently: the port default when `--port` is omitted and `--device
  pc1600emul` is in effect, the verbosity default whenever neither `-v` nor
  `-q` was passed.
- Precedence for both: an explicit command-line flag (`--port`, or `-v`/`-q`)
  always wins over its config file default.
- Exact key naming/subcommand shape is open — but the mechanism (optional
  file, global-only, get/set via the CLI) should follow the git-config model
  described above.

## 3. Serial headers

Two header formats. The parser must recognize both by their magic bytes,
independent of which `--device` was passed, and use the header's own device
info once found (for `get`, the header is more reliable than a possibly-wrong
`--device`; for `put`, a header already present overrides `--device`).

### CE-158 header (PC-1500 / PC-1500A) — 27 bytes

```
0x00  Magic: 0x01
0x01  Type char: '@'=BASIC, 'B'=MACHINE   (also 'A'=RESERVE, 'H'=VARIABLES — out of scope here)
0x02  'C'
0x03  'O'
0x04  'M'
0x05  Filename (16 bytes, CP437, space-padded)
0x15  Load start address (2 bytes, big-endian) — MACHINE only, else 0
0x17  Data length (2 bytes, big-endian) — stores (length - 1); i.e. capacity-1
0x19  Auto-run address (2 bytes, big-endian) — MACHINE only, else 0
```

- Magic check: byte 0 == 0x01, bytes 2-4 == "COM".
- Filename is CP437, trimmed of padding on read; truncated to 16 chars and
  space-padded to 16 bytes on write.
- Length field is stored as `length - 1` on the wire; add 1 back when reading.

### PC-1600 header — 16 bytes

```
0x00  Magic: 0xFF 0x10 0x00 0x00
0x04  Type byte: 0x21=BASIC, 0x10=MACHINE   (0x?? for RESERVE/VARIABLES don't exist — unsupported on this device at all)
0x05  Data length (3 bytes, little-endian)
0x08  Load start address (3 bytes, little-endian) — MACHINE only, else 0
0x0B  Auto-run address (3 bytes, little-endian) — MACHINE only, else 0
0x0E  End marker: 0x00 0x0F
```

- Magic check: byte 0 == 0xFF, bytes 1-3 == 0x10 0x00 0x00.
- Whether the PC-1600 protocol has any header type for Reserve Area or
  Variables at all is **not established** — the Java tool simply doesn't
  implement Reserve/Variables transfer for the PC-1600, but that could be a
  scope decision rather than a hardware/protocol limitation. Treat this as
  unresearched, not as a confirmed protocol fact, until checked against the
  PC-1600 documentation/ROM.

### Header scanning / expected-length computation

Given a growing byte buffer (as bytes arrive over serial), a function must:
- scan for either magic sequence starting at any offset (to tolerate leading
  junk bytes, e.g. stray 0x00 from the capture path),
- return "not enough bytes yet" until the full fixed-size header is present,
- once the header is parsed, compute total expected bytes = header offset +
  header size + payload length, so the receiver knows exactly when the
  transfer is complete (no timeout needed) — this only applies to BASIC and
  MACHINE headers, both of which have a meaningful length field.

### Header presence / auto-add on `put`

- If the input file already carries a recognized header, send it as-is
  (payload not re-wrapped).
- For machine-language input with no header: if `--start-address` is given,
  synthesize a header (type MACHINE, given start address, given or default
  `0xFFFF` run address, payload length, filename derived from the CLI input
  file's base name). If no `--start-address` is given, this is an error: abort
  with a message explaining that a headerless machine-language file needs
  `--start-address` (unless `--raw` is given — see below).
- `--raw` on `put` explicitly overrides the above: it sends the file's bytes
  exactly as read, unmodified, no header synthesized, even without
  `--start-address` — for a payload that intentionally carries no header at
  all (e.g. matching `get --raw`/hand-written-sender workflows).
- Filename for a synthesized header: input file's base name (no extension),
  uppercased, truncated to 16 characters.
- When a header is already present, `--device` may be omitted — infer the
  target device from the header (CE-158 → pc1500 family; PC-1600 header →
  pc1600). Exception: if the header says PC-1600 but `--device pc1600emul`
  was explicitly given, keep `pc1600emul` (the header can't distinguish real
  hardware from the emulator link) — this determines transport pacing/flow
  control, not the header bytes themselves.

## 4. `get` — receive from Pocket Computer

Command shape: `get [options] [<output-file>]`.

Steps:
1. Open the serial port per the resolved `--device` (baud + flow control per
   §2).
2. Accumulate incoming bytes. Determine end-of-transfer:
   - If a recognized header appears and its length field is meaningful (BASIC
     or MACHINE — not applicable here since Reserve/Variables are out of
     scope), stop as soon as `header_offset + header_size + length` bytes
     have arrived.
   - Otherwise (no header recognized yet, or header not fully received), fall
     back to an idle-timeout watchdog: no new bytes for **5000 ms** (PC-1500 /
     PC-1500A) or **500 ms** (PC-1600 / PC-1600 emulator) → transfer is done.
     The watchdog resets on every new byte and only starts once at least one
     byte has arrived.
3. Detect content type from the accumulated bytes (CE-158 header → BASIC or
   MACHINE by type char; PC-1600 header → BASIC or MACHINE by type byte; no
   header + text-like bytes with line-number-prefixed lines → ASCII BASIC;
   otherwise unknown/binary).
4. Decide output:
   - **Format `ascii` (default):**
     - Tokenized BASIC (has header) → de-tokenize to a readable listing, full
       keyword names, written as text.
     - ASCII BASIC (sent via `CSAVEa`/`SAVE...,A`, no header) → clean up line
       endings and write as text (already ASCII/CP437 keywords).
     - Machine language can't be de-tokenized to ASCII at all. If the
       detected/effective type is MACHINE and the requested `--format` is
       `ascii` (whether that's the explicit flag or just the default),
       that's an impossible request — error out and abort rather than
       silently falling back to binary. `--format binary` is the only
       accepted format for machine language.
   - **Format `binary`:** write the raw bytes as received, header included by
     default, so the file can be sent straight back with `put` unchanged.
   - `--skip-header` (binary format only): omit the header bytes from the
     saved file. Print a warning that the resulting file cannot be
     auto-identified or reloaded without it.
5. Output filename resolution:
   - If given on the command line: use it; if it has no extension, append one
     based on detected type (`.bas` for BASIC, `.bin` for MACHINE).
   - If omitted: derive from the header's filename field (if present and
     non-blank), append the type extension, print `Saving to <name>`.
   - If omitted and the header has no filename either: use `unnamed` + type
     extension, print a warning to stderr.
6. If no recognizable header was found at all, print a warning to stderr but
   still proceed with best-effort content detection/writing.

### `--raw` mode

For capturing an arbitrary byte stream with **no serial header at all** (e.g.
a hand-written assembly program that just writes bytes to the port):

- Requires an explicit output file (no header to derive a name from).
- Skips all header/length parsing — completion is decided **only** by the
  per-device idle timeout (500 ms PC-1600 / 5000 ms PC-1500 family), even if
  the stream happens to contain header-shaped bytes.
- `--format` and `--skip-header` are ignored; content-based filename/extension
  logic is bypassed entirely.
- Exception: if a header matching the *same device family* as `--device` is
  found (CE-158 for a pc1500/pc1500a target, PC-1600 header for a
  pc1600/pc1600emul target), it is stripped from the saved bytes (treated as
  a real header this time, not payload) — a header for the wrong family is
  left in place, since it's presumably payload data. Under `--verbose`, say
  which happened, e.g. `Stripping detected header` when a matching header was
  removed, or `Not stripping CE-158 header, as device is pc1600emul` when a
  header was found but belongs to the wrong family and was left alone.
- After writing, print the byte count and a 16-bit sum (mod 65536) of all
  saved bytes, so it can be manually cross-checked against a checksum the
  sender computes independently.

## 5. `put` — send to Pocket Computer

Command shape: `put [options] <input-file>`.

Steps:
1. Read the whole input file into memory; error out if empty/unreadable.
2. Detect content type from file bytes (same detector as `get`).
3. If `--start-address` is given, force type = MACHINE regardless of
   detection (trust the user), and ignore `--format` (machine language is
   always sent as binary).
4. Resolve the effective device: from `--device`, overridden by a header
   already present in the file (see §3, including the pc1600 vs. pc1600emul
   nuance).
5. Build the exact byte sequence to transmit:
   - **ASCII BASIC input:** tokenize (already-existing `convert` logic),
     reject if targeting `pc1500`/`pc1500a` and the text contains any
     character above 0x7F (PC-1500 character set is 7-bit ASCII, no CP437
     upper half) — report line/column of the offending character and abort.
     Wrap the tokenized payload in a freshly built header (BASIC type,
     filename derived from the input file's base name, computed length).
   - **Binary BASIC input (already has a header):** send as-is, unmodified.
   - **Machine-language input:** add a header if `--start-address` was given
     and none is already present (see §3); if a header is already present,
     send as-is. If neither a header nor `--start-address` is present, abort
     with an error — unless `--raw` was given, in which case send the raw
     bytes exactly as read, header-less (see §3).
6. Under `--verbose`, report the decisions made along the way: whether a
   header is already present / being added / omitted for machine-language
   transfers (with load and run addresses in the device's native hex width —
   4 hex digits for PC-1500 family, 6 for PC-1600), whether the input was
   tokenized, etc. See §7a for the general "tell what you do" requirement.
7. Open the serial port for the effective device and transmit:
   - **PC-1500 / PC-1500A / PC-1600-emulator (paced send, no working flow
     control):**
     1. Send the header bytes at full port speed.
     2. Pause **300 ms** (let the device process the header).
     3. Send the payload one byte at a time with a **1 ms** delay after each
        byte.
        - PC-1600-emulator specific: a multi-segment BASIC payload (one that
          contains one or more `0xFF 0x00 0x00` segment-boundary markers) gets
          an *extra* **300 ms** pause immediately after each such marker,
          in addition to the normal 1 ms/byte pacing — starting/finishing a
          named segment takes the receiver longer than an ordinary byte.
     4. Drain the OS write buffer (poll until empty or a 2000 ms timeout).
     5. Pause **500 ms** before the port is closed.
   - **PC-1600 (real hardware, RTS/CTS enabled):** write everything at full
     speed, then drain the OS write buffer (same 2000 ms timeout) — the
     hardware handshake does the pacing.
   - Sending an ASCII BASIC listing as discrete lines (only relevant if
     `--format ascii` forces line-by-line ASCII transfer instead of tokenized
     binary) writes each line as CP437 bytes terminated by CR (PC-1500
     family) or CR+LF (PC-1600 family), pausing 500 ms after each line on
     paced devices, followed by an EOF marker: `0x0D` (a second CR) for
     PC-1500 family, `0x1A` (Ctrl-Z) for PC-1600 family. Then drain + 500 ms
     pause as above.

## 6. CLI surface (this scope only)

`get [options] [<output-file>]`
- `-d, --device <pc1500|pc1500a|pc1600|pc1600emul>` (default `pc1500`)
- `-p, --port <name>` (falls back to the config-file default for
  `pc1600emul`, see §2a)
- `-f, --format <ascii|binary>` (default `ascii`; `ascii` on a MACHINE
  transfer is an error — see §4)
- `--skip-header` (binary only)
- `--raw` (requires an output file)
- `--dry-run` (see §7a)
- `-v, --verbose` / `-q, --quiet` (see §7a), `-V` version, `-h` help

`put [options] <input-file>`
- `-d, --device <...>` (optional if the file already has a header)
- `-p, --port <name>` (same config-file fallback as `get`)
- `-f, --format <ascii|binary>` (override auto-detection)
- `--start-address <hex>` (machine language only)
- `--run-address <hex>` (machine language only, requires `--start-address`)
- `--raw` (send a headerless machine-language file as-is even without
  `--start-address` — overrides the error described in §3)
- `--dry-run` (see §7a)
- `-v, --verbose` / `-q, --quiet` (see §7a), `-V`, `-h`

`config <get|set> <key> [<value>]` (new, see §2a) — reads/writes the
per-user config file; the two keys needed for this pass are the default port
for `pc1600emul` and the default verbosity.

(`--add-utils` and any Reserve/Variables-specific options exist in the Java
CLI too but are out of scope here since they either target Reserve
Area/Variables or are cosmetic extras — flag if you want those pulled in as
well; not included in this pass by request.)

## 7a. Verbose reporting and `--dry-run`

- The baseline (no flag, no config setting) is quiet: no play-by-play of
  internal decisions, only the essential output (final filename saved, or
  errors/warnings that matter regardless of verbosity, e.g. "no header
  found").
- `-v`/`--verbose` turns on narration of every non-obvious decision made
  along the way — anything the tool decided on the caller's behalf should be
  stated when verbose is on. Examples already called out above:
  - `get --raw`: whether a detected header was stripped or left alone, and
    why (`Stripping detected header`, or `Not stripping CE-158 header, as
    device is pc1600emul`).
  - `put`: whether the input was tokenized before sending, whether a header
    was already present / freshly added / intentionally omitted (with
    addresses), which device was inferred and from where (CLI flag vs.
    header vs. config file port default).
  - `get`: which content type was detected and why (header found vs. text
    heuristic), which extension was appended and why.
  - This "say what you're doing" principle applies everywhere in `get`/`put`,
    not just the two call-outs above — anywhere behavior branches on
    detected/inferred state, verbose mode should name the branch taken.
- `-q`/`--quiet` turns narration back off. It is not "more quiet than
  default" — there's no lower verbosity level below the baseline described
  above — it exists solely to override a default-verbose config setting
  (§2a) for one invocation. `-q` and `-v` are mutually exclusive on the same
  invocation.
- Effective verbosity resolution, highest precedence first: `-v`/`--verbose`
  on the command line → on; `-q`/`--quiet` on the command line → off;
  otherwise the config file's default verbosity setting if present;
  otherwise off (today's behavior).
- `--dry-run` is independent of the verbose/quiet setting above and does not
  require `-v` to be set: it always prints what it determined it *would* do,
  but performs no actual I/O:
  - `get --dry-run`: don't write any output file. Still run detection/header
    parsing over already-received or already-read bytes and print what would
    have been written and where. (Whether this pairs with real serial
    reception or a supplied capture is an open design question — flag if
    `get --dry-run` without live data doesn't make sense and should instead
    only apply to `convert`/`put`.)
  - `convert --dry-run`: don't write the output file; print what would have
    been produced (target path, direction, size).
  - `put --dry-run`: don't open the serial port or send anything; print the
    fully-formed byte block's summary (header present/added/omitted, size,
    addresses) that would have been transmitted.

## 8. Error handling / edge cases to replicate

- Serial port not found / can't open → clear error, exit non-zero, no silent
  hang (Java: "exits immediately without waiting for data").
- Machine language sent as if it were BASIC (wrong device-side command, e.g.
  `CLOAD` instead of `CLOADM`) is a *device-side* user error the tool can't
  detect — no requirement to guard against it, just don't corrupt data if
  the header type says MACHINE.
- A `put` of a raw machine-language binary lacking both a header and
  `--start-address` is an error (abort, don't guess) unless `--raw` is given,
  in which case it is sent completely unmodified — see §3/§5.
- `put`/`get` never trust the file extension; always detect from content.

## 9. Out of scope (explicitly, per current instructions)

- Reserve Area (`CSAVEr`/`CLOADr`, SDAR format) — PC-1500/1500A only in the
  Java tool; not handled at all here.
- Variables (SDAV format) — PC-1500/1500A only in the Java tool; not handled
  at all here.
- `terminal` mode (interactive passive session) — unrelated to `put`/`get`.
- `--add-utils` (serial utility BASIC sub-program injection) — nice-to-have,
  not required for a first `put`/`get` pass.
- Tokenizer/de-tokenizer internals — already implemented in this project via
  `convert`; `put`/`get` should reuse whatever exists here rather than port
  the Java tokenizer, even where behavior differs.
