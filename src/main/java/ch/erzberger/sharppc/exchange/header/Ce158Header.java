package ch.erzberger.sharppc.exchange.header;

import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import lombok.extern.java.Log;

import java.io.UnsupportedEncodingException;
import java.util.Arrays;
import java.util.logging.Level;

/**
 * Represents a CE-158 serial header (PC-1500/PC-1500A).
 *
 * <p>Header structure (27 bytes):
 * <pre>
 *   0x00  Magic: 0x01
 *   0x01  Type char: '@'=BASIC, 'A'=RESERVE, 'B'=MACHINE, 'H'=VARIABLES
 *   0x02  'C'
 *   0x03  'O'
 *   0x04  'M'
 *   0x05  Filename (16 bytes, CP437, space-padded)
 *   0x15  Load start address (2 bytes, used for MACHINE only)
 *   0x17  Data length (2 bytes)
 *   0x19  Auto-run address (2 bytes, used for MACHINE only)
 * </pre>
 */
@Log
public class Ce158Header extends SerialHeader {

    protected Ce158Header(FileType type, String filename, int startAddr, int length, int runAddr) {
        super(type, filename, startAddr, length, runAddr);
        super.device = PocketPcDevice.PC1500;
    }

    protected Ce158Header(byte[] header) {
        if (header.length < 27) {
            throw new IllegalArgumentException(
                    "Too short for a CE-158 header: " + header.length + " bytes");
        }
        if (!(header[0] == 1 && header[2] == 0x43 && header[3] == 0x4F && header[4] == 0x4D)) {
            throw new IllegalArgumentException("CE-158 magic marker missing");
        }
        super.device = PocketPcDevice.PC1500;
        super.type = getFileType((char) header[1]);
        byte[] nameBytes = Arrays.copyOfRange(header, 0x05, 0x15);
        try {
            this.filename = new String(nameBytes, "Cp437").trim();
        } catch (UnsupportedEncodingException e) {
            throw new NoClassDefFoundError("Code page 437 not available on this JVM");
        }
        this.startAddr = FileType.MACHINE.equals(type) ? makeInt(header, 0x15, 2) : 0;
        this.length = makeInt(header, 0x17, 2);
        this.runAddr = FileType.MACHINE.equals(type) ? makeInt(header, 0x19, 2) : 0;
    }

    @Override
    public byte[] getHeader() {
        String normalizedFilename = filename.length() > 16
                ? filename.substring(0, 16)
                : filename;

        // Magic + type char + "COM" + filename
        String typeAndFilename = getTypeChar(type) + "COM" + normalizedFilename;
        byte[] headerBytes = new byte[]{0x01};
        try {
            headerBytes = appendBytes(headerBytes, typeAndFilename.getBytes("Cp437"));
        } catch (UnsupportedEncodingException e) {
            log.log(Level.SEVERE, "Code page 437 not available on this JVM");
            System.exit(-1);
        }

        // Pad filename to exactly 16 bytes
        int padCount = 16 - normalizedFilename.length();
        if (padCount > 0) {
            headerBytes = appendBytes(headerBytes, new byte[padCount]);
        }

        headerBytes = appendInt(headerBytes, startAddr);
        headerBytes = appendInt(headerBytes, length);
        headerBytes = appendInt(headerBytes, runAddr);
        return headerBytes;
    }

    @Override
    char getTypeChar(FileType type) {
        return switch (type) {
            case BASIC -> '@';
            case RESERVE -> 'A';
            case MACHINE -> 'B';
            case VARIABLES -> 'H';
        };
    }

    @Override
    FileType getFileType(char typeChar) {
        return switch (typeChar) {
            case '@' -> FileType.BASIC;
            case 'A' -> FileType.RESERVE;
            case 'B' -> FileType.MACHINE;
            case 'H' -> FileType.VARIABLES;
            default -> throw new IllegalArgumentException(
                    "Unknown CE-158 type char: " + typeChar);
        };
    }
}
