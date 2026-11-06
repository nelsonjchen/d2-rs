// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg native_style.go.

use super::geometry::native_token;
use crate::scene::Node;
use crate::target::{SHAPE_CLASS, SHAPE_SQL_TABLE};

/// Theme-default bodies use the classic study's restrained ink palette.
pub fn native_classic_node(n: &Node) -> Node {
    let mut n = n.clone();
    if n.type_ == SHAPE_CLASS || n.type_ == SHAPE_SQL_TABLE {
        return n; // Structured documents own their header/body color contract.
    }
    if !n.stroke_explicit && native_token(&n.metadata.original.stroke) {
        n.stroke = "#263c4e".into();
    }
    n
}
