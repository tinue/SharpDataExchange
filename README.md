# SharpDataExchange (`sde`)

Transfer BASIC programs and machine-language code to/from a Sharp PC-1500 /
PC-1500A / PC-1600 pocket computer over serial, and tokenize/de-tokenize BASIC
listings offline — no Java runtime, a single self-contained binary. This is the
Rust reimplementation of the Java
[`SharpDataExchange`](https://github.com/tinue/SharpDataExchange), with
**byte-identical `convert` output** to the Java tool on the checked-in fixtures.

Embedding this in your own application (C / C++ / Swift / Rust)? See
[`library.md`](library.md) — that covers only the offline tokenize/de-tokenize
core; `get`/`put`/`config` are CLI-only.

## Table of Contents

1. [Install](#install)
2. [Hardware Setup](#hardware-setup)
3. [Quick Start](#quick-start)
4. [Concepts](#concepts)
5. [Command Reference](#command-reference)
6. [Pocket Computer Commands](#pocket-computer-commands)
7. [Data Formats](#data-formats)
8. [Common Workflows](#common-workflows)
9. [Troubleshooting](#troubleshooting)
10. [Scope / Known Limitations](#scope--known-limitations)
11. [Development](#development)
12. [License](#license)

---

## Install

### From a release

Download for your platform from the [Releases](../../releases) page. The
`.tar.gz` / `.zip` archives hold `bin/sde` plus the C library and header (see
`library.md`), the license, and this document — unpack and put `bin/sde` on your
`PATH`.

| download | platform |
|---|---|
| `SharpDataExchange-<version>.pkg` | macOS 12+ (Apple Silicon) — graphical installer |
| `sharpdx-macos-arm64.tar.gz` | macOS 12+ (Apple Silicon) — archive |
| `sharpdx-linux-x86_64.tar.gz` | Linux x86-64 (glibc 2.35+) |
| `sharpdx-linux-aarch64.tar.gz` | Linux arm64 (glibc 2.35+) |
| `sharpdx-windows-x86_64.zip` | Windows 10+ x64 |
| `sharpdx-windows-aarch64.zip` | Windows 11 arm64 |

### On macOS

Easiest: double-click **`SharpDataExchange-<version>.pkg`**. It is signed and
notarized (no Gatekeeper warning) and asks whether to install:

* **for all users** → `/usr/local/bin/sde` (already on `PATH`); or
* **for me only** → `~/.local/bin/sde`, creating that directory and adding it to
  `~/.zshrc` when needed — open a new terminal afterwards.

Prefer to place the binary yourself: use `sharpdx-macos-arm64.tar.gz`. Its `sde`
and `libsharpdx.dylib` are Developer-ID-signed and notarized (Gatekeeper verifies
online on first run); the archive itself carries no stapled ticket.

### From source

```
cargo build --release
```

`target/release/sde` is the CLI; it links only the system C library.

---

## Hardware Setup

Photos and build notes (adapter board, wiring, CE-158X) are in
[`docs/HardwareNotes.md`](docs/HardwareNotes.md).

### PC-1500 / PC-1500A: CE-158X

The PC-1500 and PC-1500A do not have a built-in serial port. The original Sharp
CE-158 add-on provided serial and parallel ports; the modern CE-158X by Jeff Birt
(available at soigeneris.com) is the recommended replacement.

Connect the CE-158X to the PC via its USB port (labeled **U1**). No driver
configuration is necessary on modern systems; the device appears as a standard
USB serial adapter.

### PC-1600: USB/UART Adapter

The PC-1600 has a built-in 5V TTL serial port via a 15-pin connector. Connect it
to the PC using an FTDI USB/UART adapter (e.g. TTL-232R-5V-WE). The adapter
signals must be **inverted** before use — a one-time operation using FTDI's
configuration utility on a Windows machine.

Wiring (pin 1 is the rightmost pin of the PC-1600's 15-pin connector):

| PC-1600 pin | Signal | Adapter wire |
|---|---|---|
| 2 | TX | RX (yellow) |
| 3 | RX | TX (orange) |
| 4 | RTS | CTS (brown) |
| 5 | CTS | RTS (green) |
| 7 | Ground | Ground (black) |

Do not connect the red (5V) wire of the adapter.

### Serial Port Auto-Detection

`sde` automatically detects the serial port for `pc1500`, `pc1500a`, and
`pc1600`:

- **macOS**: looks for `cu.usb*`
- **Linux**: looks for `ttyACM*` or `ttyUSB*`
- **Windows**: not yet implemented — always pass `--port` explicitly

If more than one matching port is present, or auto-detection isn't supported on
your platform, use `-p`/`--port` to specify the port explicitly.

### Emulators (`pc1600emul`)

To exchange data with a PC-1600 emulator instead of real hardware, use
`--device pc1600emul`. The emulator is reached over a host pseudo-terminal, which
the Calc-U-1600 emulator creates while it runs as a file always named
`calcu1600.serial` inside some directory. Pseudo-terminal paths are never
auto-detected, so `sde` needs to know that directory — either given
per-invocation with `--port <full-path-to-calcu1600.serial>`, or configured
once as a default **directory** (see [Config file](#config-file)):

```
sde config set pc1600emul.port /tmp/my-emulator-dir
sde put myprogram.bas --device pc1600emul       # uses /tmp/my-emulator-dir/calcu1600.serial
```

`sde config set pc1600emul.port <dir>` stores the *directory*, not the full
port path — `sde` always appends the fixed filename `calcu1600.serial` itself.
If you never set this, the default directory is `/tmp`, so
`--device pc1600emul` with no `--port` and no config resolves to
`/tmp/calcu1600.serial` out of the box.

`pc1600emul` sends the same data as `pc1600` but drops RTS/CTS hardware flow
control (a pseudo-terminal has no handshake lines) and paces the transfer like
the PC-1500.

---

## Quick Start

### Tokenize a BASIC listing offline (no hardware needed)

```
sde convert myprogram.bas       # -> myprogram.bbin (CE-158 header + tokens)
```

### Receive a BASIC program from the PC-1500

On the **PC-1500**: `SETDEV U1,CI,CO` then `CSAVE`.

On the **PC**, run `sde get` *before* triggering `CSAVE` (the device doesn't
buffer — see [Who goes first](#who-goes-first)):

```
sde get
```

The program is saved as readable ASCII BASIC; `sde` derives the filename from
the received header and de-tokenizes the binary automatically.

### Send a BASIC program to the PC-1500

On the **PC-1500**: `SETDEV U1,CI,CO`, then issue `CLOAD` *before* running `put`:

```
sde put myprogram.bas
```

### Receive from / send to the PC-1600

```
sde get myprogram.bas --device pc1600
sde put myprogram.bas --device pc1600
```

See [Pocket Computer Commands](#pocket-computer-commands) for the PC-1600's
one-time-per-session serial setup commands.

---

## Concepts

### Two transfer commands: `get` and `put`

Both commands describe what the *PC* does:

- **`get`** — the PC receives data from the Pocket Computer and saves it to a file
- **`put`** — the PC reads a file and sends it to the Pocket Computer

On the Pocket Computer side, `get` corresponds to a **save** command (e.g.
`CSAVE`), and `put` corresponds to a **load** command (e.g. `CLOAD`).

A third command, **`convert`**, needs no Pocket Computer: it tokenizes or
de-tokenizes a BASIC file on the PC alone.

### Who goes first

For `get`: start `sde get` first, then issue the save command on the Pocket
Computer. For `put`: issue the load command on the Pocket Computer first, then
run `sde put`. The device does not buffer outgoing data, so the ordering matters.

### Content detection, never file names

`sde` identifies data type from its content, not the file name or extension —
`.bas`, `.bin`, or any other extension can be used freely for `get`/`put`
input/output files.

### Scope: BASIC, machine language, Reserve Area, and Variables

`get`/`put` handle **BASIC programs, machine-language (assembly) programs, Reserve
Area, and Variables**. Reserve Area and Variables are PC-1500/1500A-only and are not
reachable through `convert` — see [Data Formats](#data-formats) for their layout.

### Verbosity: `-v` / `-q` / config default

By default, `get`/`put` run quietly — only the essential result is printed
(the saved filename, a warning that matters regardless of verbosity, or an
error). `-v`/`--verbose` narrates every non-obvious decision made along the way
(which device was inferred and from where, whether a header was added/stripped,
which content type was detected, etc.) to stderr. `-q`/`--quiet` is *not* a
lower verbosity than the default — it exists only to force narration off for
one invocation when a config-file default has turned it on. Precedence:
`-v` > `-q` > config file `verbose` setting > off.

### `--dry-run`

`get --dry-run` still performs a **real serial receive** (it opens the port and
waits for actual data, exactly like a normal `get`) and runs detection on what
it received, but never writes the output file. `put --dry-run` never opens the
serial port at all — it reports what would have been sent. Neither requires
`-v` to print its report. (`convert` does not currently have a `--dry-run` flag.)

### Config file

`sde` reads optional defaults from `~/.sderc` (`%USERPROFILE%\.sderc` on
Windows) — a plain `key = value` text file, modeled on `git config --global`
but simpler (no sections, no nesting). It's entirely optional; nothing breaks
if it's absent. Manage it with `sde config get/set` rather than hand-editing,
though the format is simple enough to edit directly if you prefer.

Two keys are used today:

| key | meaning |
|---|---|
| `pc1600emul.port` | **directory** holding the `pc1600emul` pseudo-terminal socket, whose filename is always `calcu1600.serial`. Defaults to `/tmp` when unset — i.e. `/tmp/calcu1600.serial` — if `--port` is also not given. |
| `verbose` | default verbosity (`true`/`false`) when neither `-v` nor `-q` is given |

An explicit `--port`/`-v`/`-q` on the command line always overrides the config
file.

```
sde config set pc1600emul.port /tmp/my-emulator-dir
sde config set verbose true
sde config get verbose
sde config get pc1600emul.port
```

---

## Command Reference

### `get` — Receive from Pocket Computer

```
sde get [options] [<output-file>]
```

Waits for the Pocket Computer to send data, then writes it to `<output-file>`.

If `<output-file>` is omitted, the filename is derived from the serial header
sent by the Pocket Computer; if the header also lacks a filename, `unnamed` is
used (a warning is printed in both fallback cases). If a filename is given but
lacks an extension, one is appended based on the detected data type (`.bas` for
BASIC, `.bin` for machine language).

| Option | Description |
|---|---|
| `-d`, `--device <device>` | Target device: `pc1500` (default), `pc1500a`, `pc1600`, `pc1600emul`. Configures baud rate and flow control before data arrives; decoding uses the device recorded in the received header when one is found, so an incorrect `--device` doesn't corrupt the output content — only the transport timing. |
| `-p`, `--port <port>` | Serial port name (auto-detected if omitted; see [Serial Port Auto-Detection](#serial-port-auto-detection)). |
| `-f`, `--format <format>` | Output format: `ascii` (default), `binary`. `ascii` on machine-language content is rejected — machine language can't be de-tokenized, so pass `--format binary` for it. |
| `--skip-header` | Omit the serial header from the saved binary file (`--format binary` only). The resulting file can't be auto-identified or reloaded by `sde` without it — a warning is printed. |
| `--raw` | Dump the received bytes verbatim, with no header/content detection at all. Requires an output file. See below. |
| `--dry-run` | Perform the real receive, run detection, but skip the file write — reports what would have been written. |
| `-v`, `--verbose` / `-q`, `--quiet` | See [Verbosity](#verbosity--v---q--config-default). |
| `-h`, `--help` | Print help. (`--version`/`-V` is top-level only: `sde --version`, not `sde get --version`.) |

**`--raw`:** for capturing an arbitrary byte stream that carries no serial
header at all — for example, a hand-written assembly program that just writes
bytes to the port. `--device` still selects baud rate and flow control, and the
per-device idle timeout (500 ms for PC-1600 family, 5000 ms for PC-1500 family)
that normally serves as a fallback end-of-transfer detector becomes the *only*
way the transfer is considered complete — no header/length parsing is
attempted, so header-shaped bytes inside the stream are never misinterpreted.
If a header matching the *same device family* as `--device` is found, it's
stripped from the saved bytes (verbose: "Stripping detected header"); a header
for the wrong family is left in place (verbose: "Not stripping `<flavor>`
header, as device is `<device>`"). After receiving, `sde` prints the byte count
and a 16-bit sum (mod 65536) of the saved bytes, for a manual cross-check
against a checksum the sender computes independently — printed even under
`--dry-run`, since it describes what was *received*, not whether it was saved.

### `put` — Send to Pocket Computer

```
sde put [options] <input-file>
```

Reads `<input-file>` and sends it to the Pocket Computer. Content is always
detected from the file's bytes, never its name.

| Option | Description |
|---|---|
| `-d`, `--device <device>` | Target device. Optional if the file already carries a recognized header — the device is then inferred from it (an explicit `--device` of the wrong *family* is an error; if the header is PC-1600 and neither `pc1600` nor `pc1600emul` is given, that's ambiguous and also an error — `sde` doesn't guess). Without a header and without `--device`, defaults to `pc1500`. |
| `-p`, `--port <port>` | Serial port name (auto-detected if omitted). |
| `-f`, `--format <format>` | For headerless ASCII BASIC input: `binary` (default when omitted) tokenizes it before sending; `ascii` sends it line-by-line, untokenized (slower; mirrors the device's `CLOADa`/ASCII load). Has no effect on machine language (always sent as raw binary) or on input that already has a header (always sent as-is). |
| `--start-address <hex>` | Load address for a **headerless** machine-language input (e.g. `38C5` or `0x38C5`). Required to send headerless machine code — see below. |
| `--run-address <hex>` | Auto-run address for a headerless machine-language input; requires `--start-address`. Defaults to `0xFFFF` (no auto-run) if `--start-address` is given without it. |
| `--raw` | Send a headerless machine-language file exactly as read, unmodified — even without `--start-address`. Overrides the error below. |
| `--dry-run` | Report what would be sent (header present/added/omitted, size, addresses), without opening the serial port. |
| `-v`, `--verbose` / `-q`, `--quiet` | See [Verbosity](#verbosity--v---q--config-default). |
| `-h`, `--help` | Print help. (`--version`/`-V` is top-level only: `sde --version`, not `sde put --version`.) |

**Header handling:**
- A file that already carries a recognized CE-158/PC-1600 header is sent as-is.
- ASCII BASIC input is tokenized and wrapped in a freshly built header — unless
  `--format ascii` is given explicitly, in which case it's sent line-by-line,
  untokenized, with no header at all (see `-f`/`--format` above).
- Headerless machine-language input needs `--start-address` — `sde` builds a
  header from it (and `--run-address`, default `0xFFFF`). **Without** a header,
  `--start-address`, or `--raw`, `put` refuses rather than guessing what the
  bytes are.

### `convert` — Tokenize / de-tokenize a BASIC file offline

```
sde convert [options] <infile> [<outfile>]
```

No Pocket Computer or serial connection involved. Direction is chosen from file
content:

* ASCII BASIC in → tokenized BASIC out (wrapped in a CE-158 or PC-1600 header,
  chosen by `--device`), so the result can be sent later with `put` or read
  back with `convert`.
* Tokenized BASIC in → ASCII BASIC out. The input **must** carry a CE-158 or
  PC-1600 header; a headerless tokenized payload is rejected.

Any other content (Reserve Area, Variables, machine code, unrecognized) is
rejected — `convert` is BASIC-only; use `get`/`put` for machine language.

`.bas` is the extension for ASCII listings, `.bbin` for tokenized BASIC. The
input extension must agree with its actual content. With no `<infile>` and data
on stdin, `sde` reads stdin and writes the converted bytes to stdout (in that
mode, direction is always ASCII→tokenized).

| Option | Description |
|---|---|
| `-d`, `--device <device>` | `pc1500` (default), `pc1500a`, `pc1600`, `pc1600emul`. Selects the keyword table + header flavor when tokenizing. Ignored when de-tokenizing (device comes from the input header). |
| `--eol <eol>` | Line ending for a de-tokenized listing: `auto` (default; CRLF on Windows, LF elsewhere), `lf`, `crlf`, `cr`. Ignored when tokenizing — CR and CRLF input are always accepted. |
| `-v`, `--verbose` | Verbose logging. |

```
sde convert myprogram.bas
    → myprogram.bbin  (CE-158 header + tokens)

sde convert -d pc1600 myprogram.bas out.bbin
    → out.bbin  (PC-1600 header + tokens)

sde convert myprogram.bbin
    → myprogram.bas  (readable listing)

cat myprogram.bas | sde convert > myprogram.bbin
```

### `config` — Read/write a default in `~/.sderc`

```
sde config get <key>
sde config set <key> <value>
```

See [Config file](#config-file) for the keys and the precedence rules.

---

## Pocket Computer Commands

### Sharp PC-1500 / PC-1500A

#### One-time serial port setup

The PC-1500 uses the CE-158X's USB port (U1). Before each session, redirect
serial I/O to that port (must be re-entered after each power cycle or `NEW`):

```
SETDEV U1,CI,CO
```

#### Receive a program from the PC (`put`)

Issue the load command on the PC-1500 **before** starting `put`:

| To receive... | Command |
|---|---|
| Tokenized BASIC (binary, default) | `CLOAD` |
| ASCII BASIC | `CLOADa` |
| Machine language | `CLOADM` |

`sde` sends BASIC as tokenized binary by default for the fastest, most reliable
transfer. Machine language is always binary and always requires `CLOADM` —
`CLOAD`/`CLOADa` only load BASIC programs.

#### Send a program to the PC (`get`)

Start `sde get` **first**, then issue the save command on the PC-1500:

| To send... | Command |
|---|---|
| Tokenized BASIC (binary, default) | `CSAVE` or `CSAVE"filename"` |
| ASCII BASIC | `CSAVEa` |
| Machine language | `CSAVE M start,end[,run]` (or `CSAVEM`) |

Machine language is always saved with `CSAVE M`, giving the start/end addresses
of the memory block to send. See [`CSAVE M` in the CE-150
reference](https://github.com/tinue/SharpBasicReference/blob/main/CE-150-Reference.md#csave-m--save-machine-language)
for the full syntax.

### Sharp PC-1600

#### Serial port setup (after each power-on or reset)

```
SETCOM "COM1:",9600,8,N,1,N,N
INIT "COM1:",4096
OUTSTAT "COM1:"
RCVSTAT "COM1:",24
```

For sending **from** the PC-1600 to the PC, additionally enter:

```
SNDSTAT "COM1:",24
```

These configure the port at 9600 baud, 8 data bits, no parity, 1 stop bit, with
RTS/CTS hardware flow control, and a 4096-byte buffer.

#### Receive a program from the PC (`put`)

Issue the load command **before** starting `put`. The PC-1600 auto-detects
ASCII vs. binary:

```
LOAD "COM1:"          ' or LOAD "COM1:",R to auto-run after loading
```

#### Send a program to the PC (`get`)

Start `sde get` **first**, then on the PC-1600:

```
SAVE "COM1:"
```

#### PC-1600 serial command reference

| Command | Purpose |
|---|---|
| `SETCOM "COM1:",<baud>,<bits>,<parity>,<stop>,<xon>,<shift>` | Configure baud rate and protocol. |
| `INIT "COM1:",<buffer-size>` | Set receive buffer size (default after power-on is 40 bytes). |
| `OUTSTAT "COM1:"` | Enable dynamic RTS/DTR flow control. |
| `RCVSTAT "COM1:",<protocol>[,<timeout>]` | Receive handshake/timeout; `24` enables CTS handshake. |
| `SNDSTAT "COM1:",<protocol>[,<timeout>]` | Send handshake/timeout; `24` enables CTS, `28` disables all flow control. |
| `SETDEV "COM1:"[,KI][,PO]` | Redirect `INPUT`/`LPRINT`/`LLIST` to the serial port. |
| `PCONSOLE "COM1:",<line-length>,<eol>` | Line length and EOL (`0`=CR, `1`=LF, `2`=CR/LF) for serial output. |

---

## Data Formats

### ASCII BASIC

Standard Sharp BASIC source code, one line per line number, full keyword names
(not abbreviations):

```
10 FOR I=1 TO 10
20 PRINT I
30 NEXT I
```

This is `get`'s default output for any BASIC program, whether the device sent
it as binary or ASCII.

### Binary (tokenized BASIC / machine language)

Raw binary format as used by the Pocket Computer's cassette/serial interface.
The PC-1500 family uses a 27-byte CE-158 header; the PC-1600 uses a 16-byte
header. Both carry a type field (BASIC or machine language), a payload length,
and — for machine language — load/run addresses.

Use `--format binary` with `get` to save the file in binary form (header
included by default, so the file can be sent straight back with `put`
unmodified). Machine language is *always* saved as raw binary regardless of
`--format`, since it can't be de-tokenized to a listing.

### Reserve Area (SDAR)

PC-1500/1500A only (`CSAVEr`/`CLOADr`); not reachable through `convert`, only
`get`/`put`. The binary payload behind a CE-158 header is a fixed 188 bytes: three
26-byte key-layer labels, followed by a 110-byte key-content pool. The pool is a
sequence of `[key code][content bytes...]` entries terminated by `0x00`; each of the
three layers has six keys, with key codes `0x01`-`0x06` (layer 1), `0x11`-`0x16`
(layer 2), and `0x09`-`0x0E` (layer 3). A key with no assigned content has no entry.
Content bytes mix literal characters with 2-byte BASIC keyword tokens.

`get --format ascii` (the default) renders this as SDAR text:

```
; sde-reserve:1.0 pc1500
; Filename: MYAPP

[layer 1]
label: MENU
key 1: PRINT
key 2:
key 3:
key 4:
key 5:
key 6:

[layer 2]
...
[layer 3]
...
```

All three `[layer N]` sections and all six `key N:` lines are always present (content
may be empty). `put` accepts this text back and tokenizes it, erroring if the encoded
pool would exceed the 110-byte hardware limit. `.sdar` is the extension for this ASCII
form.

### Variables (SDAV)

PC-1500/1500A only; not reachable through `convert`, only `get`/`put`. The binary
payload behind a CE-158 header is a sequence of records with no fixed total length —
each record is a `0x00` separator, a 4-byte prefix (record length, array dimension,
reserved, type), then data. A `0x88` type byte marks numeric data (8-byte
PC-1500-hardware BCD floats — see `src/bcd.rs`'s doc comment for the exact byte
layout); any other type byte is a string element's length in bytes. A `0` dimension
byte marks a scalar; a nonzero value `N` marks an array of `N + 1` elements (a
`DIM`'d variable).

`get --format ascii` (the default) renders this as SDAV text:

```
; sde-variables:1.0 pc1500
; Count: 3
; Filename: MYAPP
42
"Hi there!"
DIM (2)
1
2
3
```

`; Count:` is the number of top-level records (one per scalar or whole array); `put`
recomputes it from the parsed text and errors on a mismatch. A numeric array is a
`DIM (<dimMax>)` header followed by `dimMax + 1` decimal lines; a string array is a
`DIM $(<dimMax>)*<maxLen>` header followed by `dimMax + 1` double-quoted lines
(`\`, `"`, and non-printable bytes escaped as `\\`, `\"`, `\xHH`). `.sdav` is the
extension for this ASCII form.

Whether the PC-1600 protocol has equivalent header types for either format is
unresearched — `sde` recognizes Reserve Area/Variables only behind a CE-158 (PC-1500)
header.

---

## Common Workflows

### Transfer a BASIC program from the PC-1500 and keep it editable

```
sde get myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CSAVE` — `sde` de-tokenizes the binary
and writes a readable ASCII listing.

### Send a modified program back

```
sde put myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOAD`.

### Send as ASCII instead of binary (e.g. to diagnose a tokenization problem)

```
sde put --format ascii myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOADa`.

### Load a machine-language program onto the PC-1500

```
sde put --start-address 38C5 --run-address 38C5 program.bin
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOADM`.

### Save a machine-language program from the PC-1500

Start `sde get` **first**, then on the PC-1500, e.g. `CSAVE M &38C5,&3A00`:

```
sde get program.bin --format binary
```

### Set up `pc1600emul` once, then omit `--port` every time

```
sde config set pc1600emul.port /tmp/my-emulator-dir
sde put myprogram.bas --device pc1600emul     # uses /tmp/my-emulator-dir/calcu1600.serial
sde get --device pc1600emul
```

Or rely on the built-in `/tmp` default and skip `config set` entirely, if your
emulator happens to create `/tmp/calcu1600.serial`.

### Preview a `put` without touching hardware

```
sde put --dry-run --start-address 38C5 program.bin
```

### Specify the serial port manually

```
sde get myprogram.bas --port /dev/cu.usbserial-A50285BI
```

---

## Troubleshooting

**`sde` exits immediately with a port error (`get`/`put`)**
The serial port wasn't found or auto-detection was ambiguous/unsupported on
your platform (see [Serial Port Auto-Detection](#serial-port-auto-detection)).
Use `--port` to specify it explicitly.

**`ERROR 67` on the PC-1500 when loading ASCII**
A line in the program exceeds 80 characters. Load as binary instead (omit `a`
from `CLOADa`).

**`ERROR 61` on the PC-1500 when loading binary**
The binary file lacks a CE-158 header. `sde` adds the header automatically when
sending a recognized ASCII/machine-language input; this shouldn't occur with
files produced by `sde get`. For a third-party binary, supply `--start-address`
or check it already has a valid header.

**Machine-language program loads but reports a garbled/wrong header, or won't run**
`CLOAD` only loads BASIC programs — it doesn't understand a machine-language
payload even with a correct header. Use `CLOADM` on the PC-1500. Likewise, use
`CSAVE M start,end[,run]` (not plain `CSAVE`) to send machine language to `get`.

**`put` refuses a machine-language file with "needs --start-address"**
The input has no recognizable header and no `--start-address` was given. Either
supply `--start-address` (and optionally `--run-address`), or pass `--raw` if
the bytes are intentionally headerless and should be sent exactly as-is.

**`get` refuses with "cannot produce an ASCII listing from this content"**
The received/detected content is machine language, and `--format` was `ascii`
(the default). Machine language can't be de-tokenized — pass `--format binary`.

**`could not open serial port ...: Not a typewriter` for `--device pc1600emul`**
This was a real bug (fixed): the `serialport` crate always applies the baud
rate via a macOS ioctl (`IOSSIOSPEED`) that pseudo-terminals reject with
`ENOTTY`, so opening any emulator pty through it failed unconditionally. `sde`
now talks to `pc1600emul`'s pty as a plain file descriptor instead (no
OS-level baud/parity/flow-control — meaningless for a local pty anyway, since
the emulator paces bytes on its own side and `sde` already paces its
`pc1600emul` sends itself). If you see this error, you're on a build predating
that fix — update `sde`. Real hardware (`pc1500`/`pc1500a`/`pc1600`) is
unaffected; that ioctl works fine against an actual UART driver.

**Data corruption or incomplete transfer (PC-1500)**
The PC-1500 requires paced transmission; `sde` applies a 1 ms delay between
bytes and a 300 ms pause after the header automatically. If problems persist,
check the USB cable and CE-158X connection.

**The PC-1600 serial port stops responding**
Re-enter the full setup sequence on the PC-1600:
```
SETCOM "COM1:",9600,8,N,1,N,N
INIT "COM1:",4096
OUTSTAT "COM1:"
RCVSTAT "COM1:",24
```

---

## Scope / Known Limitations

Implemented: `convert` (offline tokenize/de-tokenize), `get`/`put` (serial
transfer) for **BASIC programs, machine-language programs, Reserve Area, and
Variables** (the latter two PC-1500/1500A-only, `get`/`put`-only), and `config`
(per-user defaults).

Won't implement (permanent decisions, not just currently-undone work):

- **`terminal`** mode (interactive passive serial session) — the serial link isn't
  full-duplex-capable (only input or output can be redirected at a time), so an
  interactive terminal has little value here.
- **`--add-utils`** (serial utility BASIC sub-program injection on `put`).
- Additional `--dry-run` support beyond what `get`/`put` already have (`convert
  --dry-run` stays unimplemented).
- Windows serial-port auto-detection — Windows machines routinely enumerate many COM
  ports (Bluetooth, virtual devices, etc.), making auto-detection unreliable; always
  pass `--port` there.

Not implemented (may change):

- Whether the PC-1600 protocol has header types equivalent to Reserve Area/Variables
  at all is **unresearched** — `sde` does not recognize any PC-1600 header type byte
  beyond BASIC/machine language.
- PC-1600-only BASIC token values are limited to what the Java
  `Pc1600Keywords.java` lists (no PC-1600 ROM source exists); unknown `>= 0xE0`
  byte pairs pass through opaquely during `convert`.

The one known `convert` fixture difference is the CE-158 header *filename* field
casing — `sde` upper-cases it, while a historical fixture stored it lower-case; the
byte-parity test compares those header bytes separately from the rest and treats them
as "don't care". Payloads match exactly.

---

## Development

```
cargo test          # byte-parity, round-trip, unit, and C-ABI tests
cargo fmt
cargo clippy --all-targets -- -D warnings
```

`src/keywords.rs` is generated from the Java `device/*Keywords.java` sources by
`tools/extract_keywords.py`:

```
python3 tools/extract_keywords.py            # rewrite src/keywords.rs
python3 tools/extract_keywords.py --check    # CI drift check (also a cargo test)
```

`get`/`put`'s serial transport, config file, and pacing/receive logic are unit
tested against an in-memory transport double (no real hardware needed to run
`cargo test`). Manually verified: real serial transfers against a real PC-1600 on
macOS, and port auto-detection on macOS. Still untested: real PC-1500/CE-158
transfers, and port auto-detection on Windows and Linux.

Releases are cut with [`bin/release`](bin/release); see the comment header in
that script and [`CHANGELOG.md`](CHANGELOG.md).

## License

[Polyform Noncommercial License 1.0.0](LICENSE) — free for any noncommercial
purpose. Copyright 2026 Martin Erzberger.
