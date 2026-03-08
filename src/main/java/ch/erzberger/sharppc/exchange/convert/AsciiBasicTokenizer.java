package ch.erzberger.sharppc.exchange.convert;

import ch.erzberger.sharpbasic.antlr.SharpBasicLexer;
import ch.erzberger.sharpbasic.antlr.SharpBasicParser;
import ch.erzberger.sharpbasic.antlr.visitor.BinaryEncodingVisitor;
import ch.erzberger.sharpbasic.core.keyword.KeywordRegistry;
import ch.erzberger.sharpbasic.core.preprocess.AbbreviationExpander;
import ch.erzberger.sharppc.exchange.cli.PocketPcDevice;
import org.antlr.v4.runtime.CharStreams;
import org.antlr.v4.runtime.CommonTokenStream;

/**
 * Converts ASCII BASIC text to PC-1500/1600 binary-tokenized payload bytes.
 * Abbreviations (e.g. P. for PRINT) are expanded before parsing.
 */
public class AsciiBasicTokenizer {
    private final KeywordRegistry registry;

    public AsciiBasicTokenizer(PocketPcDevice device) {
        this.registry = PocketPcDevice.PC1600.equals(device)
                ? KeywordRegistry.forPc1600()
                : KeywordRegistry.forPc1500();
    }

    /**
     * Tokenize ASCII BASIC text to binary payload bytes (no header).
     *
     * @param asciiBasic Full ASCII BASIC program text (may contain abbreviations)
     * @return Binary-tokenized payload bytes (no CE-158/PC-1600 header)
     */
    public byte[] tokenize(String asciiBasic) {
        // Expand dotted abbreviations line by line
        String[] lines = asciiBasic.split("\\r?\\n", -1);
        StringBuilder expanded = new StringBuilder();
        for (String line : lines) {
            // Strip leading/trailing whitespace: the ANTLR grammar expects the line
            // number to appear at column 0, but some editors and list outputs indent lines.
            expanded.append(AbbreviationExpander.expand(line.strip(), registry)).append('\n');
        }
        SharpBasicLexer lexer = new SharpBasicLexer(CharStreams.fromString(expanded.toString()));
        SharpBasicParser parser = new SharpBasicParser(new CommonTokenStream(lexer));
        SharpBasicParser.ProgramContext tree = parser.program();
        return new BinaryEncodingVisitor(registry).visitProgram(tree);
    }
}
