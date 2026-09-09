package ch.erzberger.sharppc.exchange.cli;

import lombok.extern.java.Log;
import org.apache.commons.cli.*;

import java.io.IOException;
import java.util.Arrays;
import java.util.jar.Attributes;
import java.util.jar.Manifest;
import java.util.logging.Handler;
import java.util.logging.Level;
import java.util.logging.Logger;

@Log
public class CliParser {

    private static final String TOOLNAME = "SharpDataExchange";
    private static final String USAGE = TOOLNAME + " get [options] [<file>] | put [options] <file> | "
            + "convert [options] <infile> [<outfile>] | terminal [options]";

    /**
     * Parse command-line arguments into a {@link CliArgs} record.
     *
     * @param args Raw command-line arguments
     * @return Populated CliArgs, or null if arguments are invalid or a no-op flag was handled
     */
    public CliArgs parse(String[] args) {
        Options options = createOptions();
        HelpFormatter formatter = new HelpFormatter();

        if (args.length == 0) {
            formatter.printHelp(USAGE, options);
            return null;
        }

        // Handle top-level flags that need no verb
        String first = args[0];
        if ("-h".equals(first) || "--help".equals(first)) {
            formatter.printHelp(USAGE, options);
            return null;
        }
        if ("-V".equals(first) || "--version".equals(first)) {
            printVersion();
            return null;
        }

        // Verb must be first
        if (!"get".equals(first) && !"put".equals(first) && !"terminal".equals(first)
                && !"convert".equals(first)) {
            formatter.printHelp(USAGE, null, options,
                    "ERROR: Expected 'get', 'put', 'convert' or 'terminal' as first argument, got: " + first);
            return null;
        }

        String verb = first;
        String[] remaining = Arrays.copyOfRange(args, 1, args.length);

        CommandLine line;
        try {
            line = new DefaultParser().parse(options, remaining);
        } catch (ParseException e) {
            formatter.printHelp(USAGE, null, options, "ERROR: " + e.getMessage());
            return null;
        }

        // Allow -h / -V after verb too
        if (line.hasOption("help")) {
            formatter.printHelp(USAGE, options);
            return null;
        }
        if (line.hasOption("version")) {
            printVersion();
            return null;
        }

        // Adjust log level
        if (line.hasOption("debug")) {
            setLogLevel(Level.FINEST);
        } else if (line.hasOption("verbose")) {
            setLogLevel(Level.FINE);
        }

        // File is required for 'put' and 'convert', optional for 'get' and 'terminal'
        String[] positional = line.getArgs();
        if (positional.length == 0 && ("put".equals(verb) || "convert".equals(verb))) {
            formatter.printHelp(USAGE, null, options, "ERROR: A file name is required");
            return null;
        }
        String file = positional.length > 0 ? positional[0] : null;
        // 'convert' takes an optional second positional: the output file
        String outputFile = "convert".equals(verb) && positional.length > 1 ? positional[1] : null;

        // Device
        PocketPcDevice device = PocketPcDevice.PC1500;
        if (line.hasOption("device")) {
            device = parseDevice(line.getOptionValue("device"), options, formatter);
            if (device == null) return null;
        }

        // Port (optional)
        String port = line.getOptionValue("port");

        // Format
        OutputFormat format = null;
        if (line.hasOption("format")) {
            if ("terminal".equals(verb)) {
                formatter.printHelp(USAGE, null, options, "ERROR: --format is not valid for terminal mode");
                return null;
            }
            if ("convert".equals(verb)) {
                formatter.printHelp(USAGE, null, options,
                        "ERROR: --format is not valid for convert (direction is detected from file content)");
                return null;
            }
            format = parseFormat(line.getOptionValue("format"), verb, options, formatter);
            if (format == null) return null;
        } else if ("get".equals(verb)) {
            format = OutputFormat.ASCII; // default for get
        }

        // get-only options
        boolean skipHeader = false;
        if ("get".equals(verb)) {
            skipHeader = line.hasOption("skip-header");
        }

        // put-only options
        Integer startAddress = null;
        Integer runAddress = null;
        boolean addUtils = false;
        String dryRunFile = null;

        if ("put".equals(verb)) {
            if (line.hasOption("dry-run")) {
                dryRunFile = line.getOptionValue("dry-run");
            }
            if (line.hasOption("run-address") && !line.hasOption("start-address")) {
                formatter.printHelp(USAGE, null, options,
                        "ERROR: Cannot specify --run-address without --start-address");
                return null;
            }
            if (line.hasOption("start-address")) {
                startAddress = parseHexInt(line.getOptionValue("start-address"));
                if (startAddress == null) {
                    formatter.printHelp(USAGE, null, options,
                            "ERROR: Invalid start address (expected hex, e.g. 38C5)");
                    return null;
                }
            }
            if (line.hasOption("run-address")) {
                runAddress = parseHexInt(line.getOptionValue("run-address"));
                if (runAddress == null) {
                    formatter.printHelp(USAGE, null, options,
                            "ERROR: Invalid run address (expected hex, e.g. 38C5)");
                    return null;
                }
            }
            // Default run address when start address is set but run address is not
            if (startAddress != null && runAddress == null) {
                runAddress = 0xFFFF;
            }
            addUtils = line.hasOption("add-utils");
        }

        // The emulator is reached over a host pseudo-terminal (/dev/ttysNNN). There are
        // always several of those present and none of them is distinguishable as "the
        // emulator", so auto-detection is not possible: the port must be given explicitly.
        // Fall back to the Calc-U-1600 app's fixed serial socket when no port is given.
        if (device.isEmulator() && (port == null || port.isBlank()) && dryRunFile == null) {
            port = "/tmp/calcu1600.serial";
        }

        log.log(Level.FINE, "Parsed CLI: verb={0} file={1} device={2} port={3} format={4}",
                new Object[]{verb, file, device, port, format});

        return new CliArgs(verb, file, device, port, format, startAddress, runAddress, addUtils, skipHeader,
                dryRunFile, outputFile);
    }

