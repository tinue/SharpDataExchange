//! `get`'s receive/accumulate/end-of-transfer logic, as a plain deadline check rather
//! than a background thread — Rust's synchronous poll-read loop doesn't need one.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::header;
use crate::serial::Transport;

/// Accumulates bytes and decides when a transfer is complete.
pub struct Receiver {
    buf: Vec<u8>,
    raw_mode: bool,
    expected_total: Option<usize>,
}

impl Receiver {
    pub fn new(raw_mode: bool) -> Self {
        Receiver { buf: Vec::new(), raw_mode, expected_total: None }
    }

    /// Feed newly-read bytes. Returns `true` once the transfer is known-complete by
    /// header length (never true in raw mode — that always relies on the watchdog).
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.buf.extend_from_slice(bytes);
        if self.raw_mode {
            return false;
        }
        if self.expected_total.is_none() {
            self.expected_total = header::expected_total_bytes(&self.buf);
        }
        matches!(self.expected_total, Some(total) if self.buf.len() >= total)
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Drive a [`Receiver`] against `transport` until the transfer is complete: either the
/// buffer reaches the header-derived expected length, or the idle-timeout watchdog
/// fires. The watchdog only starts once the first byte has arrived — `get` blocks
/// indefinitely waiting for the Pocket Computer to start sending, matching the
/// "start `get` first, then trigger the device-side save" workflow (§1).
pub fn receive_until_done<T: Transport>(
    transport: &mut T,
    idle_timeout: Duration,
    raw_mode: bool,
) -> Result<Vec<u8>> {
    const POLL: Duration = Duration::from_millis(50);
    let mut receiver = Receiver::new(raw_mode);
    let mut deadline: Option<Instant> = None;

    loop {
        let bytes = transport.read(POLL)?;
        if !bytes.is_empty() {
            if receiver.feed(&bytes) {
                return Ok(receiver.into_bytes());
            }
            deadline = Some(Instant::now() + idle_timeout);
        } else if let Some(d) = deadline {
            if Instant::now() >= d {
                return Ok(receiver.into_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::FakeSerial;

    #[test]
    fn feed_completes_once_header_length_reached() {
        let header = header::build(crate::registry::Device::Pc1500, Some("x"), 4);
        let mut r = Receiver::new(false);
        assert!(!r.feed(&header));
        assert!(!r.feed(&[1, 2]));
        assert!(r.feed(&[3, 4]));
        assert_eq!(r.buffer().len(), header.len() + 4);
    }

    #[test]
    fn raw_mode_never_completes_by_length_even_with_header_shaped_bytes() {
        let header = header::build(crate::registry::Device::Pc1500, Some("x"), 4);
        let mut r = Receiver::new(true);
        assert!(!r.feed(&header));
        assert!(!r.feed(&[1, 2, 3, 4]));
    }

    #[test]
    fn receive_until_done_stops_at_known_length() {
        let mut t = FakeSerial::new();
        let header = header::build(crate::registry::Device::Pc1500, Some("x"), 2);
        t.push_incoming(&header);
        t.push_incoming(&[0xAA, 0xBB]);
        let bytes = receive_until_done(&mut t, Duration::from_millis(10), false).unwrap();
        assert_eq!(bytes.len(), header.len() + 2);
    }

    #[test]
    fn receive_until_done_falls_back_to_watchdog_without_header() {
        let mut t = FakeSerial::new();
        t.push_incoming(&[1, 2, 3]);
        let bytes = receive_until_done(&mut t, Duration::from_millis(20), false).unwrap();
        assert_eq!(bytes, vec![1, 2, 3]);
    }

    #[test]
    fn receive_until_done_raw_mode_ignores_header_shaped_bytes_uses_watchdog() {
        let mut t = FakeSerial::new();
        let header = header::build(crate::registry::Device::Pc1500, Some("x"), 4);
        t.push_incoming(&header);
        let bytes = receive_until_done(&mut t, Duration::from_millis(20), true).unwrap();
        assert_eq!(bytes, header);
    }
}
