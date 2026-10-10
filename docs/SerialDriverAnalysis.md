# USB/UART driver analysis: FTDI TTL-232R on macOS and Linux

Analysis date: 2026-10-10. Purpose: input for changing `sde`'s PC-1600 serial defaults
and the recommended PC-1600 settings (see [Recommendations](#recommendations-for-sde)).

Adapter under test: **FTDI TTL-232R** (`FT232R`, VID/PID `0403:6001`, bcdDevice `0x0600`,
serial `FTD4RJIR`), the cable described in [HardwareNotes.md](HardwareNotes.md). Its
RX/TX/RTS/CTS inversion lives in the chip's EEPROM and is applied by the FT232R itself,
so it is invisible to every driver discussed here.

Method: the macOS findings come from disassembling the shipped binaries (driver
extension, DriverKit frameworks extracted from the DriverKit shared cache, and the
kernel-side kext carved out of the kernelcache), checked against Apple's open-source
`IOSerialFamily`. The Linux findings come from reading mainline `drivers/usb/serial/ftdi_sio.c`
(HEAD `3857c2fe5449`; identical in the relevant parts back to v6.8). **Nothing here has
been verified on hardware yet** — see [Open questions / tests](#open-questions--tests).

---

## Summary for sde

| Topic | macOS (Apple AppleUSBFTDI) | Linux (ftdi_sio) | Consequence for sde |
|---|---|---|---|
| RTS/CTS (`CRTSCTS`) | Works; handled in the FT232R | Works; handled in the FT232R | **The one mechanism that is reliable on both** |
| XON/XOFF | Buggy: `IXON` alone never reaches the chip; received 0x11/0x13 are deleted from the data | Works for `IXON`; `IXOFF`/`IXANY` ignored | Never use for binary transfers |
| DTR/DSR flow control | Not supported | Not reachable via termios | Not an option |
| Line errors (parity/framing/overrun/break) | Per USB packet (≤62 bytes), not per byte; some dropped | Per packet; better accounting | Errors are effectively invisible to sde: validate by header length |
| 9600 baud | Exact (divisor 312.5) | Exact | Keep |
| Max / min baud | 3,000,000 / ~183 | 3,000,000 / no lower check | Irrelevant at 9600 |
| Latency timer | Chip default 16 ms | 16 ms (sysfs `latency_timer`) | Fine at 9600; don't touch |

**FTDI's own macOS VCP driver (1.6.0) is worse than Apple's built-in one** for this
use: it hardcodes the XON/XOFF characters to 0x04/0x05 and ignores the line-status
byte entirely. Users should **not** install it; the built-in driver is what binds by
default.

---

## macOS: Apple's built-in driver

Driver stack (from `ioreg`), all Apple, no FTDI software installed:

```
/dev/cu.usbserial-FTD4RJIR
  IOSerialBSDClient                 (IOSerialFamily, termios → PD_* events)
    AppleUSBFTDI : IOUserUSBSerial : IOUserSerial
      /System/Library/DriverExtensions/com.apple.DriverKit-AppleUSBFTDI.dext
      USBSerialDriverKit / SerialDriverKit   (DriverKit shared cache)
      com.apple.driver.driverkit.serial      (kernel half, in the kernelcache)
```

### How termios reaches the chip

`IOSerialBSDClient::iossparam` maps termios to `PD_RS232_A_*` bits (values equal the
`TIOCM_*` constants):

| termios | bit | value |
|---|---|---|
| `CDTR_IFLOW` | `PD_RS232_A_DTR` | 0x02 |
| `CRTS_IFLOW` | `PD_RS232_A_RFR` | 0x04 |
| `IXON` | `PD_RS232_A_TXO` | 0x08 |
| `IXOFF` | `PD_RS232_A_RXO` | 0x10 |
| `CCTS_OFLOW` | `PD_RS232_A_CTS` | 0x20 |
| `IXANY` | `PD_RS232_A_XANY` | 0x400 |
| `CDSR_OFLOW` | — (never mapped) | |

`CRTSCTS` = `CCTS_OFLOW|CRTS_IFLOW` → 0x24. The BSD client sends `PD_E_FLOW_CONTROL`
**first**, then `PD_RS232_E_XON_BYTE` / `XOFF_BYTE`. The kernel kext stores the XON/XOFF
characters only (initial value 0x11/0x13) and, on `PD_E_FLOW_CONTROL`, calls
`HwProgramFlowControl(mask, storedXon, storedXoff)` with the raw mask.

### `AppleUSBFTDI::HwProgramFlowControl(arg, xon, xoff)`

- Re-programs the chip **only if `(old ^ arg) & 0x16`** (DTR | RFR | RXO) is non-zero.
- Mode: `arg & 0x24` (RTS/CTS) → `SIO_RTS_CTS_HS`; else `arg & 0x18` (TXO/RXO) →
  `SIO_XON_XOFF_HS` with `wValue = xon | xoff<<8`; else off. RTS/CTS wins over XON/XOFF.
- No DTR/DSR mode is ever selected. IXANY is not stored.
- Control request `0x40 / 2` (SET_FLOW_CTRL), 5000 ms timeout; on success stores `arg & 0x17e`.

Consequences:
1. **`IXON` alone (from no flow control) is never programmed** — only TXO changes, which
   is outside the change mask. Same for `CCTS_OFLOW` alone. `CRTSCTS` and `IXON|IXOFF`
   (what `serialport`'s `FlowControl::Hardware` / `Software` set) do work.
2. **Custom XON/XOFF characters arrive one call late** (ordering above), and changing
   only `VSTART`/`VSTOP` is never sent. The defaults 0x11/0x13 are unaffected.

### `AppleUSBFTDI::handleRxPacket` (once per 64-byte bulk-IN packet)

- Byte 0 (modem status, bits 4–7 = CTS/DSR/RI/DCD) → `SetModemStatus` only on change.
- Byte 1 (line status): only if the packet carries data (length ≥ 3) and `& 0x1e` →
  `RxError(overrun=0x02, break=0x10, framing=0x08, parity=0x04)`. Status-only packets
  are ignored, so an overrun/break reported without data is lost; bit 7 (RX FIFO error)
  is ignored. The error applies to the whole packet.
- **If TXO or RXO is set, every byte equal to the XON or XOFF character is deleted from
  the received data** — also with `IXOFF` only. Binary data containing 0x11/0x13 is
  corrupted.

### Generic USB-serial layer (`IOUserUSBSerial`)

- RX: one bulk-IN transfer of `wMaxPacketSize` (64) outstanding at a time; resubmitted
  only while the ring has ≥ 64 bytes free, so a full host buffer back-pressures the chip
  (which then drops RTS under `CRTSCTS`). A stall is cleared and retried once; any other
  error stops reception until the port is reopened.
- TX: the whole contiguous ring is sent in one bulk-OUT transfer (plus optional ZLP
  quirk). On a transfer error the consumer index is not advanced and nothing is
  resubmitted; there is no `ClearStall` on the TX pipe. Output stalls, or resends the same
  bytes on the next write.

### Baud rate, latency, other

- `findBaudDiv`: 3 MHz reference (12 MHz for H-series). `baud-1 ≥ 3,000,000` →
  `kIOReturnBadArgument`; divisor > 0x3ffe (below ~183 baud) → error. Rates between 1.5M
  and 3M snap to 1.5M / 2M / 3M. No tolerance check; the actual rate is not reported back.
- Chip detection by bcdDevice: `0x600` → FT232R (type 3, correct 17-bit divisor).
  bcdDevice `0x1000` (FT-X series, PID 0x6015 is in the personalities) is **not** in the
  table → "Invalid chip ID … assuming F1232AM" → FT232AM baud restrictions. Not relevant
  to the TTL-232R.
- `HwProgramLatencyTimer(latency)` sends `latency & 0xffff` unconverted to
  `SET_LATENCY_TIMER` (ms, 1–255). The kernel feeds it the raw `IOSSDATALAT` value, which
  Apple documents in microseconds → **don't use `IOSSDATALAT`**. The chip default (16 ms)
  is fine.
- `HwProgramMCR`: `wValue = 0x0300 | DTR | RTS<<1` (always writes both lines).
  `HwGetModemStatus` reads only the modem byte.

## macOS: FTDI VCP driver 1.6.0 (not installed, for comparison)

`FTDIUSBSerialVCPDextInstaller` 1.6.0 (2026-03-06; listed for macOS 11/12, 15, 26 — not 27),
DriverKit dext `com.ftdi.vcp.dext` built from FTDI's "NullDriver" sample.

- `HwProgramFlowControl`: ORs modes together (XON/XOFF if TXO|RXO, RTS/CTS if RFR|CTS —
  and, oddly, CAR — DTR/DSR if DTR|DSR). **`wValue` is hardcoded to `0x0504`** (XON=0x04,
  XOFF=0x05); the `xon`/`xoff` arguments are ignored. Verified in the disassembly.
- `handleRxPacket` strips the status bytes but **never reads the line-status byte**:
  parity/framing/overrun/break are dropped silently. It also logs payload via `os_log`.
- Advantages: DTR/DSR handshake, baud aliasing and latency via `ConfigData` (the
  "FTDI R Chip" personality has none → latency 16 ms).
- FTDI's AN_134 itself says Apple's driver is sufficient unless those extras are needed;
  its instructions to disable Apple's driver are kext-era and don't apply to the dext.

## Linux: ftdi_sio (mainline, ≥ 6.8)

- **Flow control** (`ftdi_set_termios`, runs on every termios change):
  `if (C_CRTSCTS) RTS_CTS_HS; else if (I_IXON) XON_XOFF_HS (value = STOP<<8|START); else off`.
  `IXOFF`/`IXANY` ignored; DTR/DSR not reachable (`FTDI_SIO_DTR_DSR_HS` defined, unused).
  Kernels before ~4.18 used `IXOFF` instead of `IXON` (commit `5ada98427f12`).
- n_tty also consumes received START/STOP characters when `IXON` is set, and
  usb-serial has no `.stop`/`.start`/`.send_xchar`: `tcflow()` has no real effect on
  output, and a manually sent XON/XOFF queues behind buffered data.
- `TIOCMGET` reports the *requested* RTS, not the chip's automatic RTS under CRTSCTS.
  `TIOCMIWAIT` / `TIOCGICOUNT` supported.
- **Errors** (`ftdi_process_packet`): processed only for packets with payload; PE/FE
  flag every byte of the packet; overrun inserts one `TTY_OVERRUN` NUL; break only on a
  trailing NUL.
- **Limits:** ≤ 3,000,000 baud (higher silently → 9600, readable back via termios); no
  lower bound (below ~183 baud the divisor overflows). Write kfifo `PAGE_SIZE` (4 KiB),
  2×512-byte read URBs, 2×256-byte write URBs. sysfs `latency_timer` (default 16 ms);
  `low_latency` forces 1 ms. CS6 rejected; CMSPAR supported.

## FT232R chip facts (FTDI DS_FT232R, AN232B-04/-05, AN_120, D2XX guide)

- Baud = 3,000,000 / (n + k/8), n = 2…16384; divisor 0 = 3 Mbaud, 1 = 2 Mbaud.
  9600 → 312.5, exact.
- 128-byte RX buffer (USB→UART), 256-byte TX buffer (UART→USB). Full-speed bulk packet:
  2 status bytes + 62 data bytes.
- Data is sent to the host when the buffer is full, a modem line changes, the event
  character arrives, or the latency timer (default 16 ms) expires.
- RTS/CTS, DTR/DSR, XON/XOFF are handled in hardware; one mode at a time ("will transmit
  if CTS is active and will drop RTS if it cannot receive any more").
- Not documented by FTDI: bytes still sent after CTS drops, the RTS deassert threshold,
  any FT232R flow-control errata.

---

## Recommendations for sde

Current state (`src/pocket_device.rs`, `src/serial.rs`, `src/sender.rs`): PC-1600 is
9600 8N1; `--flowcontrol` (opt-in) selects `serialport::FlowControl::Hardware`
(`CRTSCTS`), otherwise sends are paced (300 ms header pause, 1 ms per byte, 500 ms tail).
PC-1600 side per README: `SETCOM "COM1:",9600,8,N,1,N,N` and `RCVSTAT`/`SNDSTAT` `28`.

1. **Make RTS/CTS the default for `--device pc1600`** (keep an opt-out such as
   `--no-flowcontrol`). `CRTSCTS` is the only flow-control path that is implemented
   correctly by both the Apple and the Linux driver, it is handled inside the FT232R
   (no host latency), and it lets `put` drop the per-byte pacing
   (`is_paced_send` → false). Matching PC-1600 setting: `RCVSTAT`/`SNDSTAT "COM1:",28,0`
   (correction: `RCVSTAT` is only a filter, so host → PC-1600 flow control is the
   PC-1600's RTS under `OUTSTAT "COM1:"`, and `28` is right; `,0` disables the timeout
   that surfaces as `ERROR 142` on `LOAD`). **Implemented in 0.3.5** (`--no-flowcontrol`).
   Keep `pc1600emul` and the PC-1500 family unchanged (no handshake lines).
2. **Never offer XON/XOFF** for binary transfers (`SETCOM … ,N,` stays). On macOS it
   deletes 0x11/0x13 from received data and `IXON`-only is never programmed; with FTDI's
   VCP driver the characters are wrong (0x04/0x05).
3. **Keep 9600 8N1.** Exact on the FT232R; driver limits (183…3 M baud) are far away.
4. **Don't change the latency timer or call `IOSSDATALAT`** (unit bug on macOS; 16 ms is
   fine at ~960 B/s).
5. **Don't rely on serial error flags** — neither driver delivers per-byte errors to a
   raw-mode reader. Validate transfers by the length in the header (and by the idle
   timeout for headerless data), and report short/long transfers clearly. (Today the
   16-bit byte sum is only printed for `get --raw`, not checked.)
6. **Guard against a stalled transmit** when RTS/CTS is on: if the PC-1600 is not ready
   (or not connected), CTS stays inactive and writes block. Use bounded write/drain
   timeouts (`DRAIN_TIMEOUT` exists) and a clear error message ("PC-1600 not ready —
   check the RTS/CTS wires"). Optionally read CTS before sending
   (`serialport::SerialPort::read_clear_to_send`) and warn. **Implemented in 0.3.5**
   (`serial::STALL_TIMEOUT`; no CTS pre-check).
7. **Document for users:** use the built-in macOS driver; do not install FTDI's VCP
   driver. On Linux no setup is needed (`ftdi_sio`, kernel ≥ 4.18).

## Open questions / tests

Need the PC-1600 or a TX↔RX / RTS↔CTS loopback on the cable:

- Confirm `put`/`get` with the RTS/CTS default + `RCVSTAT`/`SNDSTAT "COM1:",28,0` on macOS and Linux,
  including a file larger than the PC-1600's `INIT` buffer, and a binary file containing
  0x11/0x13.
- Reproduce the README's `ERROR 142` case (PC-1600 at `24`, sde without flow control) and
  check which RTS level the (inverted) adapter presents when sde opens the port without
  `CRTSCTS`; decide whether sde should assert RTS explicitly in that mode.
- Loopback test of the macOS `IXON`-only bug and the 0x11/0x13 stripping (to report to
  Apple via Feedback Assistant).
- Behaviour when CTS never asserts (timeouts, error message).

## Sources

- Apple IOSerialFamily: https://github.com/apple-oss-distributions/IOSerialFamily
  (`IORS232SerialStreamSync.h`, `IOSerialBSDClient.cpp`)
- DriverKit SDK headers: `SerialDriverKit/IOUserSerial.h`, `USBSerialDriverKit/IOUserUSBSerial.h`
- Linux: https://github.com/torvalds/linux/blob/master/drivers/usb/serial/ftdi_sio.c,
  `ftdi_sio.h`, `generic.c`
- FTDI VCP drivers: https://ftdichip.com/drivers/vcp-drivers/ (1.6.0 obtained via
  Wayback Machine; ftdichip.com blocks non-browser clients)
- FTDI DS_FT232R: https://ftdichip.com/wp-content/uploads/2020/08/DS_FT232R.pdf
- FTDI AN232B-04 (latency/handshaking), AN232B-05 (baud rates), AN_120 (aliasing),
  AN_134 (Mac driver installation), D2XX Programmer's Guide FT_000071
