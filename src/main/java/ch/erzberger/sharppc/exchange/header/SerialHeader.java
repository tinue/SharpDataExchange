package ch.erzberger.sharppc.exchange.header;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.Getter;
import lombok.extern.java.Log;

import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.logging.Level;

/**
 * Represents a serial header. The concrete header will either be for a PC-1600, or for a CE-158.
 */
@Log
@Getter
public abstract class SerialHeader {

    /**
     * Type of the data payload as indicated by the header.
     */
    public enum FileType {
        BASIC, MACHINE, RESERVE, VARIABLES
    }

    FileType type;
    String filename;
    int startAddr;
    int length;
    int runAddr;
    PocketPcDevice device;

    protected SerialHeader(FileType type, String filename, int startAddr, int length, int runAddr) {
        this.type = type;
        this.filename = filename;
        this.startAddr = FileType.MACHINE.equals(type) ? startAddr : 0;
        this.length = length;
        this.runAddr = FileType.MACHINE.equals(type) ? runAddr : 0;
    }

    protected SerialHeader() {
    }

    /**
     * Factory method to create a serial header from parameters.
     */
    public static SerialHeader makeHeader(PocketPcDevice device, FileType type, String filename,
                                          int startAddr, int length, int runAddr) {
        if (PocketPcDevice.PC1600.equals(device)) {
            return new Pc1600Header(type, filename, startAddr, length, runAddr);
        } else {
            return new Ce158Header(type, filename, startAddr, length, runAddr);
        }
    }

    /**
     * Convenience factory for tokenized Basic programs.
     */
    public static SerialHeader makeHeader(PocketPcDevice device, String filename, int length) {
        if (PocketPcDevice.PC1600.equals(device)) {
            return new Pc1600Header(FileType.BASIC, filename, 0, length, 0);
        } else {
            return new Ce158Header(FileType.BASIC, filename, 0, length, 0);
        }
    }

    /**
     * Factory method to create a header by parsing raw bytes.
     *
     * @param headerBytes Byte array starting with the header (may contain payload bytes too)
     * @return Parsed header, or null if not recognized
     */
    public static SerialHeader makeHeader(byte[] headerBytes) {
        try {
            return new Ce158Header(headerBytes);
        } catch (IllegalArgumentException ex) {
            log.log(Level.FINE, "Not a CE-158 header");
        }
        try {
            return new Pc1600Header(headerBytes);
        } catch (IllegalArgumentException ex) {
            log.log(Level.FINE, "Not a PC-1600 header");
        }
        return null;
    }

    /**
     * Prepend a serial header if needed (machine language only).
     */
    public static byte[] prependHeaderIfNecessary(byte[] inputBytes, Integer startAddr,
                                                   Integer runAddr, PocketPcDevice device,
                                                   String filename) {
        if (startAddr == null) {
            log.log(Level.FINE, "startAddr is null, not adding a header");
            return inputBytes;
        }
        // Check if a header is already present
        SerialHeader existing = makeHeader(inputBytes);
        if (existing != null) {
            log.log(Level.FINE, "Header already present, skipping");
            return inputBytes;
        }
        int effectiveRunAddr = runAddr != null ? runAddr : 0xFFFF;
        SerialHeader newHeader = makeHeader(device, FileType.MACHINE, filename, startAddr,
                inputBytes.length, effectiveRunAddr);
        byte[] headerBytes = newHeader.getHeader();
        byte[] outputBytes = Arrays.copyOf(headerBytes, inputBytes.length + headerBytes.length);
        System.arraycopy(inputBytes, 0, outputBytes, headerBytes.length, inputBytes.length);
        return outputBytes;
    }

    /**
     * Scan {@code data} for a recognizable header and return the total number of bytes expected
     * for the complete transfer (header + payload).
     *
     * <p>Returns -1 when:
     * <ul>
     *   <li>No header magic has been found yet (need more bytes)</li>
     *   <li>The header has been found but there are not yet enough bytes to read the length field</li>
     *   <li>The data type is CE-158 VARIABLES, whose length field is always meaningless</li>
     * </ul>
     *
     * @param data bytes received so far
     * @return total expected byte count, or -1 if unknown
     */
    public static int expectedTotalBytes(byte[] data) {
        for (int i = 0; i < data.length - 3; i++) {
            // CE-158 magic: 0x01 at i, 'C' 'O' 'M' at i+2..4
            if (data[i] == 0x01 && i + 4 < data.length
                    && data[i + 2] == 0x43 && data[i + 3] == 0x4F && data[i + 4] == 0x4D) {
                if (i + 27 > data.length) {
                    return -1; // header not yet fully received
                }
                try {
                    Ce158Header h = new Ce158Header(Arrays.copyOfRange(data, i, data.length));
                    if (h.getType() == FileType.VARIABLES) {
                        return -1; // CE-158 VARIABLES length field is meaningless
                    }
                    return i + 27 + h.getLength();
                } catch (IllegalArgumentException ignored) {
                    // Not a valid header at this position — keep scanning
                }
            }
            // PC-1600 magic: 0xFF 0x10 0x00 0x00 at i
            if ((data[i] & 0xFF) == 0xFF && i + 3 < data.length
                    && data[i + 1] == 0x10 && data[i + 2] == 0x00 && data[i + 3] == 0x00) {
                if (i + 16 > data.length) {
                    return -1; // header not yet fully received
                }
                try {
                    Pc1600Header h = new Pc1600Header(Arrays.copyOfRange(data, i, data.length));
                    return i + 16 + h.getLength();
                } catch (IllegalArgumentException ignored) {
                    // Not a valid header at this position — keep scanning
                }
            }
        }
        return -1;
    }

    // ---- Abstract interface ----

    abstract char getTypeChar(FileType type);

    abstract FileType getFileType(char typeChar);

    public abstract byte[] getHeader();

    // ---- Static helpers ----

    static byte[] appendBytes(byte[] source, byte[] toAppend) {
        byte[] result = new byte[source.length + toAppend.length];
        System.arraycopy(source, 0, result, 0, source.length);
        System.arraycopy(toAppend, 0, result, source.length, toAppend.length);
        return result;
    }

    /**
     * Append an int as two bytes (big-endian, taking the two least-significant bytes).
     */
    static byte[] appendInt(byte[] source, int toAppend) {
        byte[] intermediary = ByteBuffer.allocate(4).putInt(toAppend).array();
        return appendBytes(source, new byte[]{intermediary[2], intermediary[3]});
    }

    /**
     * Append an int as three bytes in little-endian order.
     */
    static byte[] appendThreeBytesLE(byte[] source, int value) {
        return appendBytes(source, new byte[]{
                (byte) (value & 0xFF),
                (byte) ((value >> 8) & 0xFF),
                (byte) ((value >> 16) & 0xFF)
        });
    }

    /**
     * Read {@code length} bytes starting at {@code start} from {@code source} as a big-endian unsigned int.
     */
    static int makeInt(byte[] source, int start, int length) {
        if (length < 1 || length > 3) {
            throw new IllegalArgumentException("Length must be between 1 and 3");
        }
        if (start > source.length - length) {
            log.log(Level.SEVERE, "Invalid start index: {0}", start);
            return 0;
        }
        byte[] temp = new byte[4];
        for (int i = start; i < start + length; i++) {
            temp[i - start + (4 - length)] = source[i];
        }
        int mask = (1 << (length << 3)) - 1;
        return ByteBuffer.wrap(temp).getInt() & mask;
    }
}
