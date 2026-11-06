// SPDX-License-Identifier: MPL-2.0
//! The `d2-rs` command-line tool, a Rust port of upstream D2's `d2cli` package.
//!
//! The binary (`src/main.rs`) compiles, lays out and exports diagrams with the
//! in-process engines; this library holds command helpers shared with it, such
//! as the `lib/imgbundler` port. `oracle-fixtures` compiles the tests that read
//! regenerated Go-oracle artifacts, and `diagnostic-traces` enables diagnostic
//! traces in the pipeline crates.
pub mod imgbundler;
