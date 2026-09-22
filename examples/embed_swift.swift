// Minimal Swift embedder: read a .bas file, tokenize it, print the payload as hex.
//
//   swiftc examples/embed_swift.swift -I include -L target/release -lsharpdx \
//     -o /tmp/embed_swift
//   /tmp/embed_swift tests/fixtures/depreciation.bas
//
// `include/module.modulemap` exposes the C header as `module SharpDX`.

import Foundation
import SharpDX

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: embed_swift <file.bas>\n".data(using: .utf8)!)
    exit(2)
}

let version = String(cString: sde_version())
print("sharpdx \(version)")

let input = try! Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))

var out: UnsafeMutablePointer<UInt8>? = nil
var outLen: Int = 0

let rc = input.withUnsafeBytes { raw -> Int32 in
    "sample".withCString { name in
        sde_tokenize(SDE_DEVICE_PC1500, 1, SDE_SEGMENT_MARKER_WIRE, name,
                     raw.bindMemory(to: UInt8.self).baseAddress, raw.count,
                     &out, &outLen)
    }
}

guard rc == SDE_OK, let out = out else {
    let msg = String(cString: sde_last_error())
    FileHandle.standardError.write("sde_tokenize failed (\(rc)): \(msg)\n".data(using: .utf8)!)
    exit(1)
}

let bytes = UnsafeBufferPointer(start: out, count: outLen)
print("\(outLen) bytes:")
var line = ""
for (i, b) in bytes.enumerated() {
    line += String(format: "%02x ", b)
    if i % 16 == 15 { print(line); line = "" }
}
if !line.isEmpty { print(line) }

sde_buf_free(out, outLen)
