//! Serial transport abstraction for `get`/`put`.
//!
//! [`Transport`] is implemented by [`RealSerial`] (backed by the `serialport` crate),
//! [`PtySerial`] (a plain file descriptor, for `pc1600emul`'s pseudo-terminal — see its
//! doc comment for why), and [`FakeSerial`] (an in-memory test double). `sleep` is part
//! of the trait specifically so the paced-send/watchdog-receive logic in
//! [`crate::sender`] / [`crate::receiver`] can be unit-tested without a real,
//! multi-second-long wait. [`RealTransport`] picks between [`RealSerial`] and
//! [`PtySerial`] based on the target device and is what `get_cmd`/`put_cmd` actually use.

use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::pocket_device::PocketDevice;

/// A byte-oriented transport to/from the Pocket Computer.
pub trait Transport {
    /// Write all of `bytes` at full transport speed (no per-byte pacing here — that's
    /// the caller's job via repeated small writes + `sleep`).
    fn write_all(&mut self, bytes: &[u8]) -> Result<()>;

    /// Read whatever is available within `poll_timeout`. Returns an empty `Vec` (not an
    /// error) when nothing arrived within the timeout, so callers can check their own
    /// watchdog deadline between polls.
    fn read(&mut self, poll_timeout: Duration) -> Result<Vec<u8>>;

    /// Block (up to `timeout`) until the outgoing buffer is empty.
    fn drain(&mut self, timeout: Duration);

    /// Pause. Real transports actually sleep; test doubles just record the request.
    fn sleep(&mut self, d: Duration);
}

/// Real serial port, backed by the `serialport` crate.
pub struct RealSerial {
    port: Box<dyn serialport::SerialPort>,
}

impl RealSerial {
    /// Open `port_name` configured for `device` (baud rate + flow control per the
    /// device table in requirements §2): 8 data bits, no parity, 1 stop bit, hardware
    /// flow control iff `device.has_hardware_flow_control()`. On Unix, no exclusive
    /// lock (so a peer that already holds the port open doesn't make the open fail);
    /// Windows has no non-exclusive open mode for `serialport`, so this is skipped
    /// there and every open is exclusive.
    pub fn open(port_name: &str, device: PocketDevice) -> Result<RealSerial> {
        let flow = if device.has_hardware_flow_control() {
            serialport::FlowControl::Hardware
        } else {
            serialport::FlowControl::None
        };
        let builder = serialport::new(port_name, device.baud_rate())
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(flow)
            .timeout(Duration::from_millis(50));
        // `exclusive(false)`: a real USB/serial adapter is never opened by anyone
        // else, so dropping the exclusive lock is safe -- and needed so a peer that
        // already holds the port open (e.g. one end of a pty pair) doesn't make the
        // open fail. `SerialPortBuilder::exclusive` doesn't exist on Windows, which
        // has no non-exclusive COM port mode to begin with.
        #[cfg(not(windows))]
        let builder = builder.exclusive(false);
        let port = builder
            .open()
            .with_context(|| format!("could not open serial port {port_name}"))?;
        Ok(RealSerial { port })
    }

    /// Enumerate candidate ports for auto-detection: `cu.usb*` on macOS, `ttyACM*` /
    /// `ttyUSB*` on Linux. Succeeds only when exactly one candidate matches (per
    /// requirements §2); `pc1600emul` never calls this (its pseudo-terminal is never
    /// enumerated) and must be resolved via `--port` or the config-file default instead.
    ///
    /// Matches against the final path component (`serialport::available_ports()`
    /// returns a full device path on macOS/Linux, e.g. `/dev/cu.usbserial-XXXX`, not
    /// the bare device name).
    pub fn autodetect() -> Result<String> {
        let ports = serialport::available_ports()
            .context("could not list serial ports")?;
        let candidates: Vec<String> = ports
            .into_iter()
            .map(|p| p.port_name)
            .filter(|name| {
                let basename = std::path::Path::new(name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(name);
                if cfg!(target_os = "macos") {
                    basename.starts_with("cu.usb")
                } else if cfg!(target_os = "linux") {
                    basename.starts_with("ttyACM") || basename.starts_with("ttyUSB")
                } else {
                    false
                }
            })
            .collect();
        match candidates.len() {
            1 => Ok(candidates[0].clone()),
            0 => bail!("no serial port found; specify --port explicitly"),
            n => bail!(
                "{n} candidate serial ports found ({}); specify --port explicitly",
                candidates.join(", ")
            ),
        }
    }
}

impl Transport for RealSerial {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        self.port.write_all(bytes).context("serial write failed")
    }

    fn read(&mut self, poll_timeout: Duration) -> Result<Vec<u8>> {
        use std::io::Read;
        let _ = self.port.set_timeout(poll_timeout);
        let mut buf = [0u8; 4096];
        match self.port.read(&mut buf) {
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(Vec::new()),
            Err(e) => Err(e).context("serial read failed"),
        }
    }

