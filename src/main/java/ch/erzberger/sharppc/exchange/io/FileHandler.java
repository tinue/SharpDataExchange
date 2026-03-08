package ch.erzberger.sharppc.exchange.io;

import lombok.extern.java.Log;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.logging.Level;

/**
 * Reads and writes files for SharpDataExchange. No clipboard support.
 */
@Log
public class FileHandler {

    private FileHandler() {}

    /**
     * Read a file as raw bytes.
     *
     * @param filePath path to the file
     * @return file contents, or an empty array on error
     */
    public static byte[] readBinaryFile(String filePath) {
        try {
            return Files.readAllBytes(Path.of(filePath));
        } catch (IOException e) {
            log.log(Level.SEVERE, "Cannot read file: {0}", filePath);
            return new byte[0];
        }
    }

    /**
     * Write text to a file (UTF-8). Overwrites any existing file.
     *
     * @param filePath path to write
     * @param text     content to write
     */
    public static void writeText(String filePath, String text) {
        try {
            Files.writeString(Path.of(filePath), text);
        } catch (IOException e) {
            log.log(Level.SEVERE, "Cannot write file: {0}", filePath);
        }
    }

    /**
     * Write raw bytes to a file. Overwrites any existing file.
     *
     * @param filePath path to write
     * @param data     bytes to write
     */
    public static void writeBinary(String filePath, byte[] data) {
        try {
            Files.write(Path.of(filePath), data);
        } catch (IOException e) {
            log.log(Level.SEVERE, "Cannot write file: {0}", filePath);
        }
    }
}
