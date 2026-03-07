package ch.erzberger.sharppc.exchange.header;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class Pc1600HeaderTest {

    // ---- Round-trip tests ----

    @Test
    @DisplayName("BASIC header: construct → serialize → parse → fields match")
    void basicRoundTrip() {
        Pc1600Header original = new Pc1600Header(SerialHeader.FileType.BASIC, "PROG", 0, 2048, 0);
        byte[] bytes = original.getHeader();
        assertEquals(16, bytes.length);

        Pc1600Header parsed = new Pc1600Header(bytes);
        assertEquals(SerialHeader.FileType.BASIC, parsed.getType());
        assertEquals(2048, parsed.getLength());
        assertEquals(0, parsed.getStartAddr());
        assertEquals(0, parsed.getRunAddr());
        assertEquals(PocketPcDevice.PC1600, parsed.getDevice());
    }

    @Test
    @DisplayName("MACHINE header round-trip with addresses")
    void machineRoundTrip() {
        Pc1600Header original = new Pc1600Header(
                SerialHeader.FileType.MACHINE, "", 0x38C5, 512, 0x38C5);
        byte[] bytes = original.getHeader();
        assertEquals(16, bytes.length);

        Pc1600Header parsed = new Pc1600Header(bytes);
        assertEquals(SerialHeader.FileType.MACHINE, parsed.getType());
        assertEquals(0x38C5, parsed.getStartAddr());
        assertEquals(512, parsed.getLength());
        assertEquals(0x38C5, parsed.getRunAddr());
    }

    @Test
    @DisplayName("Large length (3-byte) round-trip")
    void largeLengthRoundTrip() {
        int bigLength = 0x12345; // needs more than 2 bytes
        Pc1600Header original = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0, bigLength, 0);
        Pc1600Header parsed = new Pc1600Header(original.getHeader());
        assertEquals(bigLength, parsed.getLength());
    }

    // ---- Magic bytes ----

    @Test
    @DisplayName("getHeader() starts with 0xFF 0x10 0x00 0x00")
    void headerMagic() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0, 100, 0);
        byte[] bytes = h.getHeader();
        assertEquals((byte) 0xFF, bytes[0]);
        assertEquals(0x10, bytes[1]);
        assertEquals(0x00, bytes[2]);
        assertEquals(0x00, bytes[3]);
    }

    @Test
    @DisplayName("BASIC type byte is 0x21")
    void basicTypeByte() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0, 100, 0);
        assertEquals(0x21, h.getHeader()[4] & 0xFF);
    }

    @Test
    @DisplayName("MACHINE type byte is 0x10")
    void machineTypeByte() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.MACHINE, "", 0, 100, 0);
        assertEquals(0x10, h.getHeader()[4] & 0xFF);
    }

    @Test
    @DisplayName("getHeader() ends with 0x00 0x0F")
    void headerEndMarker() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0, 100, 0);
        byte[] bytes = h.getHeader();
        assertEquals(0x00, bytes[14]);
        assertEquals(0x0F, bytes[15]);
    }

    // ---- BASIC header does not store addresses ----

    @Test
    @DisplayName("BASIC header zeroes start and run addresses")
    void basicAddressesZeroed() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0x1234, 100, 0x5678);
        assertEquals(0, h.getStartAddr());
        assertEquals(0, h.getRunAddr());
    }

    // ---- Error / unsupported cases ----

    @Test
    @DisplayName("Header shorter than 16 bytes throws")
    void tooShortThrows() {
        assertThrows(IllegalArgumentException.class, () -> new Pc1600Header(new byte[15]));
    }

    @Test
    @DisplayName("Missing magic marker throws")
    void missingMagicThrows() {
        byte[] bad = new byte[16]; // all zeros, no magic
        assertThrows(IllegalArgumentException.class, () -> new Pc1600Header(bad));
    }

    @Test
    @DisplayName("Unknown type byte throws")
    void unknownTypeByte() {
        Pc1600Header h = new Pc1600Header(SerialHeader.FileType.BASIC, "", 0, 100, 0);
        byte[] bytes = h.getHeader();
        bytes[4] = 0x42; // not a known type byte
        assertThrows(IllegalArgumentException.class, () -> new Pc1600Header(bytes));
    }

    @Test
    @DisplayName("RESERVE type throws UnsupportedOperationException")
    void reserveTypeUnsupported() {
        assertThrows(UnsupportedOperationException.class, () ->
                new Pc1600Header(SerialHeader.FileType.RESERVE, "", 0, 48, 0));
    }

    @Test
    @DisplayName("VARIABLES type throws UnsupportedOperationException")
    void variablesTypeUnsupported() {
        assertThrows(UnsupportedOperationException.class, () ->
                new Pc1600Header(SerialHeader.FileType.VARIABLES, "", 0, 80, 0));
    }
}
