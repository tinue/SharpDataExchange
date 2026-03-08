# SharpDataExchange Scripts

This directory contains the compiled JAR file (after running `mvn install`) and sample wrapper scripts to run the tool as a command.

## Wrapper Scripts

- `sde`: Shell script for macOS, Linux, and other Unix-like systems.
- `sde.ps1`: PowerShell script for Windows.

## Installation

1. Run `mvn install` to build the `SharpDataExchange.jar` file into this directory.
2. Copy the appropriate script for your system to a directory in your `PATH` (e.g., `~/bin` or `/usr/local/bin`).
3. **Update the path** inside the copied script to point to the actual location of `SharpDataExchange.jar` on your system.

Example for `sde` (Unix):
```bash
#!/bin/sh
java -jar /path/to/your/project/bin/SharpDataExchange.jar "$@"
```
