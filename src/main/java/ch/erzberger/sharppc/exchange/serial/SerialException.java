package ch.erzberger.sharppc.exchange.serial;

/**
 * Custom exception for serial port errors.
 */
public class SerialException extends RuntimeException {
    public SerialException(String message) {
        super(message);
    }

    public SerialException(String message, Throwable cause) {
        super(message, cause);
    }
}
