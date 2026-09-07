package ch.erzberger.sharppc.exchange.io;

import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

class SharpTextTest {

    @Test
    void decodesModernUtf8ListingAsUtf8() {
        byte[] data = "10 LPRINT \"für\"\n".getBytes(StandardCharsets.UTF_8);
        assertEquals("10 LPRINT \"für\"\n", SharpText.decodeBasListing(data));
    }

    @Test
    void decodesRawSharpListingAsCp437() {
        // 0x9A = Ü, 0x8E = Ä in CP437 — not valid UTF-8, so the CP437 fallback must win.
        byte[] data = {'1', '0', ' ', '"', 'f', (byte) 0x9A, 'r', ' ', 'M', (byte) 0x8E, 'r', 'z', '"', '\n'};
        assertEquals("10 \"fÜr MÄrz\"\n", SharpText.decodeBasListing(data));
    }

    @Test
    void stripsTrailingEofMarkerAndNormalizesNewlines() {
        byte[] data = {'1', '0', ' ', 'E', 'N', 'D', '\r', '\n', 0x1A};
        assertEquals("10 END\n", SharpText.decodeBasListing(data));
    }

    @Test
    void pureAsciiListingHasNoNonAsciiForPc1500() {
        assertEquals(null, SharpText.firstNonAsciiForPc1500("10 PRINT \"HI\"\n20 END\n"));
    }

    @Test
    void reportsFirstNonAsciiCharacterForPc1500() {
        int[] hit = SharpText.firstNonAsciiForPc1500("10 REM ok\n20 LPRINT \"für\"\n");
        assertEquals(2, hit[0]);          // line 2
        assertEquals(13, hit[1]);         // column of 'ü' (1-based)
        assertEquals('ü', (char) hit[2]);
    }

    @Test
    void cleanListingDecodesCp437AndTrims() {
        byte[] data = {'1', '0', ' ', '"', (byte) 0x81, '"', '\r', '\n', 0x1A, 0x1A};
        String out = SharpText.cleanListing(data);
        assertEquals("10 \"ü\"\n", out);
        assertFalse(out.contains("\u001A"));
    }
}