    fn drain(&mut self, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match self.port.bytes_to_write() {
                Ok(0) => return,
                Ok(_) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return,
            }
        }
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// `pc1600emul`'s pseudo-terminal, opened as a plain file descriptor with no OS-level
/// serial configuration at all.
///
/// The `serialport` crate cannot be used here: on macOS its `TTYPort::open` always
/// applies the requested baud rate via the `IOSSIOSPEED` ioctl (see
/// `serialport-4.10.1/src/posix/termios.rs`'s own comment: "attempting to set the baud
/// rate on a pseudo terminal via this ioctl call will fail with the `ENOTTY` error").
/// Real hardware supports that ioctl; a BSD pseudo-terminal never does, so opening a
/// `pc1600emul` pty through `RealSerial` always fails with "Not a typewriter"
/// (confirmed against a live Calc-U-1600 emulator's pty on macOS 26 arm64).
///
/// This isn't a loss of functionality: baud/parity/flow-control at the OS level are
/// meaningless for a local pty anyway — the emulator paces bytes on its own side
/// (Calc-U-1600's `TC8576F::tick()`), and our own `pc1600emul` send path already paces
/// every byte itself (`sender::send_data`'s paced branch). A plain blocking
/// read/write on the pty fd is exactly what's needed; a blocked write under the
/// emulator's RX back-pressure is the intended stand-in for a real RTS drop.
#[cfg(unix)]
pub struct PtySerial {
    file: std::fs::File,
}

#[cfg(unix)]
impl PtySerial {
    pub fn open(path: &str) -> Result<PtySerial> {
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("could not open pty {path}"))?;

        // Put the fd into raw mode -- everything the `serialport` crate's own
        // `cfmakeraw()` call does (see `serialport-4.10.1/src/posix/tty.rs`), except we
        // deliberately never touch the baud rate (that's the `IOSSIOSPEED` ioctl this
        // struct exists to avoid; see its doc comment). Skipping this step was a real
        // bug: a freshly-opened pty defaults to cooked/canonical line-discipline
        // processing (echo, ICRNL/ONLCR CR<->LF translation, IXON/IXOFF), which
        // corrupts a binary tokenized-BASIC stream -- every `0x0D` line terminator is
        // exactly the kind of byte ONLCR/ICRNL mangles. Confirmed as the actual cause
        // of deterministic (not occasional) corruption seen only via this pty path,
        // never via `RealSerial`'s real-hardware path (which already calls
        // `cfmakeraw()`).
        let fd = file.as_raw_fd();
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
            bail!("tcgetattr failed on pty {path}: {}", std::io::Error::last_os_error());
        }
        termios.c_cflag |= libc::CREAD | libc::CLOCAL;
        unsafe { libc::cfmakeraw(&mut termios) };
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
            bail!("tcsetattr failed on pty {path}: {}", std::io::Error::last_os_error());
        }

        Ok(PtySerial { file })
    }
}

#[cfg(unix)]
impl Transport for PtySerial {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        self.file.write_all(bytes).context("pty write failed")
    }

    fn read(&mut self, poll_timeout: Duration) -> Result<Vec<u8>> {
        use std::os::unix::io::AsRawFd;
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = poll_timeout.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ready <= 0 {
            return Ok(Vec::new());
        }
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let n = self.file.read(&mut buf).context("pty read failed")?;
        Ok(buf[..n].to_vec())
    }

    fn drain(&mut self, _timeout: Duration) {
        // No introspectable output queue for a plain fd (unlike `serialport`'s
        // `bytes_to_write`); a short pause is enough since local pty writes are
        // delivered to the line discipline essentially immediately.
        std::thread::sleep(Duration::from_millis(20));
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Dispatches to [`PtySerial`] for `pc1600emul` (see its doc comment for why) and
/// [`RealSerial`] for everything else. This is what `get_cmd`/`put_cmd` actually open.
pub enum RealTransport {
    Serial(RealSerial),
    #[cfg(unix)]
    Pty(PtySerial),
}

impl RealTransport {
    pub fn open(port_name: &str, device: PocketDevice) -> Result<RealTransport> {
        #[cfg(unix)]
        if device.is_emulator() {
            return Ok(RealTransport::Pty(PtySerial::open(port_name)?));
        }
        let _ = device.is_emulator(); // silence unused-on-non-unix warnings
        Ok(RealTransport::Serial(RealSerial::open(port_name, device)?))
    }
}

impl Transport for RealTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            RealTransport::Serial(s) => s.write_all(bytes),
            #[cfg(unix)]
            RealTransport::Pty(p) => p.write_all(bytes),
        }
    }

    fn read(&mut self, poll_timeout: Duration) -> Result<Vec<u8>> {
        match self {
            RealTransport::Serial(s) => s.read(poll_timeout),
            #[cfg(unix)]
            RealTransport::Pty(p) => p.read(poll_timeout),
        }
    }

    fn drain(&mut self, timeout: Duration) {
        match self {
            RealTransport::Serial(s) => s.drain(timeout),
            #[cfg(unix)]
            RealTransport::Pty(p) => p.drain(timeout),
        }
    }

    fn sleep(&mut self, d: Duration) {
        match self {
            RealTransport::Serial(s) => s.sleep(d),
            #[cfg(unix)]
            RealTransport::Pty(p) => p.sleep(d),
        }
    }
}

