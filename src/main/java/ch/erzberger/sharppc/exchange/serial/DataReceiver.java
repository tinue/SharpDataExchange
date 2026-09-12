package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import ch.erzberger.sharppc.exchange.header.SerialHeader;
import lombok.extern.java.Log;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.HexFormat;
import java.util.concurrent.CountDownLatch;
import java.util.logging.Level;

/**
 * Receives a complete data transfer from the Pocket Computer over serial.
 *
 * <p>Implements {@link ByteProcessor} so it can be passed directly to
 * {@link SerialPortWrapper#openPort(int, boolean, ByteProcessor)}.
 * Call {@link #getDataWhenReady()} to block until the transfer is complete.
 *
 * <p>Uses a {@link Watchdog} to detect end-of-transfer:
 * <ul>
 *   <li>PC-1500/PC-1500A: 5 000 ms timeout (variables via PRINT# are slow)</li>
 *   <li>PC-1600: 500 ms timeout</li>
 * </ul>
 *
 * <p>In raw mode ({@code rawMode = true}), header/length detection is skipped entirely —
 * the transfer always ends via the idle watchdog, regardless of the byte content. This is
 * for capturing an arbitrary byte stream (e.g. a hand-written assembly sender) with no
 * serial header at all.
 */
@Log
public class DataReceiver implements ByteProcessor {
    private final ByteArrayOutputStream buffer = new ByteArrayOutputStream();
    private final CountDownLatch done = new CountDownLatch(1);
    private final long timeout;
    private final boolean rawMode;
    private Watchdog watchdog;
    private int expectedTotal = -1; // -1 = unknown; determined from header when possible

    public DataReceiver(PocketPcDevice device) {
        this(device, false);
    }

    public DataReceiver(PocketPcDevice device, boolean rawMode) {
        this.timeout = device.isPC1500() ? 5000L : 500L;
        this.rawMode = rawMode;
        log.log(Level.FINE, "DataReceiver created, timeout={0}ms, rawMode={1}",
                new Object[]{timeout, rawMode});
    }

    @Override
    public void processByte(byte byteReceived) {
        processBytes(new byte[]{byteReceived});
    }

    @Override
    public void processBytes(byte[] bytes) {
        log.log(Level.FINEST, "Received: {0}", HexFormat.of().formatHex(bytes));
        try {
            buffer.write(bytes);
        } catch (IOException ex) {
            log.log(Level.SEVERE, "Error buffering received bytes", ex);
        }

        // Try to determine the expected byte count from the header (once, when we have enough bytes).
        // For CE-158 VARIABLES the length field is meaningless, so expectedTotal stays -1 for those.
        // Skipped entirely in raw mode: an untyped byte stream has no header to parse, and any
        // accidental header-shaped bytes inside it must not be mistaken for one.
        if (!rawMode && expectedTotal < 0) {
            expectedTotal = SerialHeader.expectedTotalBytes(buffer.toByteArray());
            if (expectedTotal > 0) {
                log.log(Level.FINE, "Header parsed — expecting {0} bytes total", expectedTotal);
            }
        }

        // If we know the total, terminate as soon as it is reached — no timeout needed.
        if (expectedTotal > 0 && buffer.size() >= expectedTotal) {
            log.log(Level.FINE, "All {0} bytes received — transfer complete", expectedTotal);
            done.countDown();
            return;
        }

        // Fall back to watchdog for VARIABLES or any unrecognised stream.
        if (watchdog == null) {
            log.log(Level.FINE, "Starting watchdog (timeout fallback)");
            watchdog = new Watchdog(() -> {
                log.log(Level.FINE, "Watchdog fired — transfer complete");
                done.countDown();
            }, timeout);
            watchdog.start();
        } else {
            log.log(Level.FINEST, "More data received — resetting watchdog");
            watchdog.reset();
        }
    }

    /**
     * Blocks until the transfer is complete (watchdog fires) and returns all received bytes.
     *
     * @return all bytes received from the Pocket Computer
     */
    public byte[] getDataWhenReady() {
        log.log(Level.FINE, "Waiting for data from Pocket Computer...");
        try {
            done.await();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            log.log(Level.SEVERE, "Interrupted while waiting for data");
        }
        return buffer.toByteArray();
    }
}
