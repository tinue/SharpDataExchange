package ch.erzberger.sharppc.exchange.cli;

public enum PocketPcDevice {
    PC1500, PC1500A, PC1600, PC1600EMUL;

    public boolean isPC1500() {
        return this == PC1500 || this == PC1500A;
    }

    public boolean isPC1600() {
        return this == PC1600 || this == PC1600EMUL;
    }

    /**
     * True when the target is an emulator reached over a host pseudo-terminal
     * (e.g. /dev/ttysNNN) rather than real hardware on a USB/serial adapter.
     */
    public boolean isEmulator() {
        return this == PC1600EMUL;
    }

    /**
     * True when the physical link provides RTS/CTS handshake lines. Emulator
     * pseudo-terminals have no modem-control lines, so this is false for them.
     */
    public boolean hasHardwareFlowControl() {
        return this == PC1600;
    }

    /**
     * True when the send side must pace bytes itself because there is no flow
     * control to throttle it: the PC-1500 (CE-158 has no handshake) and the
     * emulator (pseudo-terminal has no handshake).
     */
    public boolean isPacedSend() {
        return isPC1500() || isEmulator();
    }
}
