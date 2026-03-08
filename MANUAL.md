# SharpDataExchange User Manual

SharpDataExchange transfers data between a modern PC and Sharp Pocket Computers (PC-1500, PC-1500A, PC-1600) via a serial/USB connection. It handles BASIC programs, machine language, Reserve Area contents, and variable data.

---

## Table of Contents

1. [Requirements](#requirements)
2. [Hardware Setup](#hardware-setup)
3. [Quick Start](#quick-start)
4. [Concepts](#concepts)
5. [Command Reference](#command-reference)
6. [Pocket Computer Commands](#pocket-computer-commands)
7. [Data Formats](#data-formats)
8. [Common Workflows](#common-workflows)
9. [Troubleshooting](#troubleshooting)

---

## Requirements

- Java 21 or higher
- A command line (Terminal, PowerShell, etc.)
- A serial connection between your PC and the Pocket Computer (see [Hardware Setup](#hardware-setup))

Run the tool as:

```
java -jar SharpDataExchange.jar <command> [options] <file>
```

---

## Hardware Setup

### PC-1500 / PC-1500A: CE-158X

The PC-1500 and PC-1500A do not have a built-in serial port. The original Sharp CE-158 add-on provided serial and parallel ports; the modern CE-158X by Jeff Birt (available at soigeneris.com) is the recommended replacement and is what this software is tested against.

Connect the CE-158X to the PC via its USB port (labeled **U1**). No driver configuration is necessary on modern systems; the device appears as a standard USB serial adapter.

### PC-1600: USB/UART Adapter

The PC-1600 has a built-in 5V TTL serial port via a 15-pin connector. Connect it to the PC using an FTDI USB/UART adapter (e.g. TTL-232R-5V-WE). The adapter signals must be **inverted** before use — this is a one-time operation using FTDI's configuration utility on a Windows machine.

Wiring (pin 1 is the rightmost pin of the PC-1600's 15-pin connector):

| PC-1600 pin | Signal | Adapter wire |
|---|---|---|
| 2 | TX | RX (yellow) |
| 3 | RX | TX (orange) |
| 4 | RTS | CTS (brown) |
| 5 | CTS | RTS (green) |
| 7 | Ground | Ground (black) |

Do not connect the red (5V) wire of the adapter.

> **Note for Apple Silicon Mac users:** A hardware bug in the macOS RTS/CTS driver causes data loss when sending to the PC-1600. The workaround is to build on the Mac and run SharpDataExchange on a Raspberry Pi connected to the PC-1600 via a remote SSH session.

### Serial Port Auto-Detection

SharpDataExchange automatically detects the serial port:

- **macOS**: looks for `cu.usb*`
- **Linux / Raspberry Pi**: looks for `ttyACM*` or `ttyUSB*`

If more than one matching port is present, use `-p` to specify the port explicitly.

---

## Quick Start

### Receive a BASIC program from the PC-1500

On the **PC-1500**, enter:
```
SETDEV U1,CI,CO
CSAVE
```

On the **PC**, run:
```
java -jar SharpDataExchange.jar get
```

The program is saved as readable ASCII BASIC. SharpDataExchange derives the filename from the received header and de-tokenizes the binary data automatically.

---

### Send a BASIC program to the PC-1500

On the **PC-1500**, enter:
```
SETDEV U1,CI,CO
CLOAD
```

On the **PC**, run:
```
java -jar SharpDataExchange.jar put myprogram.bas
```

---

### Receive a BASIC program from the PC-1600

On the **PC-1600**, enter the serial setup (once after power-on or reset):
```
SETCOM "COM1:",9600,8,N,1,N,N
INIT "COM1:",4096
OUTSTAT "COM1:"
RCVSTAT "COM1:",24
```

Then save:
```
SAVE "COM1:"
```

On the **PC**, run:
```
java -jar SharpDataExchange.jar get myprogram.bas --device pc1600
```

---

### Send a BASIC program to the PC-1600

After completing the serial setup above, on the **PC-1600** enter:
```
SNDSTAT "COM1:",24
LOAD "COM1:"
```

On the **PC**, run:
```
java -jar SharpDataExchange.jar put myprogram.bas --device pc1600
```

---

## Concepts

### Two commands: `get` and `put`

All transfers involve the Pocket Computer as one party. The two commands describe what the PC does:

- **`get`** — the PC receives data from the Pocket Computer and saves it to a file
- **`put`** — the PC reads a file and sends it to the Pocket Computer

On the Pocket Computer side, `get` corresponds to a **save** command (e.g. `CSAVE`), and `put` corresponds to a **load** command (e.g. `CLOAD`). This naming avoids confusion with the Pocket Computer's own `SAVE` and `LOAD` commands.

### Who goes first

For `get`: start the PC command first, then issue the save command on the Pocket Computer.
For `put`: issue the load command on the Pocket Computer first, then start the PC command.

The exact ordering matters because the Pocket Computer does not buffer data. The PC waits for incoming data (`get`), but the Pocket Computer does not wait when sending (`put`).

### ASCII by default

When receiving data from the Pocket Computer, SharpDataExchange always saves it as ASCII by default. The recommended way to send data from the Pocket Computer is the standard binary save command (`CSAVE` / `SAVE "COM1:"`): it is faster than ASCII transfer and SharpDataExchange de-tokenizes the result automatically.

- A program saved with `CSAVE` or `SAVE "COM1:"` (binary) is de-tokenized and written as readable ASCII BASIC.
- Reserve Area data is written in a readable structured format (SDAR) showing the three layers, labels, and key definitions with BASIC keywords de-tokenized. (**PC-1500 only**; the PC-1600 does not support Reserve Area transfer via serial).
- Variable data is written in a readable key-value format (SDAV). (**PC-1500 only**; the PC-1600 does not support variable transfer via serial).

The ASCII save variants (`CSAVEa`, `SAVE "COM1:",A`) still work and produce the same result on the PC, but are slower and are not normally needed.

Use `--format binary` to skip conversion and save the raw binary instead.

### Content detection

SharpDataExchange identifies the type of data from its content, not from the file name or extension. This means `.bas`, `.bin`, or any other extension can be used freely.

---

## Command Reference

### `get` — Receive from Pocket Computer

```
java -jar SharpDataExchange.jar get [options] [<output-file>]
```

Waits for the Pocket Computer to send data, then writes it to `<output-file>`.

If `<output-file>` is omitted, the filename is derived from the serial header sent by the Pocket Computer. If the header also lacks a filename, `unnamed` is used. A warning is printed to the console in these cases.

If a filename is provided but lacks an extension (no dot in the name), the appropriate extension is appended automatically based on the data type:

| Data Type | Extension |
|---|---|
| BASIC program | `.bas` |
| Reserve Area (PC-1500 only) | `.sdar` |
| Variables (PC-1500 only) | `.sdav` |
| Machine code | `.bin` |

| Option | Description |
|---|---|
| `-d`, `--device <device>` | Target device: `pc1500` (default), `pc1500a`, `pc1600`. Required to configure the serial port (baud rate and handshaking) before data arrives. The received header is used for decoding, so an incorrect `--device` does not affect the output content. |
| `-p`, `--port <port>` | Serial port name (auto-detected if omitted) |
| `-f`, `--format <format>` | Output format: `ascii` (default), `binary` |
| `--skip-header` | Omit the serial header from the saved binary file (`--format binary` only). Not recommended — see warning below. |
| `-v`, `--verbose` | Verbose logging |
| `-vv`, `--debug` | Debug logging |
| `-V`, `--version` | Print version and exit |
| `-h`, `--help` | Print help |

**Output formats for `get`:**

| Format | Description |
|---|---|
| `ascii` | Human-readable ASCII. BASIC programs use full keyword names. Reserve Area uses SDAR format. Variables use SDAV format. Default. |
| `binary` | Raw binary as received, including the serial header. The resulting file can be sent back with `put` without re-tokenizing. |

> **Warning — `--skip-header`:** Without the header, SharpDataExchange cannot identify the file type and will refuse to load it. Only use this option when you need a raw payload for an external tool. A warning is printed to the console when `--skip-header` is active.

### `put` — Send to Pocket Computer

```
java -jar SharpDataExchange.jar put [options] <input-file>
```

Reads `<input-file>` and sends it to the Pocket Computer.

| Option | Description |
|---|---|
| `-d`, `--device <device>` | Target device: `pc1500` (default), `pc1500a`, `pc1600`. Optional when sending a binary file that already has a header — the device is inferred from the header automatically. Required for ASCII input and for headerless machine code. |
| `-p`, `--port <port>` | Serial port name (auto-detected if omitted) |
| `-f`, `--format <format>` | Override detected input format: `ascii`, `binary` |
| `--start-address <hex>` | Load address for machine language programs (e.g. `38C5`) |
| `--run-address <hex>` | Auto-run address for machine language programs |
| `--add-utils` | Prepend serial utility sub-program (shortcuts at line 61000+) |
| `-v`, `--verbose` | Verbose logging |
| `-vv`, `--debug` | Debug logging |
| `-V`, `--version` | Print version and exit |
| `-h`, `--help` | Print help |

The file format is detected automatically from the file content. Use `--format` only if detection fails or to override.

When sending a binary file with a CE-158 or PC-1600 header, `--device` can be omitted: SharpDataExchange reads the device type from the header and configures the serial port accordingly.

---

## Pocket Computer Commands

### Sharp PC-1500 / PC-1500A

#### One-time serial port setup

The PC-1500 uses the CE-158X's USB port (U1). Before each session, redirect serial I/O to that port:

```
SETDEV U1,CI,CO
```

This command must be re-entered after each power cycle or `NEW`.

#### Receive a program from the PC (`put`)

Issue the load command on the PC-1500 **before** starting `put` on the PC:

| To receive... | Command |
|---|---|
| Tokenized binary (default) | `CLOAD` |
| ASCII BASIC | `CLOADa` |

SharpDataExchange sends binary by default for the fastest and most reliable transfer. When loading binary, the PC-1500 does not need to parse each line, which avoids timing problems.

#### Send a program to the PC (`get`)

Start `get` on the PC **first**, then issue the save command on the PC-1500:

```
CSAVE
```

or with a filename:

```
CSAVE"filename"
```

SharpDataExchange de-tokenizes the binary data and writes readable ASCII BASIC to the file. Binary is faster than ASCII transfer and is the recommended method.

> The ASCII variant (`CSAVEa`) also works and produces the same result on the PC, but is significantly slower and is not normally used.

#### Save/load the Reserve Area

```
CSAVEr"filename"    (binary, Reserve Area)
```

The Reserve Area is always saved as binary from the PC-1500. SharpDataExchange converts it to SDAR format by default.

To send a Reserve Area file back:

```
CLOADr
```

#### Serial utility shortcuts (optional)

If you use `--add-utils` when sending a program, three shortcut functions are added at line 61000:

| Key | Action |
|---|---|
| `Def-J` | Re-initialize the serial port (`SETDEV U1,CI,CO`) |
| `Def-S` | Save the program via serial |
| `Def-L` | Load a program via serial |

---

### Sharp PC-1600

#### Serial port setup (after each power-on or reset)

The PC-1600 serial port must be configured before first use. Enter these commands:

```
SETCOM "COM1:",9600,8,N,1,N,N
INIT "COM1:",4096
OUTSTAT "COM1:"
RCVSTAT "COM1:",24
```

For sending data **from** the PC-1600 to the PC, additionally enter:

```
SNDSTAT "COM1:",24
```

These settings configure the port at 9600 baud, 8 data bits, no parity, 1 stop bit, with RTS/CTS hardware flow control. The buffer is set to 4096 bytes.

#### Receive a program from the PC (`put`)

Issue the load command on the PC-1600 **before** starting `put` on the PC. The PC-1600 auto-detects whether the incoming data is ASCII or binary:

```
LOAD "COM1:"
```

To auto-run the program immediately after loading:

```
LOAD "COM1:",R
```

#### Send a program to the PC (`get`)

Start `get` on the PC **first**, then issue the save command on the PC-1600:

```
SAVE "COM1:"
```

SharpDataExchange de-tokenizes the binary data and writes readable ASCII BASIC to the file. Binary is faster than ASCII transfer and is the recommended method.

> The ASCII variant (`SAVE "COM1:",A`) also works and produces the same result on the PC, but is significantly slower and is not normally used.

#### Serial utility shortcuts (optional)

If you use `--add-utils` when sending a program, three shortcut functions are added at line 61000:

| Key | Action |
|---|---|
| `Def-J` | Re-initialize the serial port (repeats the SETCOM/INIT/OUTSTAT/RCVSTAT sequence) |
| `Def-S` | Save the program via `SAVE "COM1:"` |
| `Def-L` | Load a program via `LOAD "COM1:"` |

#### PC-1600 serial command reference

The following commands are available for advanced serial configuration on the PC-1600:

**`SETCOM`** — Configure baud rate and protocol:
```
SETCOM "COM1:",<baud>,<bits>,<parity>,<stop>,<xon>,<shift>
```
Example: `SETCOM "COM1:",9600,8,N,1,N,N`

**`INIT`** — Set receive buffer size (default after power-on is 40 bytes; increase to avoid overflow):
```
INIT "COM1:",<buffer-size>
```
Example: `INIT "COM1:",4096`

**`OUTSTAT`** — Configure RTS/DTR signals. Call without a parameter to enable dynamic flow control:
```
OUTSTAT "COM1:"
```

**`RCVSTAT`** — Set receive handshake and timeout:
```
RCVSTAT "COM1:",<protocol>[,<timeout>]
```
`24` enables CTS handshake. Timeout is in units of 0.5 seconds; 0 disables it.

**`SNDSTAT`** — Set send handshake and timeout:
```
SNDSTAT "COM1:",<protocol>[,<timeout>]
```
`24` enables CTS handshake. `28` disables all flow control.

**`SETDEV`** — Redirect `INPUT` and/or `LPRINT`/`LLIST` to the serial port:
```
SETDEV "COM1:"[,KI][,PO]
```
Call `SETDEV` without parameters to release the port.

**`PCONSOLE`** — Set line length and line ending for serial output:
```
PCONSOLE "COM1:",<line-length>,<eol>
```
EOL codes: `0`=CR, `1`=LF, `2`=CR/LF. Example: `PCONSOLE "COM1:",80,2`

### `terminal` — Experimental interactive mode

```
java -jar SharpDataExchange.jar terminal [options]
```

Starts a passive terminal session. All data received from the Pocket Computer is displayed on the console, and all keyboard input is sent back to the Pocket Computer.

This is useful for debugging serial communication or interacting with custom BASIC programs that use `PRINT#` and `INPUT#` for user interaction.

**To exit:** Press the **ESC** key twice in rapid succession.

| Option | Description |
|---|---|
| `-d`, `--device <device>` | Target device: `pc1500` (default), `pc1500a`, `pc1600`. Required to configure the serial port baud rate and handshaking. |
| `-p`, `--port <port>` | Serial port name (auto-detected if omitted) |
| `-v`, `--verbose` | Verbose logging |
| `-vv`, `--debug` | Debug logging |

---

## Data Formats

### ASCII BASIC

Standard Sharp BASIC source code, one line per line number:

```
10 FOR I=1 TO 10
20 PRINT I
30 NEXT I
```

Full keyword names are used (not abbreviations). This is the default output of `get` for all BASIC programs, whether they were sent from the Pocket Computer in binary or ASCII mode.

### Binary

Raw binary format as used by the Pocket Computer's cassette interface. The PC-1500 uses a 27-byte CE-158 header; the PC-1600 uses a 16-byte header.

Use `--format binary` with `get` to save the file in binary form. The header is included by default, making the file directly usable with `put` without re-tokenizing. Use `--skip-header` to omit it — but note that SharpDataExchange cannot identify or reload a headerless binary file, and will print a warning when this option is used.

### Reserve Area (SDAR) — PC-1500 only

A human-readable format for the PC-1500 Reserve Area. The PC-1600 does not support Reserve Area transfer via serial (it is only available via the tape interface).

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

**Layer label:** Free-form text chosen by the user; typically a compact string that identifies what the keys on that layer do, such as `ABS FOR SIN COS TAN ATN`. The label is not validated against the key contents — keeping them aligned is up to the user.

**Key content:** BASIC keywords are written in their full names (`PRINT`, `FOR`, `SIN(`, etc.) and are re-tokenized automatically when the file is loaded back. Empty keys are left blank after the colon. All six keys must be present in each layer, even if empty.

Lines beginning with `;` are comments and are ignored when reading the file back. Blank lines between sections are also ignored.

> **Size limit:** The total content across all layers must fit within the hardware Reserve Area. SharpDataExchange checks this when converting from SDAR to binary and reports an error if the limit is exceeded.

### Variables (SDAV) — PC-1500 only

A human-readable key-value format for variable data on the PC-1500. The PC-1600 does not support variable transfer via serial.

```
; SDAV:1.0 pc1500
; Count: 4
; Filename: MYAPP
A=3.14159265
B$="HELLO"
C=0
D$=""
```

Numeric variables are written in decimal. String variables are enclosed in double quotes. Internal double quotes are escaped as `\"`, and control characters as `\xHH`.

---

## Common Workflows

### Transfer a BASIC program to the PC-1500 and keep it editable

```
java -jar SharpDataExchange.jar get myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CSAVE` — SharpDataExchange de-tokenizes the binary and writes readable ASCII BASIC.

### Send a modified program back to the PC-1500

```
java -jar SharpDataExchange.jar put myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOAD` — the program is sent as tokenized binary for fastest transfer.

### Send as ASCII instead of binary (e.g. to diagnose a tokenization problem)

```
java -jar SharpDataExchange.jar put --format ascii myprogram.bas
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOADa`.

### Back up the Reserve Area from the PC-1500

On the PC-1500: `SETDEV U1,CI,CO` then `CSAVEr"MYAPP"`

```
java -jar SharpDataExchange.jar get reserve.sdar
```

The Reserve Area is saved in SDAR format — a structured, human-readable file showing the three layers, labels, and key definitions. BASIC keywords are de-tokenized to full names so the file can be read and edited directly.

### Restore the Reserve Area to the PC-1500

On the PC-1500: `SETDEV U1,CI,CO` then `CLOADr`

```
java -jar SharpDataExchange.jar put reserve.sdar
```

### Load a machine language program onto the PC-1500

If the binary file has no CE-158 header, provide the addresses:

```
java -jar SharpDataExchange.jar put --start-address 38C5 --run-address 38C5 program.bin
```

On the PC-1500: `SETDEV U1,CI,CO` then `CLOAD`.

### Specify the serial port manually

```
java -jar SharpDataExchange.jar get myprogram.bas --port /dev/cu.usbserial-A50285BI
```

---

## Troubleshooting

**The tool exits immediately without waiting for data (`get`)**
The serial port was not found. Check that the CE-158X or USB/UART adapter is connected. Use `--port` to specify the port explicitly.

**`ERROR 67` on the PC-1500 when loading ASCII**
A line in the program exceeds 80 characters. Load as binary instead (omit `a` from `CLOADa`).

**`ERROR 61` on the PC-1500 when loading binary**
The binary file lacks a CE-158 header. SharpDataExchange adds the header automatically when sending; this error should not occur with files produced by `get`. If sending a third-party binary file, ensure it either has a header or supply `--start-address`.

**Data corruption or incomplete transfer (PC-1500)**
The PC-1500 requires paced transmission. SharpDataExchange applies a 1 ms delay between bytes and a 300 ms pause after the header automatically. If problems persist, check the USB cable and CE-158X connection.

**Data loss when sending to PC-1600 on Apple Silicon Mac**
A macOS driver bug causes RTS/CTS flow control to malfunction on Apple Silicon. Run SharpDataExchange on a Raspberry Pi connected to the PC-1600, and use SSH from the Mac to operate it.

**The PC-1600 serial port stops responding**
Re-enter the full setup sequence:
```
SETCOM "COM1:",9600,8,N,1,N,N
INIT "COM1:",4096
OUTSTAT "COM1:"
RCVSTAT "COM1:",24
```
If `--add-utils` was used, `Def-J` runs this sequence automatically.
