package ch.erzberger.sharppc.exchange.detect;

import ch.erzberger.sharppc.exchange.convert.DataType;
import ch.erzberger.sharppc.exchange.header.Ce158Header;
import ch.erzberger.sharppc.exchange.header.Pc1600Header;
import ch.erzberger.sharppc.exchange.header.SerialHeader;
import ch.erzberger.sharppc.exchange.io.SharpText;
import lombok.extern.java.Log;

import java.util.logging.Level;
import java.util.regex.Pattern;

/**
 * Detects the content type of a raw byte array using the following priority order:
 * <ol>
 *   <li>CE-158 magic header (PC-1500/PC-1500A)</li>
 *   <li>PC-1600 magic header</li>
 *   <li>ASCII BASIC heuristic (lines starting with a line number)</li>
 *   <li>ASCII Reserve (SDAR) marker on first line</li>
 *   <li>ASCII Variables (SDAV) marker on first line</li>
 *   <li>UNKNOWN fallback</li>
 * </ol>
 */
@Log
public class ContentDetector {

    private static final Pattern BASIC_LINE = Pattern.compile("^\\d+\\s.*");
    private static final String SDAR_MARKER = "; SDAR:";
    private static final String SDAV_MARKER = "; SDAV:";

    /**
     * Detect the content type of {@code data}.
     *
     * @param data Raw bytes (from serial or disk)
     * @return Detected {@link DataType}
     */
    public DataType detect(byte[] data) {
        if (data == null || data.length == 0) {
            return DataType.UNKNOWN;
        }

        // 1. Try CE-158 header
        DataType ce158Type = tryCe158(data);
        if (ce158Type != null) {
            log.log(Level.FINE, "Detected CE-158 type: {0}", ce158Type);
            return ce158Type;
        }

        // 2. Try PC-1600 header
        DataType pc1600Type = tryPc1600(data);
        if (pc1600Type != null) {
            log.log(Level.FINE, "Detected PC-1600 type: {0}", pc1600Type);
            return pc1600Type;
        }

        // 3-5. Try ASCII content
        String text = tryDecodeAsText(data);
        if (text != null) {
            // ASCII Reserve
            if (text.startsWith(SDAR_MARKER)) {
                log.log(Level.FINE, "Detected ASCII_RESERVE (SDAR)");
                return DataType.ASCII_RESERVE;
            }
            // ASCII Variables
            if (text.startsWith(SDAV_MARKER)) {
                log.log(Level.FINE, "Detected ASCII_VARS (SDAV)");
                return DataType.ASCII_VARS;
            }
            // ASCII BASIC: check first few non-blank lines
            if (looksLikeAsciiBasic(text)) {
                log.log(Level.FINE, "Detected ASCII_BASIC");
                return DataType.ASCII_BASIC;
            }
        }

        log.log(Level.FINE, "Type UNKNOWN");
        return DataType.UNKNOWN;
    }

    private DataType tryCe158(byte[] data) {
        SerialHeader header = SerialHeader.makeHeader(data);
        if (!(header instanceof Ce158Header)) {
            return null;
        }
        return switch (header.getType()) {
            case BASIC -> DataType.BINARY_BASIC;
            case RESERVE -> DataType.BINARY_RESERVE;
            case MACHINE -> DataType.MACHINE;
            case VARIABLES -> DataType.BINARY_VARS;
        };
    }

    private DataType tryPc1600(byte[] data) {
        SerialHeader header = SerialHeader.makeHeader(data);
        if (!(header instanceof Pc1600Header)) {
            return null;
        }
        return switch (header.getType()) {
            case BASIC -> DataType.BINARY_BASIC;
            case MACHINE -> DataType.MACHINE;
            default -> DataType.UNKNOWN;
        };
    }

    /**
     * Attempt to decode {@code data} as a text listing. High bytes (0x80–0xFF) are
     * allowed — a listing saved off the device with {@code SAVE ...,A} carries
     * Sharp-codepage characters such as umlauts — and so is a trailing {@code 0x1A}
     * (the CP/M-style end-of-file marker). Genuine binary control bytes reject.
     *
     * @return The text content (decoded CP437) if it looks like text, null otherwise
     */
    private String tryDecodeAsText(byte[] data) {
        for (byte b : data) {
            int v = b & 0xFF;
            if (v < 0x20 && v != '\r' && v != '\n' && v != '\t' && v != 0x1A) {
                return null;
            }
        }
        return new String(data, SharpText.CP437);
    }

    /**
     * Check if the text looks like an ASCII BASIC program.
     * At least 3 of the first 5 non-blank lines must start with a line number.
     */
    private boolean looksLikeAsciiBasic(String text) {
        String[] lines = text.split("\\r?\\n");
        int checked = 0;
        int matched = 0;
        for (String line : lines) {
            String trimmed = line.trim();
            if (trimmed.isEmpty()) continue;
            if (BASIC_LINE.matcher(trimmed).matches()) {
                matched++;
            }
            checked++;
            if (checked >= 5) break;
        }
        return checked > 0 && matched >= Math.min(3, checked);
    }
}
