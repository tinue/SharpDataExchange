//! Sharp PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer.
//!
//! ROM-style linear scanner (no grammar), reproducing the offline `convert` verb of the
//! Java `SharpDataExchange`. The pure core (`convert`, `scanner`, `detokenize`, `detect`,
//! `header`) has no file I/O and is shared by the CLI (`src/main.rs`) and the C ABI
//! (`ffi`).

pub mod cp437;
pub mod detect;
pub mod detokenize;
pub mod header;
pub mod keywords;
pub mod registry;
pub mod scanner;
pub mod text;

mod abbrev;
pub mod convert;

/// CLI-only filesystem glue (extension rules, output-path derivation). Kept in the
/// library for module wiring, but never called from [`ffi`].
pub mod paths;

/// CLI-only: the four `get`/`put` transport targets (superset of [`registry::Device`]).
pub mod pocket_device;
/// CLI-only: shared `get`/`put` filename helpers.
pub mod filename;
/// CLI-only: per-user config file (`~/.sderc`) for `get`/`put` defaults.
pub mod config;
/// CLI-only: `-v`/`-q`/config-file verbosity precedence, shared by `get`/`put`.
pub mod verbosity;
/// CLI-only: serial transport abstraction (`get`/`put`), real and in-memory.
pub mod serial;
/// CLI-only: receive/accumulate/end-of-transfer logic for `get`.
pub mod receiver;
/// CLI-only: paced/unpaced transmit logic for `put`.
pub mod sender;
/// CLI-only: `get` orchestration.
pub mod get_cmd;
/// CLI-only: `put` orchestration.
pub mod put_cmd;

pub mod ffi;

pub use convert::{convert, convert_with, ConvertOutcome};
pub use detect::Content;
pub use detokenize::LineEnding;
pub use registry::Device;
pub use scanner::SegmentMarker;

/// Crate version string (`CARGO_PKG_VERSION`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
