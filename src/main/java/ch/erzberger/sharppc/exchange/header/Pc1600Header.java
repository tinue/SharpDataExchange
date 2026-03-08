package ch.erzberger.sharppc.exchange.header;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

/**
 * Represents a PC-1600 serial header.
 *
 * <p>Header structure (16 bytes):
 * <pre>
 *   0x00  Magic: 0xFF 0x10 0x00 0x00
 *   0x04  Type byte: 0x21=BASIC, 0x10=MACHINE, 0x48=VARIABLES
 *   0x05  Data length (3 bytes, little-endian)
 *   0x08  Load start address (3 bytes, little-endian, MACHINE only)
 *   0x0B  Auto-run address (3 bytes, little-endian, MACHINE only)
 *   0x0E  End marker: 0x00 0x0F
 * </pre>
 *
 * <p>Note: The type byte for VARIABLES (0x48) is assumed to be the same as for
 * the PC-1500, pending hardware verification. The RESERVE type is NOT supported
 * via serial/COM on the PC-1600.
 */
@Log
public class Pc1600Header extends SerialHeader {

    private static final int HEADER_SIZE = 16;

    protected Pc1600Header(FileType type, String filename, int startAddr, int length, int runAddr) {
        super(type, filename, startAddr, length, runAddr);
        super.device = PocketPcDevice.PC1600;
        if (type == FileType.RESERVE) {
            throw new UnsupportedOperationException(
                    "PC-1600 RESERVE type is not supported via COM (only via tape)");
        }
    }

    protected Pc1600Header(byte[] header) {
        if (header.length < HEADER_SIZE) {
            throw new IllegalArgumentException(
                    "Too short for a PC-1600 header: " + header.length + " bytes");
        }
        if (!((header[0] & 0xFF) == 0xFF && header[1] == 0x10 && header[2] == 0 && header[3] == 0)) {
            throw new IllegalArgumentException("PC-1600 magic marker missing");
        }
        super.device = PocketPcDevice.PC1600;
        super.type = getFileType((char) (header[4] & 0xFF));

        // Length: bytes 5-7, little-endian
        this.length = readThreeBytesLE(header, 5);
        // Start address: bytes 8-10, little-endian
        this.startAddr = FileType.MACHINE.equals(type) ? readThreeBytesLE(header, 8) : 0;
        // Run address: bytes 11-13, little-endian
        this.runAddr = FileType.MACHINE.equals(type) ? readThreeBytesLE(header, 11) : 0;
    }

    @Override
    public byte[] getHeader() {
        byte[] headerBytes = new byte[]{(byte) 0xFF, 0x10, 0x00, 0x00, (byte) getTypeChar(type)};
        headerBytes = appendThreeBytesLE(headerBytes, length);
        headerBytes = appendThreeBytesLE(headerBytes, startAddr);
        headerBytes = appendThreeBytesLE(headerBytes, runAddr);
        headerBytes = appendBytes(headerBytes, new byte[]{0x00, 0x0F});
        return headerBytes;
    }

    @Override
    char getTypeChar(FileType type) {
        return switch (type) {
            case BASIC -> (char) 0x21;
            case MACHINE -> (char) 0x10;
            case VARIABLES -> 'H';
            default -> throw new UnsupportedOperationException(
                    "PC-1600 type char for " + type + " is unknown or unsupported");
        };
    }

    @Override
    FileType getFileType(char typeChar) {
        return switch (typeChar) {
            case 0x21 -> FileType.BASIC;
            case 0x10 -> FileType.MACHINE;
            case 'H' -> FileType.VARIABLES;
            default -> throw new IllegalArgumentException(
                    "Unknown or unsupported PC-1600 type char: 0x" + Integer.toHexString(typeChar));
        };
    }

    /** Read three bytes starting at {@code offset} as an unsigned little-endian integer. */
    private static int readThreeBytesLE(byte[] data, int offset) {
        return (data[offset] & 0xFF)
                | ((data[offset + 1] & 0xFF) << 8)
                | ((data[offset + 2] & 0xFF) << 16);
    }
}
