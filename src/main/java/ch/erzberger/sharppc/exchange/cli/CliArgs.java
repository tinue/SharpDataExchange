package ch.erzberger.sharppc.exchange.cli;

/**
 * Immutable DTO holding all parsed command-line arguments.
 *
 * @param verb         "get" or "put"
 * @param file         Output file (get) or input file (put)
 * @param device       Target Pocket PC device
 * @param port         Serial port name, or null for auto-detection
 * @param format       Output format (get) or forced input format (put); null means auto-detect
 * @param startAddress Machine language load address (put only), or null
 * @param runAddress   Machine language auto-run address (put only), or null
 * @param addUtils       Whether to prepend serial utility BASIC sub-program (put only)
 * @param includeHeader  Whether to include the serial header in the saved binary file (get only)
 */
public record CliArgs(
        String verb,
        String file,
        PocketPcDevice device,
        String port,
        OutputFormat format,
        Integer startAddress,
        Integer runAddress,
        boolean addUtils,
        boolean includeHeader) {
}
