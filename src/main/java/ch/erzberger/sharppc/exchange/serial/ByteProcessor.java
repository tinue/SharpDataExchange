package ch.erzberger.sharppc.exchange.serial;

public interface ByteProcessor {
    /**
     * Processes one byte received via serial.
     *
     * @param byteReceived the byte that was received
     */
    void processByte(byte byteReceived);

    default void processBytes(byte[] bytes) {
        for (byte b : bytes) {
            processByte(b);
        }
    }
}
