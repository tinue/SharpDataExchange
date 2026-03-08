# SharpDataExchange Scripts

This directory contains the compiled JAR file (after running `mvn install`) and sample wrapper scripts to run the tool as a command.

## Wrapper Scripts

- `sde`: Shell script for macOS, Linux, and other Unix-like systems.
- `sde.ps1`: PowerShell script for Windows.
- `sder`: Remote execution wrapper. Use this to work around the macOS USB/Serial bug when communicating with a PC-1600. It offloads the serial communication to a remote Linux machine (e.g., a Raspberry Pi) via SSH/SCP.

## Installation

1. Run `mvn install` to build the `SharpDataExchange.jar` file into this directory.
2. Copy the appropriate script for your system to a directory in your `PATH` (e.g., `~/bin` or `/usr/local/bin`).
3. **Update the paths and REMOTE_HOST** inside the copied script to point to the actual locations on your system and your remote hardware.

Example for `sde` (Unix):
```bash
#!/bin/sh
java -jar /path/to/your/project/bin/SharpDataExchange.jar "$@"
```

Example for `sder` (Remote):
The `sder` script requires `ssh` and `scp` access to a remote host where the serial adapter is connected. It automatically transfers the JAR and data files, executes the command remotely, and (for `get`) fetches the resulting file back to your local machine.
