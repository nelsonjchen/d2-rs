// SPDX-License-Identifier: MPL-2.0
// Port of Go 1.27 compress/flate (encoder) and compress/zlib (writer).
// Go is BSD-3-Clause licensed; see THIRD_PARTY_NOTICES.md.
//! Byte-exact Go `compress/flate` and `compress/zlib` encoders.
//!
//! D2 embeds WOFF fonts whose tables are zlib streams written by Go's encoder,
//! so byte-identical SVG output needs Go's exact compressed bytes. The
//! `oracle-fixtures` feature compiles tests that read regenerated Go-oracle
//! artifacts.
//!
//! Go 1.27 replaced the classic encoder (per-level fast encoders for 1-6,
//! Huffman-table reuse across blocks, fixed-Huffman EOF blocks); this crate
//! follows the 1.27 code, whose output differs from earlier Go releases.
//!
//! Source mapping (Go 1.27.0 `src/`):
//!
//! | Rust module                | Go file                                  |
//! |----------------------------|------------------------------------------|
//! | `deflate`                  | `compress/flate/deflate.go`              |
//! | `deflatefast`              | `compress/flate/deflatefast.go`, `load_store.go` |
//! | `level1` .. `level6`       | `compress/flate/level1.go` .. `level6.go`|
//! | `huffman_bit_writer`       | `compress/flate/huffman_bit_writer.go`   |
//! | `huffman_code`             | `compress/flate/huffman_code.go`         |
//! | `token`                    | `compress/flate/token.go`                |
//! | `zlib`                     | `compress/zlib/writer.go`                |
//! | `adler32`                  | `hash/adler32/adler32.go`                |
//!
//! All levels -2..=9 are translated. Preset dictionaries and `Reset` are not.
//!
//! Go's table-reuse estimate (`tokens.EstimatedBits`) uses float32 math that
//! the arm64 compiler fuses into FMAs while amd64 (GOAMD64 < v3) does not, so
//! Go's own output is in principle host-dependent. This port follows the
//! pinned darwin/arm64 oracle (`token::GO_FUSED_FMA`); no fixture has shown an
//! arm64/amd64 difference.

pub mod adler32;
mod deflate;
mod deflatefast;
mod huffman_bit_writer;
mod huffman_code;
mod level1;
mod level2;
mod level3;
mod level4;
mod level5;
mod level6;
mod token;
pub mod zlib;

/// Go `compress/flate` encoder API.
pub mod flate {
    pub use crate::deflate::{
        BEST_COMPRESSION, BEST_SPEED, DEFAULT_COMPRESSION, HUFFMAN_ONLY, NO_COMPRESSION, Writer,
    };
}

/// `zlib.NewWriter` (DefaultCompression) + Write(data) + Close().
pub fn zlib_compress_default(data: &[u8]) -> Vec<u8> {
    zlib_compress(data, flate::DEFAULT_COMPRESSION)
}

/// `zlib.NewWriterLevel(&buf, level)` + Write(data) + Close().
///
/// # Panics
/// If `level` is outside -2..=9 (Go returns an error from NewWriterLevel).
pub fn zlib_compress(data: &[u8], level: i32) -> Vec<u8> {
    let mut z = zlib::Writer::with_level(Vec::new(), level).expect("valid zlib level");
    z.write(data).expect("writing to a Vec cannot fail");
    z.close().expect("writing to a Vec cannot fail");
    z.into_inner()
}

/// `flate.NewWriter(&buf, level)` + Write(data) + Close() (raw DEFLATE).
///
/// # Panics
/// If `level` is outside -2..=9.
pub fn flate_compress(data: &[u8], level: i32) -> Vec<u8> {
    let mut f = flate::Writer::new(Vec::new(), level).expect("valid flate level");
    f.write(data).expect("writing to a Vec cannot fail");
    f.close().expect("writing to a Vec cannot fail");
    f.into_inner()
}