/// Resolve the port to use, per requirements §2/§6: explicit `--port` wins; for
/// `pc1600emul`, the port is `<configured-or-default directory>/pc1600emul.port` (the
/// socket filename is always fixed — the config key only sets which directory it lives
/// in); otherwise auto-detect.
pub fn resolve_port(
    device: PocketDevice,
    explicit: Option<&str>,
    config: &crate::config::Config,
) -> Result<String> {
    if let Some(p) = explicit {
        return Ok(p.to_string());
    }
    if device.is_emulator() {
        let dir = config
            .get(crate::config::KEY_PC1600EMUL_PORT)
            .unwrap_or(crate::config::DEFAULT_PC1600EMUL_DIR);
        // Always joined with `/`, never `std::path::Path` (whose `.join()` uses `\`
        // on Windows): this is a Unix pty path string, consumed only by the
        // `#[cfg(unix)]` PtySerial -- never a path interpreted by the build host's
        // own filesystem conventions.
        let dir = dir.trim_end_matches('/');
        return Ok(format!("{dir}/{}", crate::config::PC1600EMUL_SOCKET_FILENAME));
    }
    RealSerial::autodetect()
}

/// In-memory test double. Records every write and every requested sleep (in order) so
/// pacing logic can be asserted exactly, without actually waiting.
#[derive(Default)]
pub struct FakeSerial {
    pub written: Vec<u8>,
    /// Byte length of each individual `write_all` call, in order (lets tests assert the
    /// header/payload split without re-deriving it from `written` alone).
    pub write_calls: Vec<usize>,
    pub sleeps: Vec<Duration>,
    pub drains: usize,
    pub to_read: std::collections::VecDeque<Vec<u8>>,
}

impl FakeSerial {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a chunk to be returned by the next `read()` call.
    pub fn push_incoming(&mut self, bytes: &[u8]) {
        self.to_read.push_back(bytes.to_vec());
    }
}

impl Transport for FakeSerial {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.written.extend_from_slice(bytes);
        self.write_calls.push(bytes.len());
        Ok(())
    }

    fn read(&mut self, _poll_timeout: Duration) -> Result<Vec<u8>> {
        Ok(self.to_read.pop_front().unwrap_or_default())
    }

    fn drain(&mut self, _timeout: Duration) {
        self.drains += 1;
    }

    fn sleep(&mut self, d: Duration) {
        self.sleeps.push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_serial_records_writes_and_sleeps() {
        let mut t = FakeSerial::new();
        t.write_all(&[1, 2, 3]).unwrap();
        t.sleep(Duration::from_millis(300));
        t.write_all(&[4]).unwrap();
        t.drain(Duration::from_millis(2000));
        assert_eq!(t.written, vec![1, 2, 3, 4]);
        assert_eq!(t.write_calls, vec![3, 1]);
        assert_eq!(t.sleeps, vec![Duration::from_millis(300)]);
        assert_eq!(t.drains, 1);
    }

    #[test]
    fn fake_serial_read_returns_queued_chunks_then_empty() {
        let mut t = FakeSerial::new();
        t.push_incoming(&[1, 2]);
        t.push_incoming(&[3]);
        assert_eq!(t.read(Duration::from_millis(50)).unwrap(), vec![1, 2]);
        assert_eq!(t.read(Duration::from_millis(50)).unwrap(), vec![3]);
        assert_eq!(t.read(Duration::from_millis(50)).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn resolve_port_prefers_explicit() {
        let config = crate::config::Config::from_entries_for_test(Default::default());
        let port = resolve_port(PocketDevice::Pc1600Emul, Some("/dev/ttys006"), &config).unwrap();
        assert_eq!(port, "/dev/ttys006");
    }

    #[test]
    fn resolve_port_emulator_joins_configured_directory_with_fixed_filename() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(crate::config::KEY_PC1600EMUL_PORT.to_string(), "/tmp/emul".to_string());
        let config = crate::config::Config::from_entries_for_test(entries);
        let port = resolve_port(PocketDevice::Pc1600Emul, None, &config).unwrap();
        assert_eq!(port, "/tmp/emul/calcu1600.serial");
    }

    #[test]
    fn resolve_port_emulator_defaults_to_tmp_without_config() {
        let config = crate::config::Config::from_entries_for_test(Default::default());
        let port = resolve_port(PocketDevice::Pc1600Emul, None, &config).unwrap();
        assert_eq!(port, "/tmp/calcu1600.serial");
    }
}
