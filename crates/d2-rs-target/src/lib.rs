// SPDX-License-Identifier: MPL-2.0
//! The compiled diagram model: a Rust port of upstream D2's `d2target.Diagram`
//! JSON boundary (`target`) and D2 theme catalog resolution with overrides
//! (`theme`), shared by the scene-building and isometric renderers.

pub mod target;
pub mod theme;

pub use target::*;
