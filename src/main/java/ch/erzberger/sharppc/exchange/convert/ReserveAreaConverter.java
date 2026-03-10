package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharpbasic.core.keyword.BasicKeyword;
import ch.erzberger.sharpbasic.core.keyword.KeywordRegistry;
import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

import java.io.ByteArrayOutputStream;
import java.io.UnsupportedEncodingException;
import java.util.Arrays;
import java.util.Optional;

/**
 * Bidirectional converter for the PC-1500 Reserve Area (CE-158 type 'A').
 *
 * <p>Binary payload structure (188 bytes):
 * <ul>
 *   <li>3 × 26-byte labels (null-padded CP437)</li>
 *   <li>110-byte key contents pool</li>
 * </ul>
 *
 * <p>ASCII format: SDAR text (see plan.md for full spec).
 *
 * <p>Throws {@link IllegalArgumentException} for malformed input or if the
 * tokenized pool size exceeds the hardware limit.
 */
@Log
public class ReserveAreaConverter {

    private static final int LABEL_SIZE = 26;
    private static final int NUM_LAYERS = 3;
    private static final int NUM_KEYS = 6;
    private static final int POOL_SIZE = 110;
    private static final int PAYLOAD_SIZE = NUM_LAYERS * LABEL_SIZE + POOL_SIZE; // 188

    // Key codes per layer/key (0-indexed)
    // keyCode[layer][key] where layer 0-2, key 0-5
    private static final int[][] KEY_CODE = {
            {0x01, 0x02, 0x03, 0x04, 0x05, 0x06}, // Layer 1
            {0x11, 0x12, 0x13, 0x14, 0x15, 0x16}, // Layer 2
            {0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E}  // Layer 3
    };

    private ReserveAreaConverter() {}

    /**
     * Convert binary Reserve Area payload to SDAR text.
     *
     * @param payload  The 188-byte binary payload (CE-158 header already stripped)
     * @param filename Filename from CE-158 header (for round-trip fidelity)
     * @param device   Target device (for SDAR header line)
     * @return SDAR text
     */
    public static String toAscii(byte[] payload, String filename, PocketPcDevice device) {
        if (payload.length < PAYLOAD_SIZE) {
            throw new IllegalArgumentException(
                    "Reserve payload too short: expected " + PAYLOAD_SIZE + " bytes, got " + payload.length);
        }

        KeywordRegistry registry = registryFor(device);

        // Extract labels
        String[] labels = new String[NUM_LAYERS];
        for (int i = 0; i < NUM_LAYERS; i++) {
            byte[] labelBytes = Arrays.copyOfRange(payload, i * LABEL_SIZE, (i + 1) * LABEL_SIZE);
            labels[i] = decodeLabel(labelBytes);
        }

        // Parse pool -> key content strings [layer][key]
        String[][] keyContent = new String[NUM_LAYERS][NUM_KEYS];
        for (String[] row : keyContent) Arrays.fill(row, "");

        int poolStart = NUM_LAYERS * LABEL_SIZE;
        int offset = poolStart;
        while (offset < payload.length) {
            int b = payload[offset] & 0xFF;
            if (b == 0x00) break;

            // Decode key code to layer/key
            int layer = -1, key = -1;
            if (b >= 0x01 && b <= 0x06) { layer = 0; key = b - 1; }
            else if (b >= 0x09 && b <= 0x0E) { layer = 2; key = b - 0x09; }
            else if (b >= 0x11 && b <= 0x16) { layer = 1; key = b - 0x11; }
            else {
                throw new IllegalArgumentException("Unknown key code in pool: 0x" + Integer.toHexString(b) + " at offset " + offset);
            }
            offset++;

            // Collect content bytes until next key code or terminator
            ByteArrayOutputStream contentBytes = new ByteArrayOutputStream();
            while (offset < payload.length) {
                int cb = payload[offset] & 0xFF;
                if (cb == 0x00 || (cb >= 0x01 && cb <= 0x16)) break;
                if (cb == 0xF0 || cb == 0xF1) {
                    contentBytes.write(cb);
                    if (offset + 1 < payload.length) {
                        contentBytes.write(payload[offset + 1] & 0xFF);
                    }
                    offset += 2;
                } else {
                    contentBytes.write(cb);
                    offset++;
                }
            }
            keyContent[layer][key] = detokenizeContent(contentBytes.toByteArray(), registry);
        }

        // Build SDAR text
        StringBuilder sb = new StringBuilder();
        String deviceStr = PocketPcDevice.PC1600.equals(device) ? "pc1600" : "pc1500";
        sb.append("; SDAR:1.0 ").append(deviceStr).append('\n');
        if (filename != null && !filename.isEmpty()) {
            sb.append("; Filename: ").append(filename).append('\n');
        }
        for (int l = 0; l < NUM_LAYERS; l++) {
            sb.append('\n');
            sb.append("[layer ").append(l + 1).append("]\n");
            sb.append("label: ").append(labels[l]).append('\n');
            for (int k = 0; k < NUM_KEYS; k++) {
                sb.append("key ").append(k + 1).append(": ").append(keyContent[l][k]).append('\n');
            }
        }
        return sb.toString();
    }

