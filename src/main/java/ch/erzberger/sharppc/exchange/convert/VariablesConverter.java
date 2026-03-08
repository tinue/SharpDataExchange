package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

import java.io.ByteArrayOutputStream;
import java.math.BigDecimal;
import java.math.MathContext;
import java.math.RoundingMode;
import java.util.ArrayList;
import java.util.List;
import java.util.logging.Level;

/**
 * Bidirectional converter for PC-1500 Variables tape (CE-158 type 'H').
 *
 * <p>Payload: sequential [0x00][4-byte prefix][data] records, no trailing terminator.
 * <p>ASCII: SDAV text format (see plan.md for full spec).
 */
@Log
public class VariablesConverter {

    private static final int PREFIX_SIZE = 4;

    private VariablesConverter() {}

    /**
     * Convert binary Variables payload to SDAV text.
     *
     * @param payload  Raw binary bytes after CE-158 header (any leading 0x00 capture artifacts already excluded)
     * @param filename Filename from CE-158 header (may be null or empty)
     * @param device   Target device
     * @return SDAV text
     */
    public static String toAscii(byte[] payload, String filename, PocketPcDevice device) {
        StringBuilder sb = new StringBuilder();
        String deviceStr = PocketPcDevice.PC1600.equals(device) ? "pc1600" : "pc1500";
        sb.append("; SDAV:1.0 ").append(deviceStr).append('\n');

        // We'll fill in Count later
        int countPlaceholderPos = sb.length();
        sb.append("; Count: 0\n"); // placeholder

        if (filename != null && !filename.isEmpty()) {
            sb.append("; Filename: ").append(filename).append('\n');
        }

        int count = 0;
        int offset = 0;
        while (offset < payload.length) {
            // Expect 0x00 separator
            if ((payload[offset] & 0xFF) != 0x00) {
                throw new IllegalArgumentException(
                        "Expected 0x00 separator at offset " + offset + ", got 0x" +
                                Integer.toHexString(payload[offset] & 0xFF));
            }
            offset++;
            if (offset + PREFIX_SIZE > payload.length) break;

            byte[] prefix = new byte[PREFIX_SIZE];
            System.arraycopy(payload, offset, prefix, 0, PREFIX_SIZE);
            offset += PREFIX_SIZE;

            int discriminator = prefix[1] & 0xFF;
            int type = prefix[3] & 0xFF;
            int totalLen = (prefix[0] & 0xFF) + 1; // len-1 + 1
            int dataLen = totalLen - PREFIX_SIZE;

            if (offset + dataLen > payload.length) {
                throw new IllegalArgumentException(
                        "Record data extends beyond payload at offset " + offset);
            }

            byte[] data = new byte[dataLen];
            System.arraycopy(payload, offset, data, 0, dataLen);
            offset += dataLen;
            count++;

            if (discriminator == 0x00) {
                // Simple variable
                if (type == 0x88) {
                    // Numeric scalar
                    sb.append(formatBcd(data)).append('\n');
                } else {
                    // String scalar (type 0x10 = 16 bytes, or DIM$(0)*L)
                    sb.append('"').append(escapeString(data)).append('"').append('\n');
                }
            } else {
                // DIM'd array
                int dimMax = discriminator;
                if (type == 0x88) {
                    // Numeric array
                    sb.append("DIM (").append(dimMax).append(")\n");
                    int elemSize = 8;
                    for (int e = 0; e <= dimMax; e++) {
                        byte[] elem = new byte[elemSize];
                        System.arraycopy(data, e * elemSize, elem, 0, elemSize);
                        sb.append(formatBcd(elem)).append('\n');
                    }
                } else {
                    // String array
                    int maxLen = type;
                    sb.append("DIM $(").append(dimMax).append(")*").append(maxLen).append('\n');
                    for (int e = 0; e <= dimMax; e++) {
                        byte[] slot = new byte[maxLen];
                        System.arraycopy(data, e * maxLen, slot, 0, maxLen);
                        sb.append('"').append(escapeString(slot)).append('"').append('\n');
                    }
                }
            }
        }

        // Replace count placeholder
        String result = sb.toString();
        String countLine = "; Count: " + count + "\n";
        String placeholder = "; Count: 0\n";
        result = result.substring(0, countPlaceholderPos) + countLine +
                result.substring(countPlaceholderPos + placeholder.length());
        return result;
    }

