package ch.erzberger.sharppc.exchange;

import ch.erzberger.sharpbasic.antlr.BinaryBasicDetokenizer;
import ch.erzberger.sharpbasic.core.keyword.KeywordRegistry;
import ch.erzberger.sharppc.exchange.cli.CliArgs;
import ch.erzberger.sharppc.exchange.cli.CliParser;
import ch.erzberger.sharppc.exchange.cli.OutputFormat;
import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import ch.erzberger.sharppc.exchange.convert.*;
import ch.erzberger.sharppc.exchange.detect.ContentDetector;
import ch.erzberger.sharppc.exchange.header.SerialHeader;
import ch.erzberger.sharppc.exchange.io.FileHandler;
import ch.erzberger.sharppc.exchange.serial.DataReceiver;
import ch.erzberger.sharppc.exchange.serial.DataSender;
import ch.erzberger.sharppc.exchange.serial.SerialPortWrapper;
import lombok.extern.java.Log;

import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.List;
import java.util.logging.Level;
import java.util.logging.LogManager;

/**
 * Entry point and orchestrator for SharpDataExchange.
 *
 * <p>Two-step data flow:
 * <ol>
 *   <li>Read all raw bytes (from serial or disk)</li>
 *   <li>Detect type → convert → write (to disk or serial)</li>
 * </ol>
 */
@Log
public class SharpDataExchange {

    /** CE-158 header size in bytes. */
    private static final int CE158_HEADER_SIZE = 27;

    public static void main(String[] args) {
        try {
            LogManager.getLogManager().readConfiguration(
                    SharpDataExchange.class.getResourceAsStream("/logging.properties"));
        } catch (Exception e) {
            System.err.println("Warning: could not load logging configuration");
        }

        CliArgs cliArgs = new CliParser().parse(args);
        if (cliArgs == null) {
            System.exit(1);
        }

        try {
            if ("get".equals(cliArgs.verb())) {
                runGet(cliArgs);
            } else {
                runPut(cliArgs);
            }
        } catch (Exception e) {
            log.log(Level.SEVERE, "Fatal error: {0}", e.getMessage());
            System.exit(1);
        }
    }

    // ---- get path ----

    private static void runGet(CliArgs args) {
        // Step 1: receive all bytes from the Pocket Computer
        DataReceiver receiver = new DataReceiver(args.device());
        SerialPortWrapper port = openPort(args.device(), args.port(), receiver);
        byte[] rawData = receiver.getDataWhenReady();
        port.closePort();
        log.log(Level.FINE, "Received {0} bytes", rawData.length);

        // Step 2: detect → convert → write
        int headerOffset = findHeaderOffset(rawData);
        if (headerOffset < 0) {
            log.log(Level.SEVERE, "No recognizable header in received data");
            System.exit(1);
        }

        SerialHeader header = SerialHeader.makeHeader(sliceFrom(rawData, headerOffset));
        if (header == null) {
            log.log(Level.SEVERE, "Could not parse header");
            System.exit(1);
        }

        String filename = header.getFilename();
        if (filename != null && !filename.isBlank()) {
            System.out.println("Getting " + filename);
        }
        byte[] payload = sliceFrom(rawData, headerOffset + CE158_HEADER_SIZE);

        DataType type = new ContentDetector().detect(sliceFrom(rawData, headerOffset));
        log.log(Level.FINE, "Detected type: {0}", type);

        switch (type) {
            case BINARY_BASIC -> getBinaryBasic(payload, args.format(), args.device(), args.file());
            case BINARY_RESERVE -> {
                String sdar = ReserveAreaConverter.toAscii(payload, filename, args.device());
                FileHandler.writeText(args.file(), sdar);
            }
            case BINARY_VARS -> {
                String sdav = VariablesConverter.toAscii(payload, filename, args.device());
                FileHandler.writeText(args.file(), sdav);
            }
            case ASCII_BASIC -> {
                String text = new String(rawData, StandardCharsets.US_ASCII);
                FileHandler.writeText(args.file(), text);
            }
            case MACHINE -> FileHandler.writeBinary(args.file(), sliceFrom(rawData, headerOffset));
            default -> {
                log.log(Level.SEVERE, "Unsupported data type for get: {0}", type);
                System.exit(1);
            }
        }
        log.log(Level.FINE, "Written to: {0}", args.file());
    }

