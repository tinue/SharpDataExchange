package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharpbasic.core.keyword.BasicKeyword;
import ch.erzberger.sharpbasic.core.keyword.KeywordRegistry;
import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.Arrays;
import java.util.Optional;

import static org.junit.jupiter.api.Assertions.*;

@DisplayName("ReserveAreaConverter")
class ReserveAreaConverterTest {

    private static final int CE158_HEADER_SIZE = 27;
    private static final int PAYLOAD_SIZE = 188;

    private byte[] loadPayload(String resourcePath, int headerSize) throws IOException {
        try (var stream = getClass().getResourceAsStream(resourcePath)) {
            assertNotNull(stream, "Resource not found: " + resourcePath);
            byte[] raw = stream.readAllBytes();
            return Arrays.copyOfRange(raw, headerSize, raw.length);
        }
    }

    @Test
    @DisplayName("toAscii produces valid SDAR structure from pc1500-reserve.bin")
    void testToAsciiStructure() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-reserve.bin", CE158_HEADER_SIZE);
        assertEquals(PAYLOAD_SIZE, payload.length, "Payload should be 188 bytes after header strip");

        String sdar = ReserveAreaConverter.toAscii(payload, "MYAPP", PocketPcDevice.PC1500);

        assertNotNull(sdar);
        assertTrue(sdar.startsWith("; SDAR:1.0 pc1500"), "Should start with SDAR header");
        assertTrue(sdar.contains("[layer 1]"), "Should contain layer 1");
        assertTrue(sdar.contains("[layer 2]"), "Should contain layer 2");
        assertTrue(sdar.contains("[layer 3]"), "Should contain layer 3");
        assertTrue(sdar.contains("label:"), "Should contain label lines");
        assertTrue(sdar.contains("key 1:"), "Should contain key 1 lines");
        assertTrue(sdar.contains("key 2:"), "Should contain key 2 lines");
        assertTrue(sdar.contains("key 3:"), "Should contain key 3 lines");
        assertTrue(sdar.contains("key 4:"), "Should contain key 4 lines");
        assertTrue(sdar.contains("key 5:"), "Should contain key 5 lines");
        assertTrue(sdar.contains("key 6:"), "Should contain key 6 lines");
        assertTrue(sdar.contains("; Filename: MYAPP"), "Should contain filename comment");
    }

    @Test
    @DisplayName("toAscii then toBinary round-trips the payload")
    void testRoundTrip() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-reserve.bin", CE158_HEADER_SIZE);

        String sdar = ReserveAreaConverter.toAscii(payload, "MYAPP", PocketPcDevice.PC1500);
        byte[] roundTripped = ReserveAreaConverter.toBinary(sdar, PocketPcDevice.PC1500);

        assertEquals(PAYLOAD_SIZE, roundTripped.length, "Round-tripped payload should be 188 bytes");
        assertArrayEquals(payload, roundTripped, "Round-tripped payload should match original");
    }

    @Test
    @DisplayName("tokenizeContent tokenizes known keywords with 2-byte token codes")
    void testTokenizeContentKeyword() {
        KeywordRegistry registry = KeywordRegistry.forPc1500();

        // "ABS(" -> ABS as 2-byte token, '(' as plain byte 0x28
        byte[] result = ReserveAreaConverter.tokenizeContent("ABS(", registry);

        assertTrue(result.length >= 3, "Should have at least 3 bytes: 2 for token + 1 for '('");

        // The last byte should be 0x28 ('(')
        assertEquals(0x28, result[result.length - 1] & 0xFF, "Last byte should be '(' = 0x28");

        // The first byte should be 0xF0 or 0xF1 (token prefix)
        int firstByte = result[0] & 0xFF;
        assertTrue(firstByte == 0xF0 || firstByte == 0xF1,
                "First byte should be 0xF0 or 0xF1 token prefix, got: 0x" + Integer.toHexString(firstByte));

        // Verify ABS is in the registry
        Optional<BasicKeyword> abs = registry.lookup("ABS");
        assertTrue(abs.isPresent(), "ABS should be in PC1500 registry");
        int absCode = abs.get().tokenCode();
        assertEquals((absCode >> 8) & 0xFF, result[0] & 0xFF, "High byte of ABS token code");
        assertEquals(absCode & 0xFF, result[1] & 0xFF, "Low byte of ABS token code");
    }

    @Test
    @DisplayName("tokenizeContent handles plain text without keywords")
    void testTokenizeContentPlain() {
        KeywordRegistry registry = KeywordRegistry.forPc1500();

        byte[] result = ReserveAreaConverter.tokenizeContent("123", registry);

        assertEquals(3, result.length, "Plain chars should be 1 byte each");
        assertEquals('1', result[0] & 0xFF);
        assertEquals('2', result[1] & 0xFF);
        assertEquals('3', result[2] & 0xFF);
    }

    @Test
    @DisplayName("tokenizeContent handles FOR keyword")
    void testTokenizeContentFor() {
        KeywordRegistry registry = KeywordRegistry.forPc1500();

        byte[] result = ReserveAreaConverter.tokenizeContent("FOR", registry);

        assertEquals(2, result.length, "FOR should tokenize to exactly 2 bytes");
        int firstByte = result[0] & 0xFF;
        assertTrue(firstByte == 0xF0 || firstByte == 0xF1,
                "First byte should be token prefix 0xF0 or 0xF1");
    }

    @Test
    @DisplayName("toBinary throws when pool exceeds 110 bytes")
    void testPoolOverflow() {
        // Build a SDAR text with keys that together exceed 110 bytes
        StringBuilder sdar = new StringBuilder();
        sdar.append("; SDAR:1.0 pc1500\n");
        sdar.append("\n[layer 1]\n");
        sdar.append("label: Test\n");
        // Each key gets a long string of 20 chars; 6 keys * (1 keycode + 20 content) = 126 bytes > 110
        String longContent = "ABCDEFGHIJKLMNOPQRST"; // 20 chars (no keywords to tokenize)
        for (int k = 1; k <= 6; k++) {
            sdar.append("key ").append(k).append(": ").append(longContent).append('\n');
        }
        sdar.append("\n[layer 2]\n");
        sdar.append("label:\n");
        for (int k = 1; k <= 6; k++) {
            sdar.append("key ").append(k).append(":\n");
        }
        sdar.append("\n[layer 3]\n");
        sdar.append("label:\n");
        for (int k = 1; k <= 6; k++) {
            sdar.append("key ").append(k).append(":\n");
        }

        assertThrows(IllegalArgumentException.class,
                () -> ReserveAreaConverter.toBinary(sdar.toString(), PocketPcDevice.PC1500),
                "Should throw when pool exceeds 110 bytes");
    }

    @Test
    @DisplayName("toBinary handles empty keys correctly")
    void testEmptyKeys() {
        String sdar = "; SDAR:1.0 pc1500\n" +
                "\n[layer 1]\n" +
                "label:\n" +
                "key 1:\n" +
                "key 2:\n" +
                "key 3:\n" +
                "key 4:\n" +
                "key 5:\n" +
                "key 6:\n" +
                "\n[layer 2]\n" +
                "label:\n" +
                "key 1:\n" +
                "key 2:\n" +
                "key 3:\n" +
                "key 4:\n" +
                "key 5:\n" +
                "key 6:\n" +
                "\n[layer 3]\n" +
                "label:\n" +
                "key 1:\n" +
                "key 2:\n" +
                "key 3:\n" +
                "key 4:\n" +
                "key 5:\n" +
                "key 6:\n";

        byte[] result = ReserveAreaConverter.toBinary(sdar, PocketPcDevice.PC1500);
        assertEquals(188, result.length, "Payload should always be 188 bytes");
        // Pool area should start with 0x00 (terminator) since all keys are empty
        assertEquals(0x00, result[3 * 26] & 0xFF, "Pool should start with terminator when all keys empty");
    }

    @Test
    @DisplayName("toBinary with whitespace/blank keys produces valid 188-byte payload")
    void testBlankSdarProducesValidPayload() {
        String sdar = "; SDAR:1.0 pc1500\n" +
                "\n[layer 1]\n" +
                "label: \n" +
                "key 1: \n" +
                "key 2: \n" +
                "key 3: \n" +
                "key 4: \n" +
                "key 5: \n" +
                "key 6: \n" +
                "\n[layer 2]\n" +
                "label: \n" +
                "key 1: \n" +
                "key 2: \n" +
                "key 3: \n" +
                "key 4: \n" +
                "key 5: \n" +
                "key 6: \n" +
                "\n[layer 3]\n" +
                "label: \n" +
                "key 1: \n" +
                "key 2: \n" +
                "key 3: \n" +
                "key 4: \n" +
                "key 5: \n" +
                "key 6: \n";

        byte[] result = ReserveAreaConverter.toBinary(sdar, PocketPcDevice.PC1500);
        assertEquals(188, result.length, "Payload should be exactly 188 bytes");
    }

    @Test
    @DisplayName("toAscii throws for payload shorter than 188 bytes")
    void testToAsciiShortPayload() {
        byte[] tooShort = new byte[100];
        assertThrows(IllegalArgumentException.class,
                () -> ReserveAreaConverter.toAscii(tooShort, "TEST", PocketPcDevice.PC1500));
    }
}
