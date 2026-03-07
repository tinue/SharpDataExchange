package ch.erzberger.sharppc.exchange.header;

import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class Ce158HeaderTest {

    // ---- Round-trip tests ----

    @Test
    @DisplayName("BASIC header: construct → serialize → parse → fields match")
    void basicRoundTrip() {
        Ce158Header original = new Ce158Header(SerialHeader.FileType.BASIC, "MYAPP", 0, 1024, 0);
        byte[] bytes = original.getHeader();
        assertEquals(27, bytes.length);

        Ce158Header parsed = new Ce158Header(bytes);
        assertEquals(SerialHeader.FileType.BASIC, parsed.getType());
        assertEquals("MYAPP", parsed.getFilename());
        assertEquals(1024, parsed.getLength());
        assertEquals(0, parsed.getStartAddr());
        assertEquals(0, parsed.getRunAddr());
    }

    @Test
    @DisplayName("RESERVE header round-trip")
    void reserveRoundTrip() {
        Ce158Header original = new Ce158Header(SerialHeader.FileType.RESERVE, "DATA", 0, 48, 0);
        byte[] bytes = original.getHeader();
        Ce158Header parsed = new Ce158Header(bytes);
        assertEquals(SerialHeader.FileType.RESERVE, parsed.getType());
        assertEquals("DATA", parsed.getFilename());
        assertEquals(48, parsed.getLength());
    }

    @Test
    @DisplayName("MACHINE header round-trip with addresses")
    void machineRoundTrip() {
        Ce158Header original = new Ce158Header(SerialHeader.FileType.MACHINE, "PROG", 0x38C5, 256, 0x38C5);
        byte[] bytes = original.getHeader();
        Ce158Header parsed = new Ce158Header(bytes);
        assertEquals(SerialHeader.FileType.MACHINE, parsed.getType());
        assertEquals("PROG", parsed.getFilename());
        assertEquals(0x38C5, parsed.getStartAddr());
        assertEquals(256, parsed.getLength());
        assertEquals(0x38C5, parsed.getRunAddr());
    }

    @Test
    @DisplayName("VARIABLES header round-trip")
    void variablesRoundTrip() {
        Ce158Header original = new Ce158Header(SerialHeader.FileType.VARIABLES, "VARS", 0, 80, 0);
        byte[] bytes = original.getHeader();
        Ce158Header parsed = new Ce158Header(bytes);
        assertEquals(SerialHeader.FileType.VARIABLES, parsed.getType());
        assertEquals(80, parsed.getLength());
    }

    @Test
    @DisplayName("Filename longer than 16 chars is truncated in header")
    void filenameTruncation() {
        Ce158Header original = new Ce158Header(
                SerialHeader.FileType.BASIC, "AVERYLONGFILENAME_EXTRA", 0, 100, 0);
        byte[] bytes = original.getHeader();
        assertEquals(27, bytes.length); // still exactly 27 bytes
        Ce158Header parsed = new Ce158Header(bytes);
        assertEquals(16, parsed.getFilename().length());
    }

    @Test
    @DisplayName("BASIC header does not store addresses (they are zeroed)")
    void basicAddressesAreZero() {
        // Even if non-zero values pass the constructor, the base class zeros them for non-MACHINE
        Ce158Header original = new Ce158Header(SerialHeader.FileType.BASIC, "TEST", 0x1234, 500, 0x5678);
        assertEquals(0, original.getStartAddr());
        assertEquals(0, original.getRunAddr());
    }

    // ---- Type char mapping ----

    @Test
    @DisplayName("Type char '@' decodes to BASIC")
    void typeCharBasic() {
        Ce158Header h = new Ce158Header(SerialHeader.FileType.BASIC, "X", 0, 10, 0);
        assertEquals('@', h.getTypeChar(SerialHeader.FileType.BASIC));
    }

    @Test
    @DisplayName("Type char 'A' decodes to RESERVE")
    void typeCharReserve() {
        Ce158Header h = new Ce158Header(SerialHeader.FileType.RESERVE, "X", 0, 10, 0);
        assertEquals('A', h.getTypeChar(SerialHeader.FileType.RESERVE));
    }

    @Test
    @DisplayName("Type char 'B' decodes to MACHINE")
    void typeCharMachine() {
        Ce158Header h = new Ce158Header(SerialHeader.FileType.MACHINE, "X", 0, 10, 0);
        assertEquals('B', h.getTypeChar(SerialHeader.FileType.MACHINE));
    }

    @Test
    @DisplayName("Type char 'H' decodes to VARIABLES")
    void typeCharVariables() {
        Ce158Header h = new Ce158Header(SerialHeader.FileType.VARIABLES, "X", 0, 10, 0);
        assertEquals('H', h.getTypeChar(SerialHeader.FileType.VARIABLES));
    }

    // ---- Error cases ----

    @Test
    @DisplayName("Header shorter than 27 bytes throws")
    void tooShortThrows() {
        assertThrows(IllegalArgumentException.class, () -> new Ce158Header(new byte[26]));
    }

    @Test
    @DisplayName("Missing magic marker throws")
    void missingMagicThrows() {
        byte[] bad = new byte[27]; // all zeros, no magic
        assertThrows(IllegalArgumentException.class, () -> new Ce158Header(bad));
    }

    @Test
    @DisplayName("Unknown type char throws")
    void unknownTypeCharThrows() {
        // Build a valid header and corrupt the type char
        Ce158Header original = new Ce158Header(SerialHeader.FileType.BASIC, "TEST", 0, 10, 0);
        byte[] bytes = original.getHeader();
        bytes[1] = 'Z'; // invalid type char
        assertThrows(IllegalArgumentException.class, () -> new Ce158Header(bytes));
    }

    // ---- Wire-format length field ----

    @Test
    @DisplayName("Length field in wire bytes is capacity-1 (per §13 Technical Reference Manual)")
    void lengthFieldIsCapacityMinusOne() {
        int length = 256;
        Ce158Header h = new Ce158Header(SerialHeader.FileType.BASIC, "TEST", 0, length, 0);
        byte[] bytes = h.getHeader();
        // Bytes 0x17-0x18 (23-24) hold the length field, big-endian
        int wireLength = ((bytes[0x17] & 0xFF) << 8) | (bytes[0x18] & 0xFF);
        assertEquals(length - 1, wireLength);
    }

    @Test
    @DisplayName("Parse real hardware reserve dump: length = 188 (wire value 0x00BB = 187 = 188-1)")
    void parseRealHardwareDump() throws Exception {
        java.io.InputStream is = getClass().getResourceAsStream("/dumps/pc1500-reserve.bin");
        if (is == null) return; // skip if file not present (CI without hardware dumps)
        byte[] data = is.readAllBytes();
        Ce158Header h = new Ce158Header(data);
        assertEquals(SerialHeader.FileType.RESERVE, h.getType());
        assertEquals("x", h.getFilename());
        assertEquals(188, h.getLength());
    }

    // ---- Magic bytes in output ----

    @Test
    @DisplayName("getHeader() starts with magic 0x01")
    void headerStartsWithMagic() {
        Ce158Header h = new Ce158Header(SerialHeader.FileType.BASIC, "PROG", 0, 100, 0);
        byte[] bytes = h.getHeader();
        assertEquals(0x01, bytes[0] & 0xFF);
        assertEquals('C', (char) bytes[2]);
        assertEquals('O', (char) bytes[3]);
        assertEquals('M', (char) bytes[4]);
    }
}
