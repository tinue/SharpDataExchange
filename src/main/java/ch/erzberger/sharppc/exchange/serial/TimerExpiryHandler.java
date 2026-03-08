package ch.erzberger.sharppc.exchange.serial;

@FunctionalInterface
public interface TimerExpiryHandler {
    void timerExpired();
}
