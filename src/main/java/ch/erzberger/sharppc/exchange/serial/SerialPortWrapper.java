package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import com.fazecast.jSerialComm.SerialPort;
import com.fazecast.jSerialComm.SerialPortDataListener;
import com.fazecast.jSerialComm.SerialPortEvent;
import lombok.extern.java.Log;

import java.nio.charset.Charset;
import java.util.logging.Level;

import static com.fazecast.jSerialComm.SerialPort.FLOW_CONTROL_CTS_ENABLED;
import static com.fazecast.jSerialComm.SerialPort.FLOW_CONTROL_RTS_ENABLED;

/**
 * Helper for serial port access. Auto-detects the port when no name is given.
 *
 * <p>Implements {@link AutoCloseable} to ensure the serial port is properly released
 * when used in a try-with-resources block. Throws {@link SerialException} for
 * port access or configuration errors.
 */
@Log
public class SerialPortWrapper implements AutoCloseable {
    private final SerialPort port;
    private ByteProcessor byteProcessor;

    /** Auto-detect the serial port. */
    public SerialPortWrapper() {
        this(null);
    }

    /**
     * Use the named port, or auto-detect if {@code portName} is null or empty.
     *
     * @param portName System port name (e.g. "cu.usbserial-A1B2C3"), or null/empty for auto-detect
     */
    public SerialPortWrapper(String portName) {
        // Trim stray whitespace and trailing slashes. A path like "/dev/ttys006/"
        // makes the native open() fail with ENOTDIR (errno 20), because the trailing
        // slash asks the OS for a directory.
        if (portName != null) {
            portName = portName.trim();
            while (portName.length() > 1 && portName.endsWith("/")) {
                portName = portName.substring(0, portName.length() - 1);
            }
        }
        if (portName == null || portName.isEmpty()) {
            log.log(Level.FINE, "Auto-detecting serial port");
            this.port = autoDetectPort();
        } else {
            log.log(Level.FINE, "Detecting serial port: {0}", portName);
            this.port = detectPort(portName);
        }
        if (port == null) {
            throw new SerialException("Could not open serial port");
        }
    }

    private SerialPort detectPort(String portName) {
        SerialPort[] ports = SerialPort.getCommPorts();
        int numPorts = 0;
        SerialPort lastDetected = null;
        for (SerialPort p : ports) {
            if (p.getSystemPortName().contains(portName)) {
                if (p.getSystemPortName().startsWith("tty.usbmodem")) {
                    log.log(Level.FINE, "Filtering Mac tty.usbmodem port: {0}", p.getSystemPortName());
                    break;
                }
                lastDetected = p;
                numPorts++;
            }
        }
        if (numPorts == 1) {
            log.log(Level.FINE, "Found matching port: {0}", lastDetected.getSystemPortName());
            return lastDetected;
        }
        // The enumeration in getCommPorts() only reports "real" devices (on macOS the
        // cu.* / tty.* nodes). Pseudo-terminals such as an emulator's /dev/ttysNNN are
        // never listed, so when nothing matched, fall back to opening the name verbatim.
        if (numPorts == 0) {
            log.log(Level.FINE, "No enumerated port matched {0}; trying it as an explicit device path", portName);
            try {
                SerialPort explicit = SerialPort.getCommPort(portName);
                log.log(Level.FINE, "Using explicit port: {0}", explicit.getSystemPortPath());
                return explicit;
            } catch (com.fazecast.jSerialComm.SerialPortInvalidPortException e) {
                log.log(Level.FINE, "{0} is not a valid device path", portName);
            }
        }
        log.log(Level.FINE, "No unique port matched {0}", portName);
        return null;
    }

    private SerialPort autoDetectPort() {
        SerialPort[] ports = SerialPort.getCommPorts();
        if (ports.length == 1) {
            log.log(Level.FINE, "Single port found: {0}", ports[0].getSystemPortName());
            return ports[0];
        }
        int numFound = 0;
        SerialPort lastFound = null;
        for (SerialPort p : ports) {
            String name = p.getSystemPortName();
            if (name.startsWith("cu.usb") || name.startsWith("ttyACM") || name.startsWith("ttyUSB")) {
                log.log(Level.FINE, "Candidate port: {0}", name);
                lastFound = p;
                numFound++;
            }
        }
        if (numFound == 1) {
            log.log(Level.FINE, "Auto-detected port: {0}", lastFound.getSystemPortName());
            return lastFound;
        }
        log.log(Level.FINE, "Auto-detection found {0} candidates — ambiguous", numFound);
        return null;
    }

    public void openPort(int baudRate, boolean handShake, ByteProcessor byteProcessor) {
        this.byteProcessor = byteProcessor;
        port.addDataListener(new Listener());
        if (openPort(baudRate, handShake, port)) {
            log.log(Level.FINEST, "Port {0} opened for reading", port.getSystemPortName());
        } else {
            String detail = openFailureDetail();
            log.log(Level.SEVERE, "Failed to open port {0} for reading{1}", new Object[]{port.getSystemPortName(), detail});
            throw new SerialException("Could not open serial port " + port.getSystemPortPath() + detail);
        }
    }

