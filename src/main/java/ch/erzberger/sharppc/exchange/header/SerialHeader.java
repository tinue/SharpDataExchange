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
