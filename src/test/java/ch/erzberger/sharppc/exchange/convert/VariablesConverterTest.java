package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.*;

@DisplayName("VariablesConverter")
class VariablesConverterTest {

    private static final int CE158_HEADER_SIZE = 27;

    private byte[] loadPayload(String resourcePath, int skipBytes) throws IOException {
        try (var stream = getClass().getResourceAsStream(resourcePath)) {
            assertNotNull(stream, "Resource not found: " + resourcePath);
            byte[] raw = stream.readAllBytes();
            return Arrays.copyOfRange(raw, skipBytes, raw.length);
        }
    }

    // ---- Dump-based tests ----

    @Test
    @DisplayName("pc1500-vars-numeric.bin: 5 numeric variables")
    void testNumericVariables() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-numeric.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "VARS", PocketPcDevice.PC1500);

        assertNotNull(sdav);
        assertTrue(sdav.startsWith("; SDAV:1.0 pc1500"), "Should start with SDAV header");
        assertTrue(sdav.contains("; Count: 5"), "Should have 5 variables");

        String[] lines = sdav.split("\\r?\\n");
        int numericCount = 0;
        for (String line : lines) {
            if (!line.isEmpty() && !line.startsWith(";") && !line.startsWith("DIM") && !line.startsWith("\"")) {
                numericCount++;
            }
        }
        assertEquals(5, numericCount, "Should have 5 numeric lines");

        // Known values from the spec
        assertTrue(sdav.contains("0"), "Should contain a zero value");
    }

    @Test
    @DisplayName("pc1500-vars-strings.bin: 4 string variables")
    void testStringVariables() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-strings.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "STRVARS", PocketPcDevice.PC1500);

        assertNotNull(sdav);
        assertTrue(sdav.startsWith("; SDAV:1.0 pc1500"), "Should start with SDAV header");
        assertTrue(sdav.contains("; Count: 4"), "Should have 4 variables");

        String[] lines = sdav.split("\\r?\\n");
        int stringCount = 0;
        for (String line : lines) {
            if (line.startsWith("\"") && line.endsWith("\"")) {
                stringCount++;
            }
        }
        assertEquals(4, stringCount, "Should have 4 quoted string lines");

        assertTrue(sdav.contains("\"Hi there!\""), "Should contain known string 'Hi there!'");
    }

    @Test
    @DisplayName("pc1500-vars-mixed.bin: 2 numeric + 2 string variables")
    void testMixedVariables() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-mixed.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "MIXED", PocketPcDevice.PC1500);

        assertNotNull(sdav);
        assertTrue(sdav.contains("; Count: 4"), "Should have 4 variables");

        String[] lines = sdav.split("\\r?\\n");
        int numericCount = 0;
        int stringCount = 0;
        for (String line : lines) {
            if (line.isEmpty() || line.startsWith(";") || line.startsWith("DIM")) continue;
            if (line.startsWith("\"") && line.endsWith("\"")) {
                stringCount++;
            } else {
                numericCount++;
            }
        }
        assertTrue(numericCount > 0, "Should have some numeric variables");
        assertTrue(stringCount > 0, "Should have some string variables");
        assertEquals(4, numericCount + stringCount, "Total should be 4");
    }

    @Test
    @DisplayName("pc1500-vars-longstrings-arrays.bin: DIM string arrays")
    void testLongStringArrays() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-longstrings-arrays.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "ARRAYS", PocketPcDevice.PC1500);

        assertNotNull(sdav);
        assertTrue(sdav.contains("; Count: 2"), "Should have 2 records");
        assertTrue(sdav.contains("DIM $(1)*80"), "Should contain DIM $(1)*80");
        assertTrue(sdav.contains("DIM $(2)*40"), "Should contain DIM $(2)*40");
    }

    @Test
    @DisplayName("pc1500-vars-mixed-arrays.bin: mixed arrays (skip 2 leading bytes + 27 header)")
    void testMixedArrays() throws IOException {
        // This file has 2 leading 0x00 bytes before the CE-158 header
        byte[] payload = loadPayload("/dumps/pc1500-vars-mixed-arrays.bin", 29);

        String sdav = VariablesConverter.toAscii(payload, "MIXARR", PocketPcDevice.PC1500);

        assertNotNull(sdav);
        assertTrue(sdav.contains("; Count: 3"), "Should have 3 records");
        assertTrue(sdav.contains("DIM (5)"), "Should contain DIM (5) numeric array");
        assertTrue(sdav.contains("DIM $(1)*16"), "Should contain DIM $(1)*16 string array");
    }

    // ---- BCD round-trip tests ----

    @Test
    @DisplayName("BCD round-trip: 3.14159265")
    void testBcdRoundTripPi() {
        String original = "3.14159265";
        byte[] encoded = VariablesConverter.parseBcd(original);
        String decoded = VariablesConverter.formatBcd(encoded);
        assertEquals(original, decoded, "3.14159265 should round-trip exactly");
    }

    @Test
    @DisplayName("BCD round-trip: 0")
    void testBcdRoundTripZero() {
        String original = "0";
        byte[] encoded = VariablesConverter.parseBcd(original);
        String decoded = VariablesConverter.formatBcd(encoded);
        assertEquals(original, decoded, "0 should round-trip as '0'");
    }

    @Test
    @DisplayName("BCD round-trip: 1E-9")
    void testBcdRoundTripSmall() {
        String original = "1E-9";
        byte[] encoded = VariablesConverter.parseBcd(original);
        String decoded = VariablesConverter.formatBcd(encoded);
        assertEquals(original, decoded, "1E-9 should round-trip exactly");
    }

    @Test
    @DisplayName("BCD round-trip: -1.7534")
    void testBcdRoundTripNegative() {
        String original = "-1.7534";
        byte[] encoded = VariablesConverter.parseBcd(original);
        String decoded = VariablesConverter.formatBcd(encoded);
        assertEquals(original, decoded, "-1.7534 should round-trip exactly");
    }

    @Test
    @DisplayName("BCD all-zeros decode to 0")
    void testBcdAllZeros() {
        byte[] zeros = new byte[8];
        String result = VariablesConverter.formatBcd(zeros);
        assertEquals("0", result, "All-zero BCD bytes should decode to '0'");
    }

    // ---- String escape round-trip tests ----

    @Test
    @DisplayName("String escape/unescape round-trips plain ASCII")
    void testStringEscapeRoundTripPlain() {
        byte[] original = "Hi there!".getBytes();
        String escaped = VariablesConverter.escapeString(original);
        byte[] decoded = VariablesConverter.unescapeString(escaped);
        assertArrayEquals(original, decoded, "Plain ASCII should round-trip");
    }

    @Test
    @DisplayName("String escape/unescape round-trips special chars")
    void testStringEscapeRoundTripSpecial() {
        byte[] original = new byte[]{'a', '\\', '"', 0x01, 'z'};
        String escaped = VariablesConverter.escapeString(original);
        assertTrue(escaped.contains("\\\\"), "Backslash should be escaped as \\\\");
        assertTrue(escaped.contains("\\\""), "Quote should be escaped as \\\"");
        assertTrue(escaped.contains("\\x01"), "Control byte should be escaped as \\xHH");
        byte[] decoded = VariablesConverter.unescapeString(escaped);
        assertArrayEquals(original, decoded, "Special chars should round-trip");
    }

    @Test
    @DisplayName("'Hi there!' encodes and decodes correctly via escapeString")
    void testHiThereEscape() {
        byte[] original = "Hi there!".getBytes();
        String escaped = VariablesConverter.escapeString(original);
        assertEquals("Hi there!", escaped, "Plain ASCII should not be escaped");
        byte[] decoded = VariablesConverter.unescapeString(escaped);
        assertArrayEquals(original, decoded);
    }

    @Test
    @DisplayName("encodeStringSlot null-pads to maxLen")
    void testEncodeStringSlot() {
        byte[] slot = VariablesConverter.encodeStringSlot("Hi", 16);
        assertEquals(16, slot.length, "Slot should be padded to 16 bytes");
        assertEquals('H', slot[0] & 0xFF);
        assertEquals('i', slot[1] & 0xFF);
        assertEquals(0x00, slot[2] & 0xFF, "Remaining bytes should be null-padded");
        assertEquals(0x00, slot[15] & 0xFF, "Last byte should be null-padded");
    }

    // ---- toBinary round-trip tests ----

    @Test
    @DisplayName("toBinary round-trip for pc1500-vars-numeric.bin")
    void testToBinaryRoundTripNumeric() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-numeric.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "VARS", PocketPcDevice.PC1500);
        byte[] roundTripped = VariablesConverter.toBinary(sdav, PocketPcDevice.PC1500);

        assertArrayEquals(payload, roundTripped, "Numeric variables should round-trip binary exactly");
    }

    @Test
    @DisplayName("toBinary round-trip for pc1500-vars-strings.bin")
    void testToBinaryRoundTripStrings() throws IOException {
        byte[] payload = loadPayload("/dumps/pc1500-vars-strings.bin", CE158_HEADER_SIZE);

        String sdav = VariablesConverter.toAscii(payload, "STRVARS", PocketPcDevice.PC1500);
        byte[] roundTripped = VariablesConverter.toBinary(sdav, PocketPcDevice.PC1500);

        assertArrayEquals(payload, roundTripped, "String variables should round-trip binary exactly");
    }
}
