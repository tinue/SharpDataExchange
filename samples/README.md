# Samples
A collection of sample Basic programs, so that you have something to load!

# ce163f.bas
Def-A will print information about the currently selected memory bank. Works on a PC-1500A only, the PC-1500 needs different addresses.

# ce158x-linelength-diagnostic.bas
Diagnostic program for tracking down systematic (non-random) byte corruption on a CE-158X serial link. Lines 10–99, each `PRINT"0123456789…"` with a cyclic digit string, sized so every tokenized line is exactly 50 bytes on the wire — except line 10, shortened to 23 bytes to absorb the 27-byte CE-158 header. As a result, the cumulative byte count from the very start of the transfer (header included) lands on a clean multiple of 50 after every line: `line = 9 + offset / 50`. If a byte at wire offset `P` comes back wrong, this pinpoints the line without needing a capture tool that tracks line boundaries — and since the expected digit at any position is just `position mod 10`, corruption is visible by eye against the repeating pattern. Send with `put` (default binary/tokenized), or use `put --dry-run <file>` to inspect the exact bytes without touching the serial port.
