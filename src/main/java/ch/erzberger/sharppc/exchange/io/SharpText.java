package ch.erzberger.sharppc.exchange.io;

import java.nio.charset.CharacterCodingException;
import java.nio.charset.Charset;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;

/**
 * Text-encoding helpers for BASIC listings exchanged with the Pocket Computer.
 * <p>
 * On the device, a listing is stored in IBM PC Code Page 437 (the Sharp character set).
 * On the PC, SharpDataExchange always reads and writes {@code .bas} files as UTF-8 so
 * they display correctly in a modern editor. This class converts between the two.
 */
public final class SharpText {

    /** The Sharp PC-1500/1600 character set. */
    public static final Charset CP437 = Charset.forName("Cp437");

    private SharpText() {
    }

    /**
     * Decode a {@code .bas} file's bytes to source text. Valid UTF-8 is decoded as
     * UTF-8 (a modern, hand-edited listing); anything else is decoded as CP437 (a
     * listing saved straight off the device with {@code SAVE ...,A}). Line endings
     * are normalized to LF and a trailing {@code 0x1A} EOF marker is dropped.
     */
    public static String decodeBasListing(byte[] data) {
        String text;
        try {
            text = StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(java.nio.ByteBuffer.wrap(data))
                    .toString();
        } catch (CharacterCodingException notUtf8) {
            text = new String(data, CP437);
        }
        return normalize(text);
    }

    /**
     * Decode a raw listing off the device (always CP437) to clean, UTF-8-ready source
     * text: LF line endings, no trailing {@code 0x1A}.
     */
    public static String cleanListing(byte[] data) {
        return normalize(new String(data, CP437));
    }

    /**
     * Locate the first character that cannot be sent to a PC-1500/1500A, whose character
     * set is 7-bit ASCII (no CP437 upper half).
     *
     * @param text a listing already normalized to LF line endings
     * @return {@code {lineNumber, column, codePoint}} (both 1-based) for the first character
     *         above 0x7F, or {@code null} if every character is 7-bit ASCII
     */
    public static int[] firstNonAsciiForPc1500(String text) {
        int line = 1;
        int col = 1;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '\n') {
                line++;
                col = 1;
                continue;
            }
            if (c > 0x7F) {
                return new int[]{line, col, c};
            }
            col++;
        }
        return null;
    }

    private static String normalize(String text) {
        text = text.replace("\r\n", "\n").replace('\r', '\n');
        int end = text.length();
        while (end > 0 && text.charAt(end - 1) == '\u001A') {
            end--;
        }
        return text.substring(0, end);
    }
}
