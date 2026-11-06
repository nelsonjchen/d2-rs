// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg markdown_cards.go: leaf Markdown documents as paper cards.

use std::cell::RefCell;
use std::rc::Rc;

use d2_rs_scene::go_min;

use super::geometry::{MeshBuilder, native_material, native_paint, native_token};
use super::mesh::{OpacityGroup, nv};
use super::native_shapes::{native_canonical_height, native_face_source};
use crate::scene::Node;
use crate::target::{SHAPE_RECTANGLE, SHAPE_TEXT};

/// Leaf Markdown is a document in its own source allocation.
pub fn native_markdown_card(n: &Node) -> bool {
    let language = n.metadata.original.text.language.trim().to_lowercase();
    !n.container
        && n.sequence_role.is_empty()
        && n.type_ == SHAPE_TEXT
        && (language == "markdown" || language == "md")
}

pub const MARKDOWN_CARD_PAPER_DEPTH: f64 = 0.035;

pub fn native_plain_markdown_card(n: &Node) -> bool {
    let s = &n.metadata.original;
    native_markdown_card(n)
        && native_paint(&n.fill, "transparent").a == 0
        && !n.stroke_explicit
        && n.stroke_width == 2
        && n.stroke_dash == 0.0
        && s.border_radius == 0
        && s.fill_pattern.is_empty()
        && !s.double_border
        && !s.multiple
        && !s.three_dee
}

pub fn native_markdown_card_inset(n: &Node) -> f64 {
    go_min(0.045, go_min(n.size.x / 4.0, n.size.z / 4.0))
}

/// The default transparent Markdown face becomes paper.
pub fn native_markdown_card_paint(n: &Node) -> Node {
    let mut n = n.clone();
    if native_paint(&n.fill, "transparent").a == 0 {
        n.fill = "#fffdf6".into();
    }
    n.fill_explicit = true;
    if !n.stroke_explicit
        && (n.metadata.original.stroke.is_empty() || native_token(&n.metadata.original.stroke))
    {
        n.stroke = "#304552".into();
        if n.stroke_width == 2 {
            n.stroke_width = 1;
        }
        n.stroke_explicit = true;
    }
    n
}

impl MeshBuilder {
    pub fn markdown_card(&mut self, n: &Node, tint: &str) {
        if self.err.is_some() || n.opacity <= 0.0 || n.size.x <= 0.0 || n.size.z <= 0.0 {
            return;
        }
        let first = self.triangles.len();
        let mut n = n.clone();
        let mut group = None;
        if n.opacity < 1.0 {
            // Compose the recessed support, paper and printing before fading
            // the document once.
            group = Some(Rc::new(RefCell::new(OpacityGroup { opacity: n.opacity })));
            n.opacity = 1.0;
        }
        self.markdown_card_body(&n, tint);
        if let Some(group) = group {
            for t in &mut self.triangles[first..] {
                t.opacity_group = Some(group.clone());
            }
        }
    }

    fn markdown_card_body(&mut self, n: &Node, tint: &str) {
        let paint = native_markdown_card_paint(n);
        // Decorative borders and modifiers use the canonical source face.
        if !native_plain_markdown_card(n) {
            let first = self.triangles.len();
            self.canonical_node(&paint, tint);
            self.classic_ink_edges(&paint, first);
            return;
        }
        let (x0, x1) = (n.position.x - n.size.x / 2.0, n.position.x + n.size.x / 2.0);
        let (z0, z1) = (n.position.z - n.size.z / 2.0, n.position.z + n.size.z / 2.0);
        let floor = n.position.y - n.size.y / 2.0;
        let top = floor + native_canonical_height(n, self.scale);
        let inset = native_markdown_card_inset(n);
        self.markdown_card_slab(
            &paint,
            [x0 + inset, z0 + inset, x1 - inset, z1 - inset],
            floor + self.node_support_drop,
            top - MARKDOWN_CARD_PAPER_DEPTH,
            ["#d7d8d3", "#b6c0c0", "#7b8a90"],
        );
        let fill = paint.fill.clone();
        let stroke = paint.stroke.clone();
        self.markdown_card_slab(
            &paint,
            [x0, z0, x1, z1],
            top - MARKDOWN_CARD_PAPER_DEPTH,
            top,
            [&fill, "#e4e6e1", &stroke],
        );
        // Reuse the complete rich document, icon and link paths at the
        // raised writing plane.
        self.canonical_node_content(n, &native_face_source(n, &fill), &fill, top);
    }

    fn markdown_card_slab(
        &mut self,
        n: &Node,
        [x0, z0, x1, z1]: [f64; 4],
        bottom: f64,
        top: f64,
        [fill, side, stroke]: [&str; 3],
    ) {
        let first = self.triangles.len();
        let profile = [
            nv(x0, top, z0),
            nv(x1, top, z0),
            nv(x1, top, z1),
            nv(x0, top, z1),
        ];
        let top_material = native_material(fill, 0.80, 0.0, 1.0);
        self.extruded_profile(
            &profile,
            bottom,
            Some(&top_material),
            &native_material(side, 0.78, 0.0, 1.0),
        );
        let mut ink = n.clone();
        ink.type_ = SHAPE_RECTANGLE.into();
        ink.fill = fill.into();
        ink.stroke = stroke.into();
        ink.stroke_width = 1;
        ink.stroke_explicit = true;
        ink.metadata.original.type_ = ink.type_.clone();
        ink.metadata.original.fill = fill.into();
        ink.metadata.original.stroke = stroke.into();
        ink.metadata.original.stroke_width = 1;
        ink.metadata.original.text.label.clear();
        self.classic_ink_edges(&ink, first);
    }
}