    private Options createOptions() {
        Options options = new Options();
        options.addOption(new Option("h", "help", false, "Print this help."));
        options.addOption(new Option("V", "version", false, "Print version and exit."));
        options.addOption(new Option("v", "verbose", false, "Verbose logging."));
        options.addOption(new Option("vv", "debug", false, "Debug logging."));
        options.addOption(Option.builder("d").longOpt("device")
                .desc("Device: pc1500 (default), pc1500a, pc1600, pc1600emul").hasArg().build());
        options.addOption(Option.builder("p").longOpt("port")
                .desc("Serial port (auto-detected if omitted)").hasArg().build());
        options.addOption(Option.builder("f").longOpt("format")
                .desc("Format: ascii (default for get), binary").hasArg().build());
        options.addOption(Option.builder().longOpt("start-address")
                .desc("Machine language load address (hex, e.g. 38C5)").hasArg().build());
        options.addOption(Option.builder().longOpt("run-address")
                .desc("Machine language auto-run address (hex)").hasArg().build());
        options.addOption(new Option("u", "add-utils", false,
                "Prepend serial utility BASIC sub-program"));
        options.addOption(new Option(null, "skip-header", false,
                "Omit serial header from saved binary file (get --format binary only; not recommended)"));
        options.addOption(Option.builder().longOpt("dry-run")
                .desc("put only: write the fully-formed data block (header + payload) to this "
                        + "file instead of sending it over serial").hasArg().build());
        return options;
    }

    private PocketPcDevice parseDevice(String value, Options options, HelpFormatter formatter) {
        return switch (value.toLowerCase()) {
            case "pc1500" -> PocketPcDevice.PC1500;
            case "pc1500a" -> PocketPcDevice.PC1500A;
            case "pc1600" -> PocketPcDevice.PC1600;
            case "pc1600emul" -> PocketPcDevice.PC1600EMUL;
            default -> {
                formatter.printHelp(USAGE, null, options,
                        "ERROR: Unknown device '" + value + "' (expected pc1500, pc1500a, pc1600, pc1600emul)");
                yield null;
            }
        };
    }

    private OutputFormat parseFormat(String value, String verb, Options options, HelpFormatter formatter) {
        return switch (value.toLowerCase()) {
            case "ascii" -> OutputFormat.ASCII;
            case "binary" -> OutputFormat.BINARY;
            default -> {
                System.err.println("ERROR: Unknown format '" + value + "' (expected ascii, binary)");
                yield null;
            }
            };
    }

    /**
     * Parse a hex string (with or without 0x/$/ prefix) or decimal string into an int.
     *
     * @return Parsed integer, or null if unparseable
     */
    Integer parseHexInt(String input) {
        if (input == null || input.isEmpty()) {
            return null;
        }
        try {
            if (input.startsWith("0x") || input.startsWith("0X")) {
                return Integer.parseInt(input.substring(2), 16);
            } else if (input.startsWith("$") || input.startsWith("&")) {
                return Integer.parseInt(input.substring(1), 16);
            } else {
                // Treat bare strings as hex if they contain A-F, otherwise decimal
                boolean hasHexChars = input.chars().anyMatch(c ->
                        (c >= 'A' && c <= 'F') || (c >= 'a' && c <= 'f'));
                return hasHexChars ? Integer.parseInt(input, 16) : Integer.parseInt(input);
            }
        } catch (NumberFormatException e) {
            log.log(Level.FINE, "Cannot parse address: {0}", input);
            return null;
        }
    }

    private void setLogLevel(Level level) {
        Logger appLogger = Logger.getLogger("ch.erzberger");
        appLogger.setLevel(level);
        for (Handler h : appLogger.getHandlers()) {
            h.setLevel(level);
        }
        log.log(level, "Log level set to {0}", level);
    }

    private void printVersion() {
        String version = getVersionFromManifest();
        System.out.println(TOOLNAME + " " + (version != null ? version : "(unknown version)"));
    }

    private String getVersionFromManifest() {
        Manifest mf = new Manifest();
        try {
            var stream = Thread.currentThread().getContextClassLoader()
                    .getResourceAsStream("META-INF/MANIFEST.MF");
            if (stream == null) return null;
            mf.read(stream);
        } catch (IOException e) {
            log.log(Level.FINE, "Cannot read manifest", e);
            return null;
        }
        Attributes atts = mf.getMainAttributes();
        String buildTime = atts.getValue("Build-Time");
        String version = atts.getValue("Implementation-Version");
        if (version == null) return null;
        return buildTime == null || buildTime.isEmpty()
                ? version
                : version + " - pre-release from " + buildTime;
    }
}
