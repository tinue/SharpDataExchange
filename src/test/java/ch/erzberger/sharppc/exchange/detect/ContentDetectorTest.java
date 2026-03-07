package ch.erzberger.sharppc.exchange.detect;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import ch.erzberger.sharppc.exchange.convert.DataType;
import ch.erzberger.sharppc.exchange.header.SerialHeader;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertEquals;

class ContentDetectorTest {

    private ContentDetector detector;

    @BeforeEach
    void setUp() {
        detector = new ContentDetector();
    }

    // ---- CE-158 header detection ----

    @Test
    @DisplayName("CE-158 BASIC header → BINARY_BASIC")
    void ce158Basic() {
        byte[] header = makeCe158Header(SerialHeader.FileType.BASIC, 100);
        assertEquals(DataType.BINARY_BASIC, detector.detect(header));
    }

    @Test
    @DisplayName("CE-158 RESERVE header → BINARY_RESERVE")
    void ce158Reserve() {
        byte[] header = makeCe158Header(SerialHeader.FileType.RESERVE, 48);
        assertEquals(DataType.BINARY_RESERVE, detector.detect(header));
    }

    @Test
    @DisplayName("CE-158 MACHINE header → MACHINE")
    void ce158Machine() {
        SerialHeader h = SerialHeader.makeHeader(PocketPcDevice.PC1500,
                SerialHeader.FileType.MACHINE, "PROG", 0x38C5, 256, 0x38C5);
        assertEquals(DataType.MACHINE, detector.detect(withPayload(h.getHeader(), 256)));
    }

    @Test
    @DisplayName("CE-158 VARIABLES header → BINARY_VARS")
    void ce158Variables() {
        byte[] header = makeCe158Header(SerialHeader.FileType.VARIABLES, 80);
        assertEquals(DataType.BINARY_VARS, detector.detect(header));
    }

    // ---- PC-1600 header detection ----

    @Test
    @DisplayName("PC-1600 BASIC header → BINARY_BASIC")
    void pc1600Basic() {
        byte[] header = makePc1600Header(0x21, 2048);
        assertEquals(DataType.BINARY_BASIC, detector.detect(header));
    }

    @Test
    @DisplayName("PC-1600 MACHINE header → MACHINE")
    void pc1600Machine() {
        byte[] header = makePc1600Header(0x10, 512);
        assertEquals(DataType.MACHINE, detector.detect(header));
    }

    // ---- ASCII BASIC detection ----

    @Test
    @DisplayName("ASCII BASIC program → ASCII_BASIC")
    void asciiBasic() {
        String program = "10 PRINT \"HELLO\"\n20 GOTO 10\n30 END\n";
        assertEquals(DataType.ASCII_BASIC, detector.detect(program.getBytes(StandardCharsets.US_ASCII)));
    }

    @Test
    @DisplayName("ASCII BASIC with mixed blank lines → ASCII_BASIC")
    void asciiBasicWithBlanks() {
        String program = "\n10 PRINT \"HI\"\n\n20 END\n30 GOTO 10\n";
        assertEquals(DataType.ASCII_BASIC, detector.detect(program.getBytes(StandardCharsets.US_ASCII)));
    }

    // ---- ASCII Reserve (SDAR) detection ----

    @Test
    @DisplayName("SDAR header text → ASCII_RESERVE")
    void asciiReserve() {
        String sdar = "; SDAR:1.0 pc1500\n; Length: 48\n3A 00 FF 1A 00 00 00 00  00 00 00 00 00 00 00 00\n";
        assertEquals(DataType.ASCII_RESERVE, detector.detect(sdar.getBytes(StandardCharsets.US_ASCII)));
    }

    // ---- ASCII Variables (SDAV) detection ----

    @Test
    @DisplayName("SDAV header text → ASCII_VARS")
    void asciiVars() {
        String sdav = "; SDAV:1.0 pc1500\n; Count: 2\nA=3.14\nB$=\"HELLO\"\n";
        assertEquals(DataType.ASCII_VARS, detector.detect(sdav.getBytes(StandardCharsets.US_ASCII)));
    }

    // ---- UNKNOWN fallback ----

    @Test
    @DisplayName("Random binary bytes → UNKNOWN")
    void unknownBinary() {
        byte[] random = new byte[]{0x42, 0x00, (byte) 0xDE, (byte) 0xAD, (byte) 0xBE, (byte) 0xEF};
        assertEquals(DataType.UNKNOWN, detector.detect(random));
    }

    @Test
    @DisplayName("Null input → UNKNOWN")
    void nullInput() {
        assertEquals(DataType.UNKNOWN, detector.detect(null));
    }

    @Test
    @DisplayName("Empty input → UNKNOWN")
    void emptyInput() {
        assertEquals(DataType.UNKNOWN, detector.detect(new byte[0]));
    }

    @Test
    @DisplayName("Plain text without line numbers → UNKNOWN")
    void plainTextNotBasic() {
        String text = "Hello World\nThis is just text\nNo line numbers here\n";
        assertEquals(DataType.UNKNOWN, detector.detect(text.getBytes(StandardCharsets.US_ASCII)));
    }

    // ---- Helpers ----

    private byte[] makeCe158Header(SerialHeader.FileType type, int length) {
        SerialHeader h = SerialHeader.makeHeader(PocketPcDevice.PC1500, type, "TEST", 0, length, 0);
        return withPayload(h.getHeader(), length);
    }

    /**
     * Build a raw PC-1600 header with the given type byte and length (little-endian at bytes 5-7).
     */
    private byte[] makePc1600Header(int typeByte, int length) {
        byte[] header = new byte[16];
        header[0] = (byte) 0xFF;
        header[1] = 0x10;
        header[2] = 0x00;
        header[3] = 0x00;
        header[4] = (byte) typeByte;
        header[5] = (byte) (length & 0xFF);
        header[6] = (byte) ((length >> 8) & 0xFF);
        header[7] = (byte) ((length >> 16) & 0xFF);
        header[14] = 0x00;
        header[15] = 0x0F;
        return header;
    }

    /** Append {@code payloadLength} zero bytes to {@code header} to simulate a full data block. */
    private byte[] withPayload(byte[] header, int payloadLength) {
        return Arrays.copyOf(header, header.length + payloadLength);
    }
}
