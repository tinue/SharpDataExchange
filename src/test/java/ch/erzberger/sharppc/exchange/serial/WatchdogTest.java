package ch.erzberger.sharppc.exchange.serial;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class WatchdogTest {
    private static final long TIMEOUT_MS = 50L;
    private boolean timerFired;
    private Watchdog watchdog;

    @BeforeEach
    void setUp() {
        timerFired = false;
        watchdog = new Watchdog(() -> timerFired = true, TIMEOUT_MS);
    }

    @Test
    @DisplayName("Watchdog does not fire before start()")
    void doesNotFireBeforeStart() throws InterruptedException {
        Thread.sleep(TIMEOUT_MS * 2);
        assertFalse(timerFired);
    }

    @Test
    @DisplayName("reset() before start() does not throw and does not start the timer")
    void resetBeforeStart() throws InterruptedException {
        assertDoesNotThrow(() -> watchdog.reset());
        Thread.sleep(TIMEOUT_MS * 2);
        assertFalse(timerFired);
    }

    @Test
    @DisplayName("Watchdog fires after timeout when not reset")
    void firesAfterTimeout() throws InterruptedException {
        watchdog.start();
        Thread.sleep(TIMEOUT_MS * 2);
        assertTrue(timerFired);
    }

    @Test
    @DisplayName("reset() postpones the timeout")
    void resetPostponesToTimeout() throws InterruptedException {
        watchdog.start();
        Thread.sleep(TIMEOUT_MS / 2);
        assertFalse(timerFired);
        watchdog.reset();
        Thread.sleep(TIMEOUT_MS / 2);
        assertFalse(timerFired);   // not yet — reset extended it
        Thread.sleep(TIMEOUT_MS);
        assertTrue(timerFired);    // now it should have fired
    }
}