    /**
     * Convert SDAV text to binary Variables payload.
     *
     * @param sdavText SDAV text
     * @param device   Target device (unused for now, kept for symmetry)
     * @return Binary payload bytes (no CE-158 header)
     */
    public static byte[] toBinary(String sdavText, PocketPcDevice device) {
        String[] lines = sdavText.split("\\r?\\n");
        ByteArrayOutputStream out = new ByteArrayOutputStream();

        int expectedCount = -1;
        int recordCount = 0;

        // Parse lines, tracking DIM context
        int dimMax = -1;
        int dimMaxLen = -1; // -1 for numeric, >=0 for string
        boolean inDim = false;
        List<byte[]> dimElements = new ArrayList<>();

        int i = 0;
        while (i < lines.length) {
            String raw = lines[i];
            String line = raw.trim();
            i++;

            if (line.isEmpty() || line.startsWith(";")) {
                // Extract Count from header comment
                if (line.startsWith("; Count:")) {
                    try {
                        expectedCount = Integer.parseInt(line.substring(8).trim());
                    } catch (NumberFormatException e) {
                        log.log(Level.WARNING, "Could not parse Count header: {0}", line);
                    }
                }
                continue;
            }

            if (inDim) {
                // Collect DIM elements
                if (dimMaxLen >= 0) {
                    // String array element
                    if (!line.startsWith("\"") || !line.endsWith("\"")) {
                        throw new IllegalArgumentException(
                                "Expected quoted string for DIM array element, got: " + line);
                    }
                    String content = line.substring(1, line.length() - 1);
                    byte[] slot = encodeStringSlot(content, dimMaxLen);
                    dimElements.add(slot);
                } else {
                    // Numeric array element
                    byte[] bcd = parseBcd(line);
                    dimElements.add(bcd);
                }
                if (dimElements.size() == dimMax + 1) {
                    // All elements collected -- emit the array record
                    if (dimMaxLen >= 0) {
                        // String array
                        int totalData = (dimMax + 1) * dimMaxLen;
                        int totalRecord = PREFIX_SIZE + totalData;
                        out.write(0x00); // separator
                        out.write(totalRecord - 1); // len-1
                        out.write(dimMax);
                        out.write(0x00);
                        out.write(dimMaxLen);
                        for (byte[] slot : dimElements) {
                            out.writeBytes(slot);
                        }
                    } else {
                        // Numeric array
                        int totalData = (dimMax + 1) * 8;
                        int totalRecord = PREFIX_SIZE + totalData;
                        out.write(0x00); // separator
                        out.write(totalRecord - 1); // len-1
                        out.write(dimMax);
                        out.write(0x00);
                        out.write(0x88);
                        for (byte[] elem : dimElements) {
                            out.writeBytes(elem);
                        }
                    }
                    recordCount++;
                    inDim = false;
                    dimElements.clear();
                }
            } else if (line.startsWith("DIM $(")) {
                // String array header: DIM $(<dim_max>)*<max_len>
                int rparen = line.indexOf(')');
                int star = line.indexOf('*');
                dimMax = Integer.parseInt(line.substring(6, rparen).trim());
                dimMaxLen = Integer.parseInt(line.substring(star + 1).trim());
                inDim = true;
                dimElements.clear();
            } else if (line.startsWith("DIM (")) {
                // Numeric array header: DIM (<dim_max>)
                int rparen = line.indexOf(')');
                dimMax = Integer.parseInt(line.substring(5, rparen).trim());
                dimMaxLen = -1; // numeric
                inDim = true;
                dimElements.clear();
            } else if (line.startsWith("\"") && line.endsWith("\"")) {
                // String scalar
                String content = line.substring(1, line.length() - 1);
                byte[] slot = encodeStringSlot(content, 16);
                // Prefix: 0x13 0x00 0x00 0x10 + 16 bytes = 20 bytes total, len-1=19
                out.write(0x00); // separator
                out.write(0x13);
                out.write(0x00);
                out.write(0x00);
                out.write(0x10);
                out.writeBytes(slot);
                recordCount++;
            } else {
                // Numeric scalar
                byte[] bcd = parseBcd(line);
                // Prefix: 0x0B 0x00 0x00 0x88 + 8 bytes = 12 bytes total, len-1=11
                out.write(0x00); // separator
                out.write(0x0B);
                out.write(0x00);
                out.write(0x00);
                out.write((byte) 0x88);
                out.writeBytes(bcd);
                recordCount++;
            }
        }

        if (expectedCount >= 0 && recordCount != expectedCount) {
            throw new IllegalArgumentException(
                    "Count mismatch: SDAV header says " + expectedCount + " records but found " + recordCount);
        }

        return out.toByteArray();
    }

    // ---- BCD helpers ----

    /**
     * Decode an 8-byte BCD value to a formatted decimal string.
     */
    static String formatBcd(byte[] data) {
        BigDecimal bd = decodeBcd(data);
        if (bd.compareTo(BigDecimal.ZERO) == 0) return "0";
        BigDecimal stripped = bd.stripTrailingZeros();
        // Determine exponent (floor(log10(|value|)))
        int precision = stripped.precision();
        int scale = stripped.scale();
        int exp = precision - scale - 1;
        if (exp >= -3 && exp <= 9) {
            return stripped.toPlainString();
        }
        return stripped.toString(); // scientific notation e.g. 1E-9
    }

