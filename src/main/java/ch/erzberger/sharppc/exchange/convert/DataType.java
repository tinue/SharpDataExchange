package ch.erzberger.sharppc.exchange.convert;

/**
 * Content-based data type as determined by ContentDetector.
 */
public enum DataType {
    BINARY_BASIC,
    ASCII_BASIC,
    /** Reserve Area (PC-1500 only) */
    BINARY_RESERVE,
    /** Reserve Area (PC-1500 only) */
    ASCII_RESERVE,
    BINARY_VARS,
    ASCII_VARS,
    MACHINE,
    UNKNOWN
}
