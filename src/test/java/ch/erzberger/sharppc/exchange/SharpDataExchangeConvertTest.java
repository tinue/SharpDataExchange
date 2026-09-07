package ch.erzberger.sharppc.exchange;

import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

/**
 * End-to-end tests for the {@code convert} verb, driving {@link SharpDataExchange#main(String[])}
 * against the fixtures in {@code testsuite/}.
 */
class SharpDataExchangeConvertTest {

    private static final Path FIXTURES = Path.of("testsuite");
    private static final Pattern LINE_NUMBER = Pattern.compile("^\\s*(\\d+)\\s");

    @Test
    @DisplayName("convert tokenizes ASCII BASIC and wraps it in a CE-158 header")
    void asciiToTokenized(@TempDir Path tmp) throws IOException {
        Path in = tmp.resolve("depreciation.bas");
        Files.copy(FIXTURES.resolve("depreciation.bas"), in);
        Path out = tmp.resolve("out.bas");

        SharpDataExchange.main(new String[]{"convert", in.toString(), out.toString()});

        assertTrue(Files.exists(out), "output file was not written");
        byte[] bytes = Files.readAllBytes(out);
        // CE-158 magic: 0x01 at [0], 'C' 'O' 'M' at [2..4]
        assertEquals(0x01, bytes[0] & 0xFF);
        assertEquals('C', bytes[2]);
        assertEquals('O', bytes[3]);
        assertEquals('M', bytes[4]);
        assertTrue(bytes.length > 100, "tokenized output is implausibly short");
    }

    @Test
    @DisplayName("convert de-tokenizes a header-carrying tokenized program to ASCII")
    void tokenizedToAscii(@TempDir Path tmp) throws IOException {
        Path in = tmp.resolve("dep-tok.bas");
        Files.copy(FIXTURES.resolve("depreciation-tokenized-ce158header.bin"), in);

        SharpDataExchange.main(new String[]{"convert", in.toString()});

        Path out = tmp.resolve("dep-tok_ascii.bas");
        assertTrue(Files.exists(out), "default ASCII output file was not written");
        String text = Files.readString(out, StandardCharsets.UTF_8);
        String firstLine = text.lines().filter(l -> !l.isBlank()).findFirst().orElseThrow();
        assertTrue(LINE_NUMBER.matcher(firstLine).find(),
                "first non-blank line is not a numbered BASIC line: " + firstLine);
    }

    @Test
    @DisplayName("convert round-trips ASCII -> tokenized -> ASCII: same line numbers, stable on a second pass")
    void roundTrip(@TempDir Path tmp) throws IOException {
        Path src = tmp.resolve("src.bas");
        Files.copy(FIXTURES.resolve("depreciation.bas"), src);

        Path tok1 = tmp.resolve("tok1.bas");
        SharpDataExchange.main(new String[]{"convert", src.toString(), tok1.toString()});
        Path ascii1 = tmp.resolve("ascii1.bas");
        SharpDataExchange.main(new String[]{"convert", tok1.toString(), ascii1.toString()});

        // The de-tokenized listing carries the same sequence of line numbers as the source.
        assertEquals(lineNumbers(Files.readString(src, StandardCharsets.UTF_8)),
                lineNumbers(Files.readString(ascii1, StandardCharsets.UTF_8)));

        // A second tokenize/de-tokenize pass reproduces the first listing byte-for-byte.
        Path tok2 = tmp.resolve("tok2.bas");
        SharpDataExchange.main(new String[]{"convert", ascii1.toString(), tok2.toString()});
        Path ascii2 = tmp.resolve("ascii2.bas");
        SharpDataExchange.main(new String[]{"convert", tok2.toString(), ascii2.toString()});

        assertEquals(Files.readString(ascii1, StandardCharsets.UTF_8),
                Files.readString(ascii2, StandardCharsets.UTF_8));
    }

    private static List<String> lineNumbers(String text) {
        return Arrays.stream(text.split("\\r?\\n"))
                .map(LINE_NUMBER::matcher)
                .filter(Matcher::find)
                .map(m -> m.group(1))
                .toList();
    }
}