    /**
     * Decode an 8-byte BCD value to BigDecimal.
     */
    static BigDecimal decodeBcd(byte[] data) {
        // B2H binary integer: byte 4 == 0xB2
        if ((data[4] & 0xFF) == 0xB2) {
            int value = (short) (((data[5] & 0xFF) << 8) | (data[6] & 0xFF));
            return BigDecimal.valueOf(value);
        }
        // BCD float
        int exponent = data[0]; // signed 8-bit (Java byte is already signed)
        boolean negative = (data[1] & 0xFF) == 0x80;
        long mantissa = 0;
        for (int i = 2; i <= 6; i++) {
            int hi = (data[i] >> 4) & 0xF;
            int lo = data[i] & 0xF;
            mantissa = mantissa * 100 + hi * 10 + lo;
        }
        if (mantissa == 0) return BigDecimal.ZERO;
        BigDecimal bd = new BigDecimal(mantissa).scaleByPowerOfTen(exponent - 9);
        return negative ? bd.negate() : bd;
    }

    /**
     * Encode a decimal string to 8 BCD bytes.
     */
    static byte[] parseBcd(String decimal) {
        byte[] result = new byte[8];
        BigDecimal bd;
        try {
            bd = new BigDecimal(decimal.trim());
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("Invalid decimal value: " + decimal);
        }
        if (bd.compareTo(BigDecimal.ZERO) == 0) {
            return result; // all zeros
        }
        boolean negative = bd.signum() < 0;
        BigDecimal abs = bd.abs().round(new MathContext(10, RoundingMode.HALF_UP));

        // Exponent = floor(log10(abs)) = precision - scale - 1
        int exp = abs.precision() - abs.scale() - 1;
        if (exp < -99 || exp > 99) {
            throw new IllegalArgumentException("Value out of range for PC-1500 BCD: " + decimal);
        }

        // Mantissa as 10-digit integer
        BigDecimal mantissaBd = abs.scaleByPowerOfTen(9 - exp).setScale(0, RoundingMode.HALF_UP);
        long mantissa = mantissaBd.longValueExact();

        result[0] = (byte) exp;
        result[1] = negative ? (byte) 0x80 : 0x00;
        // Encode mantissa as 5 packed BCD bytes (bytes 2-6), MSB first
        for (int i = 6; i >= 2; i--) {
            int lo = (int) (mantissa % 10); mantissa /= 10;
            int hi = (int) (mantissa % 10); mantissa /= 10;
            result[i] = (byte) ((hi << 4) | lo);
        }
        result[7] = 0x00;
        return result;
    }

    // ---- String helpers ----

    /**
     * Trim trailing null bytes and apply SDAV escaping.
     */
    static String escapeString(byte[] buffer) {
        // Trim trailing nulls
        int len = buffer.length;
        while (len > 0 && buffer[len - 1] == 0x00) len--;

        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < len; i++) {
            int b = buffer[i] & 0xFF;
            if (b == '\\') {
                sb.append("\\\\");
            } else if (b == '"') {
                sb.append("\\\"");
            } else if (b < 0x20 || b > 0x7E) {
                sb.append(String.format("\\x%02X", b));
            } else {
                sb.append((char) b);
            }
        }
        return sb.toString();
    }

    /**
     * Unescape an SDAV string and null-pad to {@code maxLen} bytes.
     */
    static byte[] encodeStringSlot(String escaped, int maxLen) {
        byte[] slot = new byte[maxLen];
        byte[] decoded = unescapeString(escaped);
        int copyLen = Math.min(decoded.length, maxLen);
        System.arraycopy(decoded, 0, slot, 0, copyLen);
        return slot;
    }

    static byte[] unescapeString(String escaped) {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        int i = 0;
        while (i < escaped.length()) {
            char c = escaped.charAt(i);
            if (c == '\\' && i + 1 < escaped.length()) {
                char next = escaped.charAt(i + 1);
                if (next == '\\') {
                    buf.write('\\');
                    i += 2;
                } else if (next == '"') {
                    buf.write('"');
                    i += 2;
                } else if (next == 'x' && i + 3 < escaped.length()) {
                    String hex = escaped.substring(i + 2, i + 4);
                    buf.write(Integer.parseInt(hex, 16));
                    i += 4;
                } else {
                    buf.write(c);
                    i++;
                }
            } else {
                buf.write(c);
                i++;
            }
        }
        return buf.toByteArray();
    }
}
