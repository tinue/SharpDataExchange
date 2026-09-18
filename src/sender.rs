//! `put`'s paced/unpaced transmit logic.

use std::time::Duration;

use anyhow::Result;

use crate::pocket_device::PocketDevice;
use crate::serial::Transport;

const HEADER_PAUSE: Duration = Duration::from_millis(300);
const BYTE_DELAY: Duration = Duration::from_millis(1);
const TAIL_PAUSE: Duration = Duration::from_millis(500);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(2000);
const SEGMENT_PAUSE: Duration = Duration::from_millis(300);
const SEGMENT_MARKER: [u8; 3] = [0xFF, 0x00, 0x00];

/// Send a fully-formed data block (`header_len` leading bytes are the header, if any;
/// `header_len == 0` for a headerless send, e.g. `--raw`). Paced devices (PC-1500
/// family, PC-1600 emulator) send the header at full speed, pause, then send the
/// payload byte-by-byte with a 1ms delay; a real PC-1600 (hardware RTS/CTS) sends
/// everything at full speed and relies on the handshake to throttle it.
pub fn send_data<T: Transport>(
    transport: &mut T,
    device: PocketDevice,
    header_len: usize,
    data: &[u8],
) -> Result<()> {
    if device.is_paced_send() {
        let header = &data[..header_len.min(data.len())];
        let payload = &data[header_len.min(data.len())..];

        transport.write_all(header)?;
        transport.sleep(HEADER_PAUSE);

        write_paced_payload(transport, device, payload)?;

        transport.drain(DRAIN_TIMEOUT);
        transport.sleep(TAIL_PAUSE);
    } else {
        transport.write_all(data)?;
        transport.drain(DRAIN_TIMEOUT);
    }
    Ok(())
}

/// Send a paced payload, pausing an extra [`SEGMENT_PAUSE`] after every
/// `0xFF 0x00 0x00` segment-boundary marker found in it. Only a PC-1600 tokenized-BASIC
/// payload can contain this marker; elsewhere it is sent as an ordinary run of paced
/// bytes.
fn write_paced_payload<T: Transport>(
    transport: &mut T,
    device: PocketDevice,
    payload: &[u8],
) -> Result<()> {
    if !device.is_pc1600_family() {
        return write_byte_by_byte(transport, payload);
    }
    let mut start = 0usize;
    let mut i = 0usize;
    while i + SEGMENT_MARKER.len() <= payload.len() {
        if payload[i..i + SEGMENT_MARKER.len()] == SEGMENT_MARKER {
            let end = i + SEGMENT_MARKER.len();
            write_byte_by_byte(transport, &payload[start..end])?;
            transport.sleep(SEGMENT_PAUSE);
            start = end;
            i = end;
        } else {
            i += 1;
        }
    }
    write_byte_by_byte(transport, &payload[start..])
}

fn write_byte_by_byte<T: Transport>(transport: &mut T, bytes: &[u8]) -> Result<()> {
    for &b in bytes {
        transport.write_all(&[b])?;
        transport.sleep(BYTE_DELAY);
    }
    Ok(())
}

/// Send an ASCII BASIC listing as discrete lines (only reached when `--format ascii`
/// forces line-by-line transfer instead of tokenized binary). Each line is CP437 bytes
/// terminated by CR (PC-1500 family) or CR+LF (PC-1600 family); paced devices pause
/// 500ms after each line. A final EOF marker follows: `0x0D` (PC-1500 family) or
/// `0x1A` (PC-1600 family).
pub fn send_ascii_lines<T: Transport>(
    transport: &mut T,
    device: PocketDevice,
    lines: &[String],
) -> Result<()> {
    for line in lines {
        let mut bytes = crate::cp437::encode_lossy(line);
        bytes.push(0x0D);
        if device.is_pc1600_family() {
            bytes.push(0x0A);
        }
        transport.write_all(&bytes)?;
        if device.is_paced_send() {
            transport.sleep(TAIL_PAUSE);
        }
    }
    let eof: u8 = if device.is_pc1500_family() { 0x0D } else { 0x1A };
    transport.write_all(&[eof])?;
    transport.drain(DRAIN_TIMEOUT);
    transport.sleep(TAIL_PAUSE);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::FakeSerial;

    #[test]
    fn plain_pc1500_send_paces_header_then_payload() {
        let mut t = FakeSerial::new();
        let data = [0xAAu8, 0xBB, 0x01, 0x02, 0x03];
        send_data(&mut t, PocketDevice::Pc1500, 2, &data).unwrap();
        assert_eq!(t.written, data);
        // header write, then 1 sleep per payload byte + header pause + tail pause.
        assert_eq!(t.sleeps[0], HEADER_PAUSE);
        assert_eq!(&t.sleeps[1..4], &[BYTE_DELAY, BYTE_DELAY, BYTE_DELAY]);
        assert_eq!(t.sleeps[4], TAIL_PAUSE);
        assert_eq!(t.drains, 1);
    }

    #[test]
    fn real_pc1600_send_is_unpaced_no_sleeps() {
        let mut t = FakeSerial::new();
        let data = [1u8, 2, 3, 4];
        send_data(&mut t, PocketDevice::Pc1600, 0, &data).unwrap();
        assert_eq!(t.written, data);
        assert!(t.sleeps.is_empty());
        assert_eq!(t.drains, 1);
    }

    #[test]
    fn pc1600emul_pauses_extra_after_segment_marker() {
        let mut t = FakeSerial::new();
        // header_len=0 for simplicity; payload has one marker at index 2.
        let payload = [0x01u8, 0x02, 0xFF, 0x00, 0x00, 0x03];
        send_data(&mut t, PocketDevice::Pc1600Emul, 0, &payload).unwrap();
        assert_eq!(t.written, payload);
        // header pause, then per-byte sleeps for [01,02,FF,00,00] with an extra
        // segment pause right after the marker completes, then one more byte-sleep.
        assert_eq!(
            t.sleeps,
            vec![
                HEADER_PAUSE,
                BYTE_DELAY, // 0x01
                BYTE_DELAY, // 0x02
                BYTE_DELAY, // 0xFF
                BYTE_DELAY, // 0x00
                BYTE_DELAY, // 0x00 (marker complete)
                SEGMENT_PAUSE,
                BYTE_DELAY, // 0x03
                TAIL_PAUSE,
            ]
        );
    }

    #[test]
    fn ascii_lines_pace_and_terminate_per_family() {
        let mut t = FakeSerial::new();
        let lines = vec!["10 PRINT".to_string(), "20 END".to_string()];
        send_ascii_lines(&mut t, PocketDevice::Pc1500, &lines).unwrap();
        assert_eq!(t.written, b"10 PRINT\r20 END\r\r");
        assert_eq!(t.sleeps, vec![TAIL_PAUSE, TAIL_PAUSE, TAIL_PAUSE]);

        let mut t = FakeSerial::new();
        send_ascii_lines(&mut t, PocketDevice::Pc1600, &lines).unwrap();
        assert_eq!(t.written, b"10 PRINT\r\n20 END\r\n\x1A");
        // real PC-1600 is not paced: no per-line sleeps, but the trailing tail pause
        // after the EOF marker still applies (drain/tail-pause happen unconditionally).
        assert_eq!(t.sleeps, vec![TAIL_PAUSE]);
    }
}