    public void openPort(int baudRate, boolean handShake) {
        if (openPort(baudRate, handShake, port)) {
            log.log(Level.FINEST, "Port {0} opened for writing (baud={1})", new Object[]{port.getSystemPortName(), baudRate});
        } else {
            String detail = openFailureDetail();
            log.log(Level.SEVERE, "Failed to open port {0} for writing{1}", new Object[]{port.getSystemPortName(), detail});
            throw new SerialException("Could not open serial port " + port.getSystemPortPath() + detail);
        }
    }

    /** jSerialComm's native errno/location for the last failed open, for the log message. */
    private String openFailureDetail() {
        return " (path=" + port.getSystemPortPath()
                + ", errno=" + port.getLastErrorCode()
                + ", location=" + port.getLastErrorLocation() + ")";
    }

    @Override
    public void close() {
        if (port != null && port.isOpen()) {
            port.closePort();
        }
    }

    public void closePort() {
        close();
    }

    /**
     * Write bytes with a per-byte delay.
     *
     * @param bytesToWrite bytes to send
     * @param delay        milliseconds to sleep after each byte
     * @return number of bytes written
     */
    public int writeBytes(byte[] bytesToWrite, long delay) {
        int written = 0;
        for (byte b : bytesToWrite) {
            port.writeBytes(new byte[]{b}, 1);
            written++;
            try {
                Thread.sleep(delay);
            } catch (InterruptedException e) {
                log.log(Level.WARNING, "writeBytes sleep interrupted");
                Thread.currentThread().interrupt();
            }
        }
        return written;
    }

    public int writeBytes(byte[] bytesToWrite) {
        return port.writeBytes(bytesToWrite, bytesToWrite.length);
    }

    /**
     * Block until the OS write buffer has been handed off, or {@code timeoutMs} elapses.
     *
     * <p>Without a hardware handshake there is nothing to tell the sender the far end has
     * caught up. Draining before the port is closed keeps a pseudo-terminal peer (an
     * emulator) from losing the tail of the transfer, which is discarded if the port
     * closes while bytes are still queued.
     */
    public void drainOutput(long timeoutMs) {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            int pending = port.bytesAwaitingWrite();
            if (pending <= 0) {
                return;
            }
            try {
                Thread.sleep(5);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
        }
        log.log(Level.FINE, "drainOutput: {0} bytes still pending after {1}ms",
                new Object[]{port.bytesAwaitingWrite(), timeoutMs});
    }

    public int writeAscii(String line, PocketPcDevice device) {
        if (line == null || line.isEmpty()) {
            log.log(Level.SEVERE, "writeAscii called with null/empty line");
            return 0;
        }
        byte[] lineBytes = line.getBytes(Charset.forName("Cp437"));
        int eolSize = device.isPC1500() ? 1 : 2;
        byte[] buf = new byte[lineBytes.length + eolSize];
        System.arraycopy(lineBytes, 0, buf, 0, lineBytes.length);
        buf[lineBytes.length] = 0x0D;
        if (device.isPC1600()) {
            buf[lineBytes.length + 1] = 0x0A;
        }
        return writeBytes(buf);
    }

    public void flush() {
        if (port.flushIOBuffers()) {
            log.log(Level.FINEST, "Flush successful on {0}", port.getSystemPortName());
        } else {
            log.log(Level.WARNING, "Flush failed on {0}", port.getSystemPortName());
        }
    }

    public String getSystemPortName() {
        return port.getSystemPortName();
    }

    private boolean openPort(int baudRate, boolean handShake, SerialPort p) {
        // jSerialComm grabs an exclusive lock (TIOCEXCL) by default. When the other end
        // is an emulator's pseudo-terminal, the emulator already holds the slave open and
        // the lock request makes openPort() fail with EBUSY. A real USB adapter is never
        // opened by anyone else, so dropping the exclusive lock is safe in both cases.
        p.disableExclusiveLock();
        p.setParity(SerialPort.NO_PARITY);
        p.setNumStopBits(SerialPort.ONE_STOP_BIT);
        p.setNumDataBits(8);
        p.setBaudRate(baudRate);
        if (handShake) {
            p.setFlowControl(FLOW_CONTROL_RTS_ENABLED | FLOW_CONTROL_CTS_ENABLED);
        }
        return p.openPort();
    }

    private class Listener implements SerialPortDataListener {
        @Override
        public int getListeningEvents() {
            return SerialPort.LISTENING_EVENT_DATA_AVAILABLE;
        }

        @Override
        public void serialEvent(SerialPortEvent event) {
            if (port == null) {
                log.log(Level.SEVERE, "serialEvent: port is null");
                return;
            }
            if (event.getEventType() != SerialPort.LISTENING_EVENT_DATA_AVAILABLE) {
                log.log(Level.SEVERE, "Unexpected event type: {0}", event.getEventType());
                return;
            }
            int available = port.bytesAvailable();
            if (available <= 0) {
                log.log(Level.SEVERE, "Event fired but no bytes available: {0}", available);
                return;
            }
            byte[] buffer = new byte[available];
            int read = port.readBytes(buffer, available);
            byte[] bytes = new byte[read];
            System.arraycopy(buffer, 0, bytes, 0, read);
            byteProcessor.processBytes(bytes);
        }
    }
}