    private static void getBinaryBasic(byte[] payload, OutputFormat format, PocketPcDevice device, String outFile) {
        if (OutputFormat.BINARY.equals(format)) {
            // Write payload as binary — caller should re-attach header if needed
            FileHandler.writeBinary(outFile, payload);
            return;
        }
        // ASCII / ASCIICOMPACT: detokenize
        KeywordRegistry registry = device.isPC1600()
                ? KeywordRegistry.forPc1600()
                : KeywordRegistry.forPc1500();
        List<String> lines = new BinaryBasicDetokenizer(registry).detokenize(payload);
        FileHandler.writeText(outFile, String.join("\n", lines) + "\n");
    }

    // ---- put path ----

    private static void runPut(CliArgs args) {
        // Step 1: read file
        byte[] rawData = FileHandler.readBinaryFile(args.file());
        if (rawData.length == 0) {
            log.log(Level.SEVERE, "Input file is empty or could not be read: {0}", args.file());
            System.exit(1);
        }

        // Step 2: detect → convert → build fully-formed data block
        int headerOffset = findHeaderOffset(rawData);
        DataType type = new ContentDetector().detect(rawData);
        log.log(Level.FINE, "Detected input type: {0}", type);

        String putFilename = deriveFilename(args.file());
        System.out.println("Putting " + putFilename);

        byte[] dataToSend = switch (type) {
            case ASCII_BASIC -> encodeAsciiBasic(rawData, args);
            case ASCII_RESERVE -> encodeAsciiReserve(rawData, args);
            case ASCII_VARS -> encodeAsciiVars(rawData, args);
            case BINARY_BASIC, BINARY_RESERVE, BINARY_VARS -> {
                // Already has a header — send as-is
                yield headerOffset >= 0 ? sliceFrom(rawData, headerOffset) : rawData;
            }
            case MACHINE -> SerialHeader.prependHeaderIfNecessary(
                    rawData, args.startAddress(), args.runAddress(), args.device(), putFilename);
            default -> {
                log.log(Level.SEVERE, "Cannot send data of type {0}", type);
                System.exit(1);
                yield new byte[0];
            }
        };

        // Step 3: send
        SerialPortWrapper port = openPort(args.device(), args.port(), null);
        DataSender sender = new DataSender(port, args.device());
        sender.sendData(dataToSend);
        port.closePort();
        log.log(Level.FINE, "Sent {0} bytes", dataToSend.length);
    }

    private static byte[] encodeAsciiBasic(byte[] rawData, CliArgs args) {
        String text = new String(rawData, StandardCharsets.UTF_8);
        if (args.addUtils()) {
            text = text + loadUtilBasic(args.device());
        }
        byte[] payload = new AsciiBasicTokenizer(args.device()).tokenize(text);
        String filename = deriveFilename(args.file());
        SerialHeader header = SerialHeader.makeHeader(args.device(), SerialHeader.FileType.BASIC,
                filename, 0, payload.length, 0);
        return concat(header.getHeader(), payload);
    }

    private static byte[] encodeAsciiReserve(byte[] rawData, CliArgs args) {
        String text = new String(rawData, StandardCharsets.UTF_8);
        byte[] payload = ReserveAreaConverter.toBinary(text, args.device());
        String filename = deriveFilename(args.file());
        SerialHeader header = SerialHeader.makeHeader(args.device(), SerialHeader.FileType.RESERVE,
                filename, 0, payload.length, 0);
        return concat(header.getHeader(), payload);
    }

