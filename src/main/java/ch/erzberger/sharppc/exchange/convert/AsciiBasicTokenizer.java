package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharpbasic.antlr.SharpBasicLexer;
import ch.erzberger.sharpbasic.antlr.SharpBasicParser;
import ch.erzberger.sharpbasic.antlr.SpaceNormalizer;
import ch.erzberger.sharpbasic.antlr.visitor.BinaryEncodingVisitor;
import ch.erzberger.sharpbasic.core.keyword.KeywordRegistry;
import ch.erzberger.sharpbasic.core.preprocess.AbbreviationExpander;
import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.antlr.v4.runtime.CharStreams;
import org.antlr.v4.runtime.CommonTokenStream;

/**
 * Converts ASCII BASIC text to PC-1500/1600 binary-tokenized payload bytes.
 *
 * <p>Processing pipeline (mirrors RoundTripChecker.emitBinaryFromSource):
 * <ol>
 *   <li>Expand dotted abbreviations line by line via {@link AbbreviationExpander}.</li>
 *   <li>Normalize keyword spacing and strip leading whitespace via
 *       {@link SpaceNormalizer#forSource(String)} — the recommended library
 *       entry point. This satisfies the {@code LINE_NUMBER} semantic predicate
 *       ({@code getCharPositionInLine() == 0}) required by the ANTLR grammar.</li>
 *   <li>Parse and encode with {@link SharpBasicLexer}, {@link SharpBasicParser},
 *       and {@link BinaryEncodingVisitor}.</li>
 * </ol>
 */
public class AsciiBasicTokenizer {
    private final KeywordRegistry registry;

    public AsciiBasicTokenizer(PocketPcDevice device) {
        this.registry = device != null && device.isPC1600()
                ? KeywordRegistry.forPc1600()
                : KeywordRegistry.forPc1500();
    }

    /**
     * Tokenize ASCII BASIC text to binary payload bytes (no header).
     *
     * @param asciiBasic Full ASCII BASIC program text (may contain dotted abbreviations
     *                   and leading whitespace before line numbers)
     * @return Binary-tokenized payload bytes (no CE-158/PC-1600 header)
     */
    public byte[] tokenize(String asciiBasic) {
        // Step 1: expand dotted abbreviations (e.g. P. → PRINT)
        String[] lines = asciiBasic.split("\\r?\\n", -1);
        StringBuilder expanded = new StringBuilder();
        for (String line : lines) {
            expanded.append(AbbreviationExpander.expand(line, registry)).append('\n');
        }

        // Step 2: normalize keyword spacing and strip leading whitespace per line.
        // SpaceNormalizer.forSource() is the recommended library entry point and
        // handles the LINE_NUMBER precondition internally.
        String normalized = SpaceNormalizer.forSource(expanded.toString()).normalize();

        // Step 3: parse and encode
        SharpBasicLexer lexer = new SharpBasicLexer(CharStreams.fromString(normalized));
        SharpBasicParser parser = new SharpBasicParser(new CommonTokenStream(lexer));
        SharpBasicParser.ProgramContext tree = parser.program();
        return new BinaryEncodingVisitor(registry).visitProgram(tree);
    }
}
