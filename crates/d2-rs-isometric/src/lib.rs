// SPDX-License-Identifier: MPL-2.0
//! Native port of D2's isometric renderer at the pinned master
//! (d2renderers/d2isometric and d2isometricimg). The input boundary is the
//! compiled d2target.Diagram JSON; see SOURCE.md for the source mapping.

pub use d2_rs_target::{target, theme};
pub mod document;
pub mod img;
pub mod scene;
pub mod svg;
