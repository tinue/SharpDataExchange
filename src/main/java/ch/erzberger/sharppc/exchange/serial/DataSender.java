package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

import java.util.List;
import java.util.logging.Level;

/**
 * Sends data to the Pocket Computer over serial.
 *
 * <p>PC-1500/PC-1500A timing (CE-158, 19 200 baud, no flow control):
 * <ol>
 *   <li>Send the 27-byte header at full speed.</li>
 *   <li>Pause 300 ms (let the PC-1500 process the header).</li>
 *   <li>Send payload bytes at 1 ms/byte.</li>
 *   <li>Pause 500 ms before closing the port.</li>
 * </ol>
 *
 * <p>PC-1600 (9 600 baud, RTS/CTS flow control): send at full speed, no delays needed.
 */
@Log
public class DataSender {
    private static final int CE158_HEADER_SIZE = 27;
    private static final long HEADER_PAUSE_MS = 300L;
    private static final long BYTE_DELAY_MS = 1L;
    private static final long TAIL_PAUSE_MS = 500L;

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
        if (device.isPC1500()) {
            int headerSize = Math.min(CE158_HEADER_SIZE, data.length);
            byte[] header = new byte[headerSize];
            byte[] payload = new byte[data.length - headerSize];
            System.arraycopy(data, 0, header, 0, headerSize);
            System.arraycopy(data, headerSize, payload, 0, payload.length);

            serial.writeBytes(header);
            log.log(Level.FINE, "Header sent, pausing {0}ms", HEADER_PAUSE_MS);
            sleep(HEADER_PAUSE_MS);

            serial.writeBytes(payload, BYTE_DELAY_MS);
            log.log(Level.FINE, "Payload sent, pausing {0}ms", TAIL_PAUSE_MS);
            sleep(TAIL_PAUSE_MS);
        } else {
            serial.writeBytes(data);
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
            if (device.isPC1500()) {
                sleep(TAIL_PAUSE_MS);
            }
        }
        // EOF marker
        if (device.isPC1500()) {
            serial.writeBytes(new byte[]{0x0D}); // double CR signals end
        } else {
            serial.writeBytes(new byte[]{0x1A}); // ASCII EOF (Ctrl-Z)
        }
        sleep(TAIL_PAUSE_MS);
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
