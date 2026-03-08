package ch.erzberger.sharppc.exchange.serial;

import lombok.extern.java.Log;

import java.util.logging.Level;

/**
 * Raises a callback after a timeout expires. The timer can be reset at any time using {@link #reset()}.
 */
@Log
public class Watchdog {
    private final TimerExpiryHandler handler;
    private final long delay;
    private Thread timerThread;

    public Watchdog(TimerExpiryHandler handler, long delay) {
        this.handler = handler;
        this.delay = delay;
    }

    public void start() {
        log.log(Level.FINEST, "Watchdog starting");
        // Virtual threads are daemon threads by default — no need to setDaemon().
        timerThread = Thread.ofVirtual().start(() -> {
            try {
                log.log(Level.FINEST, "Watchdog thread sleeping for {0} ms", delay);
                Thread.sleep(delay);
            } catch (InterruptedException ex) {
                log.log(Level.FINEST, "Watchdog thread interrupted");
                Thread.currentThread().interrupt();
                return;
            }
            log.log(Level.FINEST, "Watchdog expired, raising callback");
            handler.timerExpired();
        });
    }

    public void reset() {
        if (timerThread != null) {
            log.log(Level.FINEST, "Watchdog resetting");
            timerThread.interrupt();
            this.start();
        }
    }
}
