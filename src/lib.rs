//! Sharp PC-1500 / PC-1600 BASIC tokenizer / de-tokenizer.
//!
//! ROM-style linear scanner (no grammar) for the offline `convert` verb. The pure core
//! (`convert`, `scanner`, `detokenize`, `detect`, `header`) has no file I/O and is shared
//! by the CLI (`src/main.rs`) and the C ABI (`ffi`).

pub mod bcd;
pub mod cpu_guess;
pub mod cp437;
pub mod detect;
pub mod detokenize;
pub mod header;
pub mod info;
pub mod keywords;
pub mod registry;
pub mod reserve;
pub mod scanner;
pub mod text;
pub mod variables;
/// Cassette-tape WAV files (PC-1500 + CE-150, PC-1600 + CE-1600P).
pub mod wav;

mod abbrev;
pub mod convert;
/// Host-file ⇄ device-bytes conversion shared by every transfer target.
pub mod transfer;

/// Calc-U-1600 `.floppy.yaml` container (two 64 KB sides).
pub mod floppy_image;
/// PC-1600 FAT filesystem on a CE-1600F floppy side.
pub mod diskfs;

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
#[cfg(feature = "serial")]
pub mod serial;
/// CLI-only: receive/accumulate/end-of-transfer logic for `get`.
#[cfg(feature = "serial")]
pub mod receiver;
/// CLI-only: paced/unpaced transmit logic for `put`.
#[cfg(feature = "serial")]
pub mod sender;
/// CLI-only: `get` orchestration.
#[cfg(feature = "serial")]
pub mod get_cmd;
/// CLI-only: `put` orchestration.
#[cfg(feature = "serial")]
pub mod put_cmd;
/// CLI-only: the cassette-WAV forms of `get`/`put`/`convert`.
pub mod wav_cmd;
/// CLI-only: `put -f wav` playback through the default audio output.
#[cfg(feature = "audio")]
pub mod audio_out;
/// CLI-only: `dir`/`del` and the disk-image forms of `get`/`put`.
#[cfg(feature = "cli")]
pub mod disk_cmd;

pub mod ffi;

pub use convert::{convert, convert_with, ConvertOutcome};
pub use detect::Content;
pub use detokenize::LineEnding;
pub use info::{classify, FileKind, FileSummary};
pub use registry::Device;
pub use scanner::SegmentMarker;

/// Crate version string (`CARGO_PKG_VERSION`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