    private static byte[] encodeAsciiVars(byte[] rawData, CliArgs args) {
        String text = new String(rawData, StandardCharsets.UTF_8);
        byte[] payload = VariablesConverter.toBinary(text, args.device());
        String filename = deriveFilename(args.file());
        // CE-158 VARIABLES header length field is always 1 (wire: 0x0000 — meaningless)
        SerialHeader header = SerialHeader.makeHeader(args.device(), SerialHeader.FileType.VARIABLES,
                filename, 0, 1, 0);
        return concat(header.getHeader(), payload);
    }

    /**
     * Load the serial utility BASIC sub-program for the given device from resources.
     * Returns an empty string on failure (caller proceeds without utilities).
     */
    private static String loadUtilBasic(PocketPcDevice device) {
        String resourceName = device.isPC1600() ? "/setcom1600.bas" : "/setcom1500.bas";
        try (java.io.InputStream in = SharpDataExchange.class.getResourceAsStream(resourceName)) {
            if (in == null) {
                log.log(Level.SEVERE, "Could not find resource: {0}", resourceName);
                return "";
            }
            return "\n" + new String(in.readAllBytes(), StandardCharsets.US_ASCII);
        } catch (java.io.IOException e) {
            log.log(Level.SEVERE, "Failed to read resource: {0}", resourceName);
            return "";
        }
    }

    // ---- Serial port helpers ----

    /**
     * Open the serial port configured for the given device.
     *
     * @param device       target device (determines baud rate and flow control)
     * @param portName     explicit port name, or null/empty for auto-detection
     * @param receiver     non-null to open for reading, null to open for writing
     * @return configured open port
     */
    private static SerialPortWrapper openPort(PocketPcDevice device, String portName, DataReceiver receiver) {
        SerialPortWrapper port = new SerialPortWrapper(portName);
        int baudRate = device.isPC1500() ? 19200 : 9600;
        boolean handShake = device.isPC1600();
        if (receiver != null) {
            port.openPort(baudRate, handShake, receiver);
        } else {
            port.openPort(baudRate, handShake);
        }
        return port;
    }

    // ---- Byte-array helpers ----

    /**
     * Scan {@code data} for CE-158 or PC-1600 magic bytes, starting from offset 0.
     * Returns the offset of the header, or -1 if not found.
     * Handles capture artifacts (leading 0x00 bytes).
     */
    static int findHeaderOffset(byte[] data) {
        for (int i = 0; i < data.length - 4; i++) {
            // CE-158 magic: 0x01 at i, 'C' 'O' 'M' at i+2..4
            if (data[i] == 0x01
                    && i + 4 < data.length
                    && data[i + 2] == 0x43 && data[i + 3] == 0x4F && data[i + 4] == 0x4D) {
                return i;
            }
            // PC-1600 magic: 0xFF 0x10 0x00 0x00 at i
            if ((data[i] & 0xFF) == 0xFF && i + 3 < data.length
                    && data[i + 1] == 0x10 && data[i + 2] == 0x00 && data[i + 3] == 0x00) {
                return i;
            }
        }
        return -1;
    }

    private static byte[] sliceFrom(byte[] data, int offset) {
        if (offset <= 0) return data;
        if (offset >= data.length) return new byte[0];
        byte[] result = new byte[data.length - offset];
        System.arraycopy(data, offset, result, 0, result.length);
        return result;
    }

    private static byte[] concat(byte[] a, byte[] b) {
        byte[] result = new byte[a.length + b.length];
        System.arraycopy(a, 0, result, 0, a.length);
        System.arraycopy(b, 0, result, a.length, b.length);
        return result;
    }

    /**
     * Derive a CE-158 filename from a file path: basename without extension, uppercase, max 16 chars.
     */
    static String deriveFilename(String filePath) {
        String name = Path.of(filePath).getFileName().toString();
        int dot = name.lastIndexOf('.');
        String base = dot > 0 ? name.substring(0, dot) : name;
        base = base.toUpperCase();
        return base.length() > 16 ? base.substring(0, 16) : base;
    }
}
