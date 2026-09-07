package ch.erzberger.sharppc.exchange.cli;

import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class CliParserTest {

    private CliParser parser;

    @BeforeEach
    void setUp() {
        parser = new CliParser();
    }

    // ---- Happy paths ----

    @Test
    @DisplayName("get with file returns correct CliArgs")
    void getWithFile() {
        CliArgs args = parser.parse(new String[]{"get", "output.bas"});
        assertNotNull(args);
        assertEquals("get", args.verb());
        assertEquals("output.bas", args.file());
        assertEquals(PocketPcDevice.PC1500, args.device());
        assertEquals(OutputFormat.ASCII, args.format()); // default
        assertNull(args.port());
        assertNull(args.startAddress());
        assertNull(args.runAddress());
        assertFalse(args.addUtils());
    }

    @Test
    @DisplayName("put with file returns correct CliArgs")
    void putWithFile() {
        CliArgs args = parser.parse(new String[]{"put", "program.bas"});
        assertNotNull(args);
        assertEquals("put", args.verb());
        assertEquals("program.bas", args.file());
        assertEquals(PocketPcDevice.PC1500, args.device());
        assertNull(args.format()); // auto-detect for put
        assertFalse(args.addUtils());
    }

    @Test
    @DisplayName("get -d pc1600 sets PC1600 device")
    void getWithPc1600Device() {
        CliArgs args = parser.parse(new String[]{"get", "-d", "pc1600", "output.bin"});
        assertNotNull(args);
        assertEquals(PocketPcDevice.PC1600, args.device());
    }

    @Test
    @DisplayName("get --device pc1500a sets PC1500A device")
    void getWithPc1500aDevice() {
        CliArgs args = parser.parse(new String[]{"get", "--device", "pc1500a", "output.bin"});
        assertNotNull(args);
        assertEquals(PocketPcDevice.PC1500A, args.device());
    }

    @Test
    @DisplayName("get -f binary sets BINARY format")
    void getWithBinaryFormat() {
        CliArgs args = parser.parse(new String[]{"get", "-f", "binary", "output.bin"});
        assertNotNull(args);
        assertEquals(OutputFormat.BINARY, args.format());
    }

    @Test
    @DisplayName("get -p /dev/ttyUSB0 sets port")
    void getWithPort() {
        CliArgs args = parser.parse(new String[]{"get", "-p", "/dev/ttyUSB0", "output.bas"});
        assertNotNull(args);
        assertEquals("/dev/ttyUSB0", args.port());
    }

    @Test
    @DisplayName("put --start-address 38C5 sets start address from bare hex")
    void putWithStartAddress() {
        CliArgs args = parser.parse(new String[]{"put", "--start-address", "38C5", "prog.bin"});
        assertNotNull(args);
        assertEquals(0x38C5, args.startAddress());
        assertEquals(0xFFFF, args.runAddress()); // default when start-address given
    }

    @Test
    @DisplayName("put --start-address 0x38C5 parses 0x-prefixed hex")
    void putWithStartAddress0x() {
        CliArgs args = parser.parse(new String[]{"put", "--start-address", "0x38C5", "prog.bin"});
        assertNotNull(args);
        assertEquals(0x38C5, args.startAddress());
    }

    @Test
    @DisplayName("put with explicit --run-address uses that value")
    void putWithRunAddress() {
        CliArgs args = parser.parse(
                new String[]{"put", "--start-address", "38C5", "--run-address", "38C5", "prog.bin"});
        assertNotNull(args);
        assertEquals(0x38C5, args.startAddress());
        assertEquals(0x38C5, args.runAddress());
    }

    @Test
    @DisplayName("put --add-utils sets addUtils=true")
    void putWithAddUtils() {
        CliArgs args = parser.parse(new String[]{"put", "--add-utils", "prog.bas"});
        assertNotNull(args);
        assertTrue(args.addUtils());
    }

    @Test
    @DisplayName("put -u sets addUtils=true (short form)")
    void putWithAddUtilsShort() {
        CliArgs args = parser.parse(new String[]{"put", "-u", "prog.bas"});
        assertNotNull(args);
        assertTrue(args.addUtils());
    }

    @Test
    @DisplayName("convert with input file returns correct CliArgs")
    void convertWithInputFile() {
        CliArgs args = parser.parse(new String[]{"convert", "program.bas"});
        assertNotNull(args);
        assertEquals("convert", args.verb());
        assertEquals("program.bas", args.file());
        assertNull(args.outputFile());
        assertEquals(PocketPcDevice.PC1500, args.device());
        assertNull(args.format());
    }

    @Test
    @DisplayName("convert with input and output file sets outputFile")
    void convertWithOutputFile() {
        CliArgs args = parser.parse(new String[]{"convert", "in.bas", "out.bas"});
        assertNotNull(args);
        assertEquals("in.bas", args.file());
        assertEquals("out.bas", args.outputFile());
    }

    @Test
    @DisplayName("convert -d pc1600 sets PC1600 device")
    void convertWithPc1600Device() {
        CliArgs args = parser.parse(new String[]{"convert", "-d", "pc1600", "in.bas"});
        assertNotNull(args);
        assertEquals(PocketPcDevice.PC1600, args.device());
    }

    @Test
    @DisplayName("convert rejects --format")
    void convertRejectsFormat() {
        assertNull(parser.parse(new String[]{"convert", "-f", "ascii", "in.bas"}));
    }

    // ---- No-op returns (help / version) ----

    @Test
    @DisplayName("-h returns null (help displayed)")
    void helpFlag() {
        assertNull(parser.parse(new String[]{"-h"}));
    }

    @Test
    @DisplayName("--help returns null (help displayed)")
    void helpFlagLong() {
        assertNull(parser.parse(new String[]{"--help"}));
    }

    @Test
    @DisplayName("-V returns null (version displayed)")
    void versionFlag() {
        assertNull(parser.parse(new String[]{"-V"}));
    }

    @Test
    @DisplayName("--version returns null (version displayed)")
    void versionFlagLong() {
        assertNull(parser.parse(new String[]{"--version"}));
    }

    // ---- Error cases ----

    @Test
    @DisplayName("No args returns null")
    void noArgs() {
        assertNull(parser.parse(new String[]{}));
    }

    @Test
    @DisplayName("Unknown verb returns null")
    void unknownVerb() {
        assertNull(parser.parse(new String[]{"send", "file.bas"}));
    }

    @Test
    @DisplayName("get without file is allowed")
    void getWithoutFile() {
        CliArgs args = parser.parse(new String[]{"get"});
        assertNotNull(args);
        assertEquals("get", args.verb());
        assertNull(args.file());
    }

    @Test
    @DisplayName("terminal without file is allowed")
    void terminalWithoutFile() {
        CliArgs args = parser.parse(new String[]{"terminal"});
        assertNotNull(args);
        assertEquals("terminal", args.verb());
        assertNull(args.file());
    }

    @Test
    @DisplayName("terminal rejects --format")
    void terminalRejectsFormat() {
        assertNull(parser.parse(new String[]{"terminal", "-f", "ascii"}));
    }

    @Test
    @DisplayName("put without file returns null")
    void putWithoutFile() {
        assertNull(parser.parse(new String[]{"put"}));
    }

    @Test
    @DisplayName("convert without file returns null")
    void convertWithoutFile() {
        assertNull(parser.parse(new String[]{"convert"}));
    }

    @Test
    @DisplayName("Unknown device returns null")
    void unknownDevice() {
        assertNull(parser.parse(new String[]{"get", "-d", "pc9000", "out.bas"}));
    }

    @Test
    @DisplayName("Unknown format returns null")
    void unknownFormat() {
        assertNull(parser.parse(new String[]{"get", "-f", "xml", "out.bas"}));
    }

    @Test
    @DisplayName("--run-address without --start-address returns null")
    void runAddressWithoutStartAddress() {
        assertNull(parser.parse(new String[]{"put", "--run-address", "38C5", "prog.bin"}));
    }

    @Test
    @DisplayName("Invalid start address returns null")
    void invalidStartAddress() {
        assertNull(parser.parse(new String[]{"put", "--start-address", "ZZZZ", "prog.bin"}));
    }

    // ---- parseHexInt corner cases ----

    @Test
    @DisplayName("parseHexInt handles decimal input")
    void parseHexIntDecimal() {
        assertEquals(12345, parser.parseHexInt("12345"));
    }

    @Test
    @DisplayName("parseHexInt handles 0x prefix")
    void parseHexInt0x() {
        assertEquals(0x38C5, parser.parseHexInt("0x38C5"));
    }

    @Test
    @DisplayName("parseHexInt handles $ prefix")
    void parseHexIntDollar() {
        assertEquals(0x38C5, parser.parseHexInt("$38C5"));
    }

    @Test
    @DisplayName("parseHexInt handles bare hex with A-F chars")
    void parseHexIntBare() {
        assertEquals(0x38C5, parser.parseHexInt("38C5"));
    }

    @Test
    @DisplayName("parseHexInt returns null for invalid input")
    void parseHexIntInvalid() {
        assertNull(parser.parseHexInt("ZZZZ"));
    }

    @Test
    @DisplayName("parseHexInt returns null for null input")
    void parseHexIntNull() {
        assertNull(parser.parseHexInt(null));
    }
}