    /**
     * Convert SDAR text to binary Reserve Area payload.
     *
     * @param sdarText SDAR text
     * @param device   Target device
     * @return 188-byte binary payload (no CE-158 header)
     * @throws IllegalArgumentException if pool exceeds 110 bytes
     */
    public static byte[] toBinary(String sdarText, PocketPcDevice device) {
        KeywordRegistry registry = registryFor(device);

        String[] labels = new String[NUM_LAYERS];
        String[][] keyContent = new String[NUM_LAYERS][NUM_KEYS];
        for (String[] row : keyContent) Arrays.fill(row, "");

        int currentLayer = -1;

        for (String rawLine : sdarText.split("\\r?\\n")) {
            String line = rawLine.trim();
            if (line.isEmpty() || line.startsWith(";")) continue;

            if (line.startsWith("[layer ")) {
                int layerNum = Character.getNumericValue(line.charAt(7));
                currentLayer = layerNum - 1;
            } else if (line.startsWith("label:") && currentLayer >= 0) {
                labels[currentLayer] = line.substring("label:".length()).trim();
            } else if (line.startsWith("key ") && currentLayer >= 0) {
                int colonIdx = line.indexOf(':');
                if (colonIdx >= 0) {
                    int keyNum = Integer.parseInt(line.substring(4, colonIdx).trim());
                    String content = line.substring(colonIdx + 1);
                    // Remove leading space after colon only (key content is after "key N: ")
                    if (content.startsWith(" ")) content = content.substring(1);
                    keyContent[currentLayer][keyNum - 1] = content;
                }
            }
        }

        // Build payload
        byte[] payload = new byte[PAYLOAD_SIZE];

        // Write labels
        for (int l = 0; l < NUM_LAYERS; l++) {
            String label = labels[l] != null ? labels[l] : "";
            byte[] labelBytes = encodeLabel(label);
            System.arraycopy(labelBytes, 0, payload, l * LABEL_SIZE, LABEL_SIZE);
        }

        // Build pool
        ByteArrayOutputStream pool = new ByteArrayOutputStream();
        for (int l = 0; l < NUM_LAYERS; l++) {
            for (int k = 0; k < NUM_KEYS; k++) {
                String content = keyContent[l][k];
                if (content == null || content.isEmpty()) continue;
                byte[] tokenized = tokenizeContent(content, registry);
                pool.write(KEY_CODE[l][k]);
                pool.writeBytes(tokenized);
            }
        }
        pool.write(0x00); // terminator

        byte[] poolBytes = pool.toByteArray();
        if (poolBytes.length > POOL_SIZE) {
            throw new IllegalArgumentException(
                    "Reserve pool too large: " + poolBytes.length + " bytes (max " + POOL_SIZE + ")");
        }

        System.arraycopy(poolBytes, 0, payload, NUM_LAYERS * LABEL_SIZE, poolBytes.length);
        // remaining bytes in pool area are already 0x00

        return payload;
    }

    // ---- Helpers ----

    private static KeywordRegistry registryFor(PocketPcDevice device) {
        return PocketPcDevice.PC1600.equals(device)
                ? KeywordRegistry.forPc1600()
                : KeywordRegistry.forPc1500();
    }

    private static String decodeLabel(byte[] labelBytes) {
        try {
            return new String(labelBytes, "Cp437").replace("\0", "").trim();
        } catch (UnsupportedEncodingException e) {
            throw new NoClassDefFoundError("CP437 not available");
        }
    }

    private static byte[] encodeLabel(String label) {
        byte[] result = new byte[LABEL_SIZE];
        try {
            byte[] encoded = label.getBytes("Cp437");
            int len = Math.min(encoded.length, LABEL_SIZE);
            System.arraycopy(encoded, 0, result, 0, len);
        } catch (UnsupportedEncodingException e) {
            throw new NoClassDefFoundError("CP437 not available");
        }
        return result; // remaining bytes are 0x00
    }

    /**
     * Detokenize pool content bytes to a readable string.
     * 0xF0/0xF1 + byte = 2-byte BASIC token; other bytes = plain CP437 char.
     */
    private static String detokenizeContent(byte[] bytes, KeywordRegistry registry) {
        StringBuilder sb = new StringBuilder();
        int i = 0;
        while (i < bytes.length) {
            int b = bytes[i] & 0xFF;
            if ((b == 0xF0 || b == 0xF1) && i + 1 < bytes.length) {
                int code = (b << 8) | (bytes[i + 1] & 0xFF);
                Optional<BasicKeyword> kw = registry.lookupByTokenCode(code);
                if (kw.isPresent()) {
                    sb.append(kw.get().name());
                } else {
                    sb.append((char) b);
                    sb.append((char) (bytes[i + 1] & 0xFF));
                }
                i += 2;
            } else {
                sb.append((char) b);
                i++;
            }
        }
        return sb.toString();
    }

    /**
     * Tokenize a key content string to pool bytes.
     * Keywords (full names from registry) -> 2-byte token codes; everything else -> plain byte.
     */
    static byte[] tokenizeContent(String content, KeywordRegistry registry) {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        int i = 0;
        while (i < content.length()) {
            char c = content.charAt(i);
            if (Character.isLetter(c) || c == '$') {
                // Collect alphabetic + $ run
                int start = i;
                while (i < content.length() && (Character.isLetter(content.charAt(i)) || content.charAt(i) == '$')) {
                    i++;
                }
                String run = content.substring(start, i);
                // Try longest match first
                int matched = 0;
                for (int len = run.length(); len >= 1; len--) {
                    Optional<BasicKeyword> kw = registry.lookup(run.substring(0, len));
                    if (kw.isPresent()) {
                        int code = kw.get().tokenCode();
                        buf.write(code >> 8);
                        buf.write(code & 0xFF);
                        matched = len;
                        break;
                    }
                }
                // Emit remaining chars as plain bytes
                for (int j = matched; j < run.length(); j++) {
                    buf.write(run.charAt(j));
                }
            } else {
                buf.write(c);
                i++;
            }
        }
        return buf.toByteArray();
    }
}
