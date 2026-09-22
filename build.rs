//! Generate the C header (`include/sharpdx.h`) from `src/ffi/` via cbindgen.
//!
//! The header is committed so C/Swift embedders don't need the Rust toolchain; a test
//! (`tests/ffi.rs::header_is_current`) re-runs this and asserts the committed copy matches.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/ffi");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let ffi = Path::new(&crate_dir).join("src/ffi/mod.rs");
    if !ffi.exists() {
        return;
    }
    let out = Path::new(&crate_dir).join("include/sharpdx.h");

    let config = cbindgen::Config::from_file(Path::new(&crate_dir).join("cbindgen.toml"))
        .unwrap_or_default();

    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(bindings) => {
            bindings.write_to_file(&out);
        }
        Err(e) => {
            // Don't hard-fail the build on a transient cbindgen parse issue; the drift
            // test will still catch a stale header.
            println!("cargo:warning=cbindgen failed: {e}");
        }
    }
}
