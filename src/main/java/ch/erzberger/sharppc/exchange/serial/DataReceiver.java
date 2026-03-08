package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.HexFormat;
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
 */
@Log
public class DataReceiver implements ByteProcessor {
    private final ByteArrayOutputStream buffer = new ByteArrayOutputStream();
    private final long timeout;
    private Watchdog watchdog;
    private boolean dataReady = false;

    public DataReceiver(PocketPcDevice device) {
        this.timeout = device.isPC1500() ? 5000L : 500L;
        log.log(Level.FINE, "DataReceiver created, timeout={0}ms", timeout);
    }

    @Override
    public void processByte(byte byteReceived) {
        processBytes(new byte[]{byteReceived});
    }

    @Override
    public void processBytes(byte[] bytes) {
        log.log(Level.FINEST, "Received: {0}", HexFormat.of().formatHex(bytes));
        if (watchdog == null) {
            log.log(Level.FINE, "First bytes received — starting watchdog");
            watchdog = new Watchdog(() -> {
                log.log(Level.FINE, "Watchdog fired — transfer complete");
                synchronized (DataReceiver.this) {
                    dataReady = true;
                    notifyAll();
                }
            }, timeout);
            watchdog.start();
        } else {
            log.log(Level.FINEST, "More data received — resetting watchdog");
            watchdog.reset();
        }
        try {
            buffer.write(bytes);
        } catch (IOException ex) {
            log.log(Level.SEVERE, "Error buffering received bytes", ex);
        }
    }

    /**
     * Blocks until the transfer is complete (watchdog fires) and returns all received bytes.
     *
     * @return all bytes received from the Pocket Computer
     */
    public synchronized byte[] getDataWhenReady() {
        log.log(Level.FINE, "Waiting for data from Pocket Computer...");
        while (!dataReady) {
            try {
                wait();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                log.log(Level.SEVERE, "Interrupted while waiting for data");
            }
        }
        return buffer.toByteArray();
    }
}
