// SPDX-License-Identifier: MPL-2.0
//! The isometric renderer: a Rust port of upstream D2's
//! `d2renderers/d2isometric` and `d2isometricimg` packages. Its input is the
//! compiled `d2target.Diagram` JSON.
//!
//! The `diagnostic-traces` feature enables renderer traces; `oracle-fixtures`
//! compiles tests that read regenerated Go-oracle artifacts.

pub use d2_rs_target::{target, theme};
pub mod document;
pub mod img;
pub mod scene;
pub mod svg;
