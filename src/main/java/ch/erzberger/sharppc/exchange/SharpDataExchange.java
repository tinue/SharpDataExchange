package ch.erzberger.sharppc.exchange;

import java.util.logging.LogManager;

public class SharpDataExchange {

    public static void main(String[] args) {
        try {
            LogManager.getLogManager().readConfiguration(
                    SharpDataExchange.class.getResourceAsStream("/logging.properties"));
        } catch (Exception e) {
            System.err.println("Warning: could not load logging configuration");
        }
        System.out.println("SharpDataExchange not yet implemented");
        System.exit(1);
    }
}
