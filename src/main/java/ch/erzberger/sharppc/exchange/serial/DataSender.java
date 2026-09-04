package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import ch.erzberger.sharppc.exchange.header.SerialHeader;
import lombok.extern.java.Log;

import java.util.Arrays;
import java.util.List;
import java.util.logging.Level;

/**
 * Sends data to the Pocket Computer over serial.
 *
 * <p>PC-1500/PC-1500A timing (CE-158, 19 200 baud, no flow control):
 * <ol>
 *   <li>Send the header at full speed.</li>
 *   <li>Pause 300 ms (let the PC-1500 process the header).</li>
 *   <li>Send payload bytes at 1 ms/byte.</li>
 *   <li>Drain, then pause 500 ms before closing the port.</li>
 * </ol>
 *
 * <p>PC-1600 (9 600 baud, RTS/CTS flow control): send at full speed, then drain.
 *
 * <p>PC-1600 emulator (9 600 baud, pseudo-terminal, no flow control): same paced
 * scheme as the PC-1500, because there is no handshake to throttle the sender and a
 * pseudo-terminal drops whatever is still queued when the port closes.
 */
@Log
public class DataSender {
    private static final int CE158_HEADER_SIZE = 27;
    private static final long HEADER_PAUSE_MS = 300L;
    private static final long BYTE_DELAY_MS = 1L;
    private static final long TAIL_PAUSE_MS = 500L;
    private static final long DRAIN_TIMEOUT_MS = 2000L;

    private final SerialPortWrapper serial;
    private final PocketPcDevice device;

    public DataSender(SerialPortWrapper serial, PocketPcDevice device) {
        this.serial = serial;
        this.device = device;
    }

    /**
     * Send a binary data block (header + payload) to the Pocket Computer.
     *
     * @param data fully-formed data bytes including header
     */
    public void sendData(byte[] data) {
        if (device.isPacedSend()) {
            int headerSize = pacedHeaderSize(data);
            byte[] header = Arrays.copyOf(data, headerSize);
            byte[] payload = Arrays.copyOfRange(data, headerSize, data.length);

            serial.writeBytes(header);
            log.log(Level.FINE, "Header ({0} bytes) sent, pausing {1}ms", new Object[]{headerSize, HEADER_PAUSE_MS});
            sleep(HEADER_PAUSE_MS);

            serial.writeBytes(payload, BYTE_DELAY_MS);
            serial.drainOutput(DRAIN_TIMEOUT_MS);
            log.log(Level.FINE, "Payload sent, pausing {0}ms", TAIL_PAUSE_MS);
            sleep(TAIL_PAUSE_MS);
        } else {
            serial.writeBytes(data);
            serial.drainOutput(DRAIN_TIMEOUT_MS);
        }
    }

    /**
     * Send ASCII BASIC lines to the Pocket Computer.
     * Each line is sent as CP437 bytes with the device-appropriate line ending.
     * An EOF marker is appended after the last line.
     *
     * @param lines ASCII BASIC lines (without line endings)
     */
    public void sendData(List<String> lines) {
        for (String line : lines) {
            serial.writeAscii(line, device);
            if (device.isPacedSend()) {
                sleep(TAIL_PAUSE_MS);
            }
        }
        // EOF marker
        if (device.isPC1500()) {
            serial.writeBytes(new byte[]{0x0D}); // double CR signals end
        } else {
            serial.writeBytes(new byte[]{0x1A}); // ASCII EOF (Ctrl-Z)
        }
        serial.drainOutput(DRAIN_TIMEOUT_MS);
        sleep(TAIL_PAUSE_MS);
    }

    /**
     * Number of leading bytes to treat as the header for the paced send (sent at full
     * speed, then followed by a pause). The PC-1500 CE-158 header is a fixed 27 bytes;
     * for anything else, ask the header parser how long the recognised header is.
     */
    private int pacedHeaderSize(byte[] data) {
        if (device.isPC1500()) {
            return Math.min(CE158_HEADER_SIZE, data.length);
        }
        SerialHeader header = SerialHeader.makeHeader(data);
        int size = header != null ? header.getHeader().length : 0;
        return Math.min(size, data.length);
    }

    private void sleep(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException e) {
            log.log(Level.WARNING, "Sleep interrupted");
            Thread.currentThread().interrupt();
        }
    }
}
