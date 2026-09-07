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
import ch.erzberger.sharppc.exchange.io.SharpText;
import ch.erzberger.sharppc.exchange.serial.DataReceiver;
import ch.erzberger.sharppc.exchange.serial.DataSender;
import ch.erzberger.sharppc.exchange.serial.SerialPortWrapper;
import lombok.extern.java.Log;

import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.List;
import java.util.Locale;
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
            } else if ("put".equals(cliArgs.verb())) {
                runPut(cliArgs);
            } else if ("convert".equals(cliArgs.verb())) {
                runConvert(cliArgs);
            } else if ("terminal".equals(cliArgs.verb())) {
                runTerminal(cliArgs);
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
        byte[] rawData;
        try (SerialPortWrapper port = openPort(args.device(), args.port(), (ch.erzberger.sharppc.exchange.serial.ByteProcessor) receiver)) {
            rawData = receiver.getDataWhenReady();
        }
        log.log(Level.FINE, "Received {0} bytes", rawData.length);

        // Step 2: detect → convert → write
        int headerOffset = findHeaderOffset(rawData);
        SerialHeader header = null;
        if (headerOffset >= 0) {
            header = SerialHeader.makeHeader(sliceFrom(rawData, headerOffset));
        }

        if (header == null) {
            System.err.println("WARNING: No recognizable header in received data");
        }

        String filename = header != null ? header.getFilename() : null;
        byte[] withHeader = headerOffset >= 0 ? sliceFrom(rawData, headerOffset) : rawData;
        byte[] payload = (header != null && headerOffset >= 0)
                ? sliceFrom(rawData, headerOffset + header.getHeader().length)
                : rawData;

        // Use the device identified in the header for decoding — more reliable than --device.
        // Fall back to CLI arg if header is missing.
        PocketPcDevice detectedDevice = header != null ? header.getDevice() : args.device();

        DataType type = new ContentDetector().detect(withHeader);
        log.log(Level.FINE, "Detected type: {0}, device: {1}", new Object[]{type, detectedDevice});

        if (args.skipHeader() && OutputFormat.BINARY.equals(args.format())) {
            System.err.println("WARNING: --skip-header omits the serial header from the saved file." +
                    " The file cannot be identified or reloaded by SharpDataExchange without it.");
        }

        String finalFile = args.file();
        if (finalFile == null || finalFile.isEmpty()) {
            if (filename != null && !filename.isBlank()) {
                finalFile = filename;
                finalFile = appendExtension(finalFile, type);
                System.out.println("Saving to " + finalFile);
            } else {
                finalFile = "unnamed";
                finalFile = appendExtension(finalFile, type);
                System.err.println("WARNING: No filename provided on command line or in header, saving to " + finalFile);
            }
        } else {
            if (!finalFile.contains(".")) {
                finalFile = appendExtension(finalFile, type);
            }
            System.out.println("Saving to " + finalFile);
        }

        switch (type) {
            case BINARY_BASIC -> {
                if (OutputFormat.BINARY.equals(args.format())) {
                    getBinaryBasic(args.skipHeader() ? payload : withHeader, args.format(), detectedDevice, finalFile);
                } else {
                    if (header == null) {
                        log.log(Level.SEVERE, "Cannot detokenize BASIC without a header");
                        System.exit(1);
                    }
                    getBinaryBasic(payload, args.format(), detectedDevice, finalFile);
                }
            }
            case BINARY_RESERVE -> {
                if (OutputFormat.BINARY.equals(args.format())) {
                    FileHandler.writeBinary(finalFile, args.skipHeader() ? payload : withHeader);
                } else {
                    String sdar = ReserveAreaConverter.toAscii(payload, filename, detectedDevice);
                    FileHandler.writeText(finalFile, sdar);
                }
            }
            case BINARY_VARS -> {
                if (OutputFormat.BINARY.equals(args.format())) {
                    FileHandler.writeBinary(finalFile, args.skipHeader() ? payload : withHeader);
                } else {
                    String sdav = VariablesConverter.toAscii(payload, filename, detectedDevice);
                    FileHandler.writeText(finalFile, sdav);
                }
            }
            case ASCII_BASIC -> FileHandler.writeText(finalFile, SharpText.cleanListing(rawData));
            case MACHINE, UNKNOWN -> FileHandler.writeBinary(finalFile,
                    (args.skipHeader() || header == null) ? payload : withHeader);
            default -> {
                log.log(Level.SEVERE, "Unsupported data type for get: {0}", type);
                System.exit(1);
            }
        }
        log.log(Level.FINE, "Written to: {0}", finalFile);
    }

    private static String appendExtension(String filename, DataType type) {
        String ext = switch (type) {
            case BINARY_BASIC, ASCII_BASIC -> ".bas";
            case BINARY_RESERVE, ASCII_RESERVE -> ".sdar";
            case BINARY_VARS, ASCII_VARS -> ".sdav";
            case MACHINE -> ".bin";
            default -> "";
        };
        return filename + ext;
    }

    private static void getBinaryBasic(byte[] payload, OutputFormat format, PocketPcDevice device, String outFile) {
        if (OutputFormat.BINARY.equals(format)) {
            FileHandler.writeBinary(outFile, payload);
            return;
        }
        // ASCII: detokenize
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

        // If a start address is given, the input is a raw machine language binary.
        // Trust the user over content detection: skip the header check and treat as MACHINE.
        // The --format flag is also irrelevant for machine language (always binary).
        if (args.startAddress() != null) {
            if (type != DataType.MACHINE) {
                log.log(Level.FINE, "Start address given; overriding detected type {0} → MACHINE", type);
            }
            type = DataType.MACHINE;
            if (args.format() != null) {
                log.log(Level.FINE, "--format is ignored for machine language programs");
            }
        }

        // For binary files with a recognizable header, infer the target device from the header.
        // --device is then optional and only required for ASCII input or headerless machine code.
        PocketPcDevice effectiveDevice = args.device();
        if (headerOffset >= 0) {
            SerialHeader existingHeader = SerialHeader.makeHeader(sliceFrom(rawData, headerOffset));
            if (existingHeader != null && existingHeader.getDevice() != null) {
                PocketPcDevice inferred = existingHeader.getDevice();
                // The header only records the hardware model (PC-1600), not that the link
                // is an emulator. Keep an explicit --device pc1600emul so the transport
                // still runs without hardware flow control and with send pacing.
                if (inferred.isPC1600() && args.device().isEmulator()) {
                    inferred = PocketPcDevice.PC1600EMUL;
                }
                effectiveDevice = inferred;
                log.log(Level.FINE, "Device inferred from binary header: {0}", effectiveDevice);
            }
        }

        String putFilename = deriveFilename(args.file());
        System.out.println("Putting " + putFilename);
        if (type == DataType.MACHINE) {
            reportMachineHeaderInfo(rawData, headerOffset, args, effectiveDevice);
        }

        byte[] dataToSend = switch (type) {
            case ASCII_BASIC -> encodeAsciiBasic(rawData, args);
            case ASCII_RESERVE -> encodeAsciiReserve(rawData, args);
            case ASCII_VARS -> encodeAsciiVars(rawData, args);
            case BINARY_BASIC, BINARY_RESERVE, BINARY_VARS -> {
                // Already has a header — send as-is
                yield headerOffset >= 0 ? sliceFrom(rawData, headerOffset) : rawData;
            }
            case MACHINE -> SerialHeader.prependHeaderIfNecessary(
                    rawData, args.startAddress(), args.runAddress(), effectiveDevice, putFilename);
            default -> {
                log.log(Level.SEVERE, "Cannot send data of type {0}", type);
                System.exit(1);
                yield new byte[0];
            }
        };

        // Step 3: send (or write to disk for --dry-run)
        if (args.dryRunFile() != null) {
            FileHandler.writeBinary(args.dryRunFile(), dataToSend);
            System.out.println("Dry run: wrote " + dataToSend.length + " bytes to " + args.dryRunFile()
                    + " (nothing sent over serial)");
            return;
        }
        try (SerialPortWrapper port = openPort(effectiveDevice, args.port(), (ch.erzberger.sharppc.exchange.serial.ByteProcessor) null)) {
            DataSender sender = new DataSender(port, effectiveDevice);
            sender.sendData(dataToSend);
        }
        log.log(Level.FINE, "Sent {0} bytes", dataToSend.length);
    }

    /**
     * Print, for a machine-language transfer, whether a header is already present, will be
     * added, or is being omitted entirely — plus the load and run (auto-run) addresses in
     * play, so the user can confirm what is actually being sent to the device.
     */
    private static void reportMachineHeaderInfo(byte[] rawData, int headerOffset, CliArgs args,
                                                  PocketPcDevice effectiveDevice) {
        String addrFmt = effectiveDevice.isPC1600() ? "0x%06X" : "0x%04X";
        if (headerOffset >= 0) {
            SerialHeader existing = SerialHeader.makeHeader(sliceFrom(rawData, headerOffset));
            System.out.println("Header: already present in file (device=" + effectiveDevice + ") — sending as-is");
            System.out.println("  Load address: " + String.format(addrFmt, existing.getStartAddr()));
            System.out.println("  Run address:  " + String.format(addrFmt, existing.getRunAddr())
                    + (existing.getRunAddr() == 0 ? " (no auto-run)" : ""));
            if (args.startAddress() != null) {
                System.out.println("  Note: --start-address/--run-address ignored — file already has a header");
            }
        } else if (args.startAddress() != null) {
            int runAddr = args.runAddress() != null ? args.runAddress() : 0xFFFF;
            System.out.println("Header: adding new header (device=" + effectiveDevice + ")");
            System.out.println("  Load address: " + String.format(addrFmt, args.startAddress()));
            System.out.println("  Run address:  " + String.format(addrFmt, runAddr)
                    + (args.runAddress() == null ? " (default — no --run-address given)" : ""));
        } else {
            System.out.println("Header: none added — sending raw machine code as-is (no --start-address given)");
        }
    }

    private static byte[] encodeAsciiBasic(byte[] rawData, CliArgs args) {
        // Accept both a modern UTF-8 listing and a raw Sharp listing saved with SAVE ...,A
        // (CP437 bytes). The tokenizer maps the resulting text back to CP437 on the wire.
        String text = SharpText.decodeBasListing(rawData);
        if (args.device() != null && args.device().isPC1500()) {
            rejectNonAsciiForPc1500(text, args.file());
        }
        if (args.addUtils()) {
            text = text + loadUtilBasic(args.device());
        }
        byte[] payload = new AsciiBasicTokenizer(args.device()).tokenize(text);
        String filename = deriveFilename(args.file());
        SerialHeader header = SerialHeader.makeHeader(args.device(), SerialHeader.FileType.BASIC,
                filename, 0, payload.length, 0);
        return concat(header.getHeader(), payload);
    }

    /**
     * The PC-1500/1500A character set is 7-bit ASCII (with a few glyph substitutions in the
     * 0x5B–0x7F range); it has no CP437 upper half. A listing bound for a PC-1500 that contains
     * any character above 0x7F — an umlaut, say — cannot be represented on the device, so abort
     * with a pointer to the offending line rather than silently mangling it.
     */
    private static void rejectNonAsciiForPc1500(String text, String file) {
        int[] hit = SharpText.firstNonAsciiForPc1500(text);
        if (hit != null) {
            log.log(Level.SEVERE, "{0}: line {1}, column {2}: character ''{3}'' (U+{4}) is not "
                            + "representable on the PC-1500 — its character set is 7-bit ASCII. "
                            + "Use plain ASCII, or target the PC-1600 with --device pc1600.",
                    new Object[]{file, hit[0], hit[1], (char) hit[2],
                            String.format("%04X", hit[2])});
            System.exit(1);
        }
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

    // ---- convert path ----

    /** Extension for ASCII BASIC listings. */
    private static final String ASCII_BASIC_EXT = ".bas";
    /** Extension for tokenized BASIC (payload wrapped in a CE-158 / PC-1600 serial header). */
    private static final String TOKENIZED_BASIC_EXT = ".bbin";

    /**
     * Offline tokenize / de-tokenize of a single BASIC file. Direction is chosen from the
     * file content, not its name: valid ASCII BASIC is tokenized (payload wrapped in a
     * CE-158/PC-1600 serial header), a header-carrying tokenized program is de-tokenized to
     * ASCII. Anything else is rejected. The input extension must agree with the content
     * ({@code .bas} = ASCII, {@code .bbin} = tokenized); a mismatch is rejected.
     */
    private static void runConvert(CliArgs args) {
        String inputFile = appendBasIfMissing(args.file());
        byte[] rawData = FileHandler.readBinaryFile(inputFile);
        if (rawData.length == 0) {
            log.log(Level.SEVERE, "Input file is empty or could not be read: {0}", inputFile);
            System.exit(1);
        }

        int headerOffset = findHeaderOffset(rawData);
        DataType type = new ContentDetector().detect(rawData);
        log.log(Level.FINE, "Detected input type: {0}", type);

        checkExtensionMatchesContent(inputFile, type);

        switch (type) {
            case ASCII_BASIC -> {
                String outFile = deriveConvertOutput(args.outputFile(), inputFile, TOKENIZED_BASIC_EXT);
                byte[] block = encodeAsciiBasic(rawData, args);
                FileHandler.writeBinary(outFile, block);
                System.out.println("Converted " + inputFile + " -> " + outFile + " (tokenized, " + args.device() + ")");
            }
            case BINARY_BASIC -> {
                SerialHeader header = headerOffset >= 0
                        ? SerialHeader.makeHeader(sliceFrom(rawData, headerOffset))
                        : null;
                if (header == null) {
                    log.log(Level.SEVERE, "Tokenized BASIC input must carry a CE-158 or PC-1600 header: {0}",
                            inputFile);
                    System.exit(1);
                }
                byte[] payload = sliceFrom(rawData, headerOffset + header.getHeader().length);
                PocketPcDevice device = header.getDevice() != null ? header.getDevice() : args.device();
                String outFile = deriveConvertOutput(args.outputFile(), inputFile, ASCII_BASIC_EXT);
                getBinaryBasic(payload, OutputFormat.ASCII, device, outFile);
                System.out.println("Converted " + inputFile + " -> " + outFile + " (ASCII, " + device + ")");
            }
            default -> {
                log.log(Level.SEVERE, "convert only handles BASIC; got {0}. A tokenized file must include a "
                        + "CE-158 or PC-1600 header.", type);
                System.exit(1);
            }
        }
    }

    /**
     * Append ".bas" when the file name (last path segment) has no extension.
     */
    private static String appendBasIfMissing(String path) {
        if (path == null || path.isEmpty()) {
            return path;
        }
        String name = Path.of(path).getFileName().toString();
        return name.contains(".") ? path : path + ASCII_BASIC_EXT;
    }

    /**
     * Reject an input whose extension contradicts its actual content: {@code .bas} is for ASCII
     * BASIC listings only, {@code .bbin} is for tokenized BASIC only. Any other extension is left
     * unconstrained.
     */
    private static void checkExtensionMatchesContent(String inputFile, DataType type) {
        String lower = inputFile.toLowerCase(Locale.ROOT);
        if (lower.endsWith(TOKENIZED_BASIC_EXT) && type != DataType.BINARY_BASIC) {
            log.log(Level.SEVERE, "{0} has a {1} extension (tokenized BASIC) but its content is {2}. "
                            + "ASCII BASIC listings must use {3}.",
                    new Object[]{inputFile, TOKENIZED_BASIC_EXT, type, ASCII_BASIC_EXT});
            System.exit(1);
        }
        if (lower.endsWith(ASCII_BASIC_EXT) && type != DataType.ASCII_BASIC) {
            log.log(Level.SEVERE, "{0} has a {1} extension (ASCII BASIC) but its content is {2}. "
                            + "Tokenized BASIC must use {3}.",
                    new Object[]{inputFile, ASCII_BASIC_EXT, type, TOKENIZED_BASIC_EXT});
            System.exit(1);
        }
    }

    /**
     * Resolve the output path for {@code convert}. {@code newExt} is the extension required for
     * this conversion direction ({@code .bas} for a de-tokenized listing, {@code .bbin} for a
     * tokenized program). When {@code givenOutput} is set it is used verbatim; a missing
     * extension gets {@code newExt}, and a {@code .bas}/{@code .bbin} extension that contradicts
     * {@code newExt} is rejected. Otherwise the input file's base name is reused with
     * {@code newExt}, placed next to the input file.
     */
    private static String deriveConvertOutput(String givenOutput, String inputFile, String newExt) {
        if (givenOutput != null && !givenOutput.isEmpty()) {
            String name = Path.of(givenOutput).getFileName().toString();
            if (!name.contains(".")) {
                return givenOutput + newExt;
            }
            String lower = givenOutput.toLowerCase(Locale.ROOT);
            String wrongExt = newExt.equals(ASCII_BASIC_EXT) ? TOKENIZED_BASIC_EXT : ASCII_BASIC_EXT;
            if (lower.endsWith(wrongExt)) {
                log.log(Level.SEVERE, "Output file {0} has a {1} extension, but this conversion "
                        + "produces {2}.", new Object[]{givenOutput, wrongExt, newExt});
                System.exit(1);
            }
            return givenOutput;
        }
        Path in = Path.of(inputFile);
        String name = in.getFileName().toString();
        int dot = name.lastIndexOf('.');
        String base = dot > 0 ? name.substring(0, dot) : name;
        String outName = base + newExt;
        Path parent = in.getParent();
        return parent != null ? parent.resolve(outName).toString() : outName;
    }

    // ---- terminal path ----

    private static void runTerminal(CliArgs args) {
        System.out.println("Terminal mode active (" + args.device() + "). Press ESC twice to exit.");

        try (SerialPortWrapper port = openPort(args.device(), args.port(), new ch.erzberger.sharppc.exchange.serial.ByteProcessor() {
            private boolean lastWasHex = false;

            @Override
            public void processByte(byte b) {
                int val = b & 0xFF;
                // Printable ASCII (excluding 127) or line breaks
                if ((val >= 32 && val <= 126) || val == 10 || val == 13) {
                    if (lastWasHex) {
                        System.out.print(" ");
                    }
                    System.out.print((char) val);
                    lastWasHex = false;
                } else {
                    System.out.printf(" 0x%02X", val);
                    lastWasHex = true;
                }
                System.out.flush();
            }
        })) {
            int escapeCount = 0;
            long lastEscapeTime = 0;
            while (true) {
                if (System.in.available() > 0) {
                    int c = System.in.read();
                    if (c == 27) { // ESC
                        long now = System.currentTimeMillis();
                        if (escapeCount > 0 && (now - lastEscapeTime) < 500) {
                            break; // Double ESC within 500ms
                        }
                        escapeCount = 1;
                        lastEscapeTime = now;
                    } else {
                        escapeCount = 0;
                        port.writeBytes(new byte[]{(byte) c});
                    }
                } else {
                    Thread.sleep(10);
                }
            }
        } catch (Exception e) {
            log.log(Level.SEVERE, "Terminal error: {0}", e.getMessage());
        }
        System.out.println("\nTerminal mode closed.");
    }

    // ---- Serial port helpers ----

    /**
     * Open the serial port configured for the given device.
     *
     * @param device       target device (determines baud rate and flow control)
     * @param portName     explicit port name, or null/empty for auto-detection
     * @param byteProcessor non-null to open for reading, null to open for writing
     * @return configured open port
     */
    private static SerialPortWrapper openPort(PocketPcDevice device, String portName, ch.erzberger.sharppc.exchange.serial.ByteProcessor byteProcessor) {
        SerialPortWrapper port = new SerialPortWrapper(portName);
        int baudRate = device.isPC1500() ? 19200 : 9600;
        // Only real PC-1600 hardware has RTS/CTS lines; the emulator pseudo-terminal
        // has none, so it runs without hardware handshake and relies on send pacing.
        boolean handShake = device.hasHardwareFlowControl();
        if (byteProcessor != null) {
            port.openPort(baudRate, handShake, byteProcessor);
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
