package ch.erzberger.sharppc.exchange.serial;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.util.concurrent.CompletableFuture;

import static org.junit.jupiter.api.Assertions.*;

class DataReceiverTest {

    @Test
    @DisplayName("raw mode ignores header-encoded length and ends via the idle watchdog")
    void rawModeIgnoresHeaderLength() {
        // A well-formed PC-1600 MACHINE header (magic FF 10 00 00, type 0x10) declaring a
        // payload length (little-endian, bytes 5-7) far larger than what actually follows.
        // In header-aware mode this would make the receiver wait forever for more bytes;
        // in raw mode it must be ignored entirely.
        byte[] fakeHeaderShapedData = new byte[]{
                (byte) 0xFF, 0x10, 0x00, 0x00, 0x10,
                (byte) 0xFF, (byte) 0xFF, 0x00, // declared length 0xFFFF -- far more than sent
                0x00, 0x00, 0x00,
                0x00, 0x00, 0x00,
                0x00, 0x0F,
                0x01, 0x02, 0x03
        };

        DataReceiver receiver = new DataReceiver(PocketPcDevice.PC1600, true);
        CompletableFuture<byte[]> result = CompletableFuture.supplyAsync(receiver::getDataWhenReady);

        receiver.processBytes(fakeHeaderShapedData);

        byte[] received = assertDoesNotThrow(() -> result.get(2, java.util.concurrent.TimeUnit.SECONDS));
        assertArrayEquals(fakeHeaderShapedData, received);
    }

    @Test
    @DisplayName("raw mode buffers bytes across multiple calls and preserves order")
    void rawModeBuffersInOrder() {
        DataReceiver receiver = new DataReceiver(PocketPcDevice.PC1600, true);
        CompletableFuture<byte[]> result = CompletableFuture.supplyAsync(receiver::getDataWhenReady);

        receiver.processBytes(new byte[]{1, 2, 3});
        receiver.processBytes(new byte[]{4, 5, 6});

        byte[] received = assertDoesNotThrow(() -> result.get(2, java.util.concurrent.TimeUnit.SECONDS));
        assertArrayEquals(new byte[]{1, 2, 3, 4, 5, 6}, received);
    }

    @Test
    @DisplayName("non-raw mode still completes via the watchdog when no header is present")
    void nonRawModeFallsBackToWatchdog() {
        DataReceiver receiver = new DataReceiver(PocketPcDevice.PC1600);
        CompletableFuture<byte[]> result = CompletableFuture.supplyAsync(receiver::getDataWhenReady);

        receiver.processBytes(new byte[]{0x42});

        byte[] received = assertDoesNotThrow(() -> result.get(2, java.util.concurrent.TimeUnit.SECONDS));
        assertArrayEquals(new byte[]{0x42}, received);
    }
}
