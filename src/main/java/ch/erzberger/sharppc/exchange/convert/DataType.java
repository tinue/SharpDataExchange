package ch.erzberger.sharppc.exchange.convert;

/**
 * Content-based data type as determined by ContentDetector.
 */
public enum DataType {
    BINARY_BASIC,
    ASCII_BASIC,
    BINARY_RESERVE,
    ASCII_RESERVE,
    BINARY_VARS,
    ASCII_VARS,
    MACHINE,
    UNKNOWN
}
