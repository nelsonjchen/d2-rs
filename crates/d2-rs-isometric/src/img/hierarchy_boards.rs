// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg hierarchy_boards.go: container plates,
//! terraces and hierarchy relief. Contraction sites are marked `ARM64:`.

use std::collections::HashMap;

use d2_rs_render::go_float::{go_fmadd, go_fmsub};
use d2_rs_scene::go_max;

use super::geometry::{
    MeshBuilder, native_material, native_paint, readable_surface_ink, tint_dark,
};
use super::gradient_paints::native_gradient_opacity;
use super::hierarchy_support::hierarchy_strong_terraces;
use super::markdown_cards::native_markdown_card;
use super::mesh::{MaterialRef, Vec, nunit, nv};
use super::native_shapes::{
    NativeAnimatedNode, native_canonical_height, native_face_source, native_relief_symbol,
    native_shape_profiles,
};
use super::native_solids::{native_solid_height, native_solid_node};
use super::native_structured::native_structured_node;
use super::rich_labels::rich_color;
use crate::scene::{Board, Node};
use crate::target::{MULTIPLE_OFFSET, THREE_DEE_OFFSET};

/// Presentation copies can lift a nested plate within the clearance below
/// the common component and route planes.
pub fn hierarchy_surface_y(board: &Board) -> f64 {
    let mut y = hierarchy_base_surface_y(board);
    if board.kind != "sequence-group" {
        y += board.position.y;
    }
    y
}

pub fn hierarchy_base_surface_y(board: &Board) -> f64 {
    // ARM64 hierarchy_boards.go:25.
    go_fmadd(0.max(128.min(board.level)) as f64, 0.0001, 0.028)
}

/// Leaves start at .07; routes remain at .08.
pub const HIERARCHY_TERRACE_CEILING: f64 = 0.066;
pub const HIERARCHY_RELIEF: f64 = 0.34;

/// Organizational panels stay shallow, while volume-bearing source shapes
/// retain enough height to read as cylinders and prisms.
pub fn hierarchy_node_relief(node: &Node) -> f64 {
    if native_structured_node(node) || native_markdown_card(node) {
        return 1.0;
    }
    if native_solid_node(node) {
        return 0.60;
    }
    if native_relief_symbol(node) {
        return 1.0;
    }
    HIERARCHY_RELIEF
}

pub fn hierarchy_render_nodes(
    nodes: &[Node],
    supports: Option<&HashMap<String, f64>>,
) -> std::vec::Vec<Node> {
    let mut out = nodes.to_vec();
    for n in &mut out {
        if n.container {
            continue;
        }
        let floor = n.position.y - n.size.y / 2.0;
        let mut height = n.size.y;
        if native_solid_node(n) {
            height = native_solid_height(n);
        } else if native_relief_symbol(n) || native_structured_node(n) || native_markdown_card(n) {
            height = native_canonical_height(n, 0.0);
        }
        n.size.y = height * hierarchy_node_relief(n);
        n.position.y = floor + n.size.y / 2.0;
        if let Some(supports) = supports {
            let drop = supports.get(&n.board_id).copied().unwrap_or(0.0);
            n.size.y -= drop;
            n.position.y += drop / 2.0;
        }
    }
    out
}

pub fn hierarchy_spacer(node: &Node) -> bool {
    node.label.trim().is_empty()
        && node.icon.is_empty()
        && native_paint(&node.fill, "#edf1f7").a == 0
        && (node.stroke_width == 0 || native_paint(&node.stroke, "#849ebc").a == 0)
}

/// Blank white/transparent grid wrappers carry layout, not a physical
/// boundary.
pub fn hierarchy_layout_wrapper(owner: Option<&Node>) -> bool {
    let Some(owner) = owner else {
        return false;
    };
    if !owner.label.trim().is_empty() || !owner.icon.is_empty() {
        return false;
    }
    let (fill, stroke) = (
        native_paint(&owner.fill, "white"),
        native_paint(&owner.stroke, "#849ebc"),
    );
    let quiet_fill = fill.a == 0 || fill.r >= 245 && fill.g >= 245 && fill.b >= 245;
    let quiet_stroke = owner.stroke_width == 0 || stroke.a == 0 || stroke == fill;
    quiet_fill && quiet_stroke
}

pub type NodeIndex<'a> = HashMap<&'a str, &'a Node>;

pub fn hierarchy_presentation_boards(boards: &[Board], nodes: &NodeIndex) -> std::vec::Vec<Board> {
    let mut out = boards.to_vec();
    let index: HashMap<&str, &Board> = boards.iter().map(|b| (b.id.as_str(), b)).collect();
    for board in &mut out {
        if hierarchy_layout_wrapper(nodes.get(board.source_id.as_str()).copied()) {
            board.kind = "ungrouped".into();
            board.label.clear();
            continue;
        }
        if board.kind != "group" {
            continue;
        }
        let mut parent = board.parent_id.clone();
        let mut steps = 0;
        while !parent.is_empty() && steps < boards.len() {
            let Some(p) = index.get(parent.as_str()) else {
                break;
            };
            if !hierarchy_layout_wrapper(nodes.get(p.source_id.as_str()).copied()) {
                break;
            }
            parent = p.parent_id.clone();
            steps += 1;
        }
        if parent.is_empty() {
            board.kind = "platform".into();
        }
    }
    hierarchy_terraces(&mut out, nodes);
    out
}

/// Thickness and elevation belong to presentation copies only.
pub fn hierarchy_terraces(boards: &mut [Board], nodes: &NodeIndex) {
    let mut index: HashMap<String, usize> = HashMap::with_capacity(boards.len());
    for (i, board) in boards.iter_mut().enumerate() {
        index.insert(board.id.clone(), i);
        board.position.y = 0.0;
    }
    // Sequence annotations keep their ancestor backgrounds at the original
    // elevation.
    let mut sequence_branch: HashMap<String, bool> = HashMap::new();
    for board in boards.iter() {
        if nodes
            .get(board.source_id.as_str())
            .is_some_and(|o| o.sequence_role == "container")
        {
            let mut id = board.id.clone();
            let mut steps = 0;
            while !id.is_empty() && steps < boards.len() {
                if sequence_branch.get(&id).copied().unwrap_or(false) {
                    break;
                }
                sequence_branch.insert(id.clone(), true);
                let Some(&i) = index.get(&id) else { break };
                id = boards[i].parent_id.clone();
                steps += 1;
            }
        }
    }
    let mut state = vec![0u8; boards.len()];
    let mut in_sequence = vec![false; boards.len()];
    fn place(
        i: usize,
        boards: &mut [Board],
        index: &HashMap<String, usize>,
        nodes: &NodeIndex,
        sequence_branch: &HashMap<String, bool>,
        state: &mut [u8],
        in_sequence: &mut [bool],
    ) {
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        let base = hierarchy_base_surface_y(&boards[i]);
        let mut parent_y = 0.028;
        if let Some(&parent) = index.get(&boards[i].parent_id) {
            if state[parent] != 1 {
                place(
                    parent,
                    boards,
                    index,
                    nodes,
                    sequence_branch,
                    state,
                    in_sequence,
                );
                parent_y = hierarchy_surface_y(&boards[parent]);
                in_sequence[i] = in_sequence[parent];
            }
        }
        let owner = nodes.get(boards[i].source_id.as_str()).copied();
        if owner.is_some_and(|o| o.sequence_role == "container") {
            in_sequence[i] = true;
        }
        let board = &mut boards[i];
        if board.kind == "ungrouped" {
            // Carry the nearest visible cap through layout-only wrappers.
            board.position.y = go_max(0.0, parent_y - base);
        }
        if board.kind == "group" {
            board.size.y = 0.0;
            if !sequence_branch.get(&board.id).copied().unwrap_or(false) && !in_sequence[i] {
                let mut top = go_max(base, parent_y);
                if let Some(owner) = owner {
                    if owner.sequence_role.is_empty()
                        && owner.opacity > 0.0
                        && owner.stroke_dash == 0.0
                        && native_paint(&owner.fill, "transparent").a > 0
                    {
                        // ARM64 hierarchy_boards.go:220.
                        top = go_fmadd(
                            go_max(0.0, HIERARCHY_TERRACE_CEILING - top),
                            0.030 / 0.038,
                            top,
                        );
                        board.size.y = go_max(0.0, top - parent_y);
                    }
                }
                board.position.y = go_max(0.0, top - base);
            }
        }
        state[i] = 2;
    }
    for i in 0..boards.len() {
        place(
            i,
            boards,
            &index,
            nodes,
            &sequence_branch,
            &mut state,
            &mut in_sequence,
        );
    }
    hierarchy_strong_terraces(boards, nodes, &sequence_branch, &in_sequence);
}

pub fn hierarchy_physical_plate(board: &Board, owner: Option<&Node>, fill_alpha: u8) -> bool {
    let Some(owner) = owner else {
        return false;
    };
    if fill_alpha == 0 || owner.stroke_dash > 0.0 {
        return false;
    }
    board.kind == "platform"
        || board.kind == "terrace"
        || board.kind == "group" && board.position.y > 0.0 && owner.sequence_role.is_empty()
}

pub fn hierarchy_casing_floor(boards: &[Board], nodes: &NodeIndex) -> f64 {
    let mut floor: f64 = 0.0;
    for board in boards {
        let owner = nodes.get(board.source_id.as_str()).copied();
        if (board.kind == "terrace" || board.kind == "group" && board.position.y > 0.0)
            && owner.is_some_and(|o| {
                o.opacity > 0.0
                    && hierarchy_physical_plate(
                        board,
                        Some(o),
                        native_paint(&o.fill, "transparent").a,
                    )
            })
        {
            floor = go_max(floor, hierarchy_surface_y(board) + 0.0005);
        }
    }
    floor
}

/// Header contrast follows the actually visible parent surface.
pub fn hierarchy_header_ink(
    board: &Board,
    boards: &HashMap<String, Board>,
    nodes: &NodeIndex,
    tints: &HashMap<String, String>,
    background: &str,
) -> String {
    let mut chain = vec![board.clone()];
    let mut p = board.parent_id.clone();
    while !p.is_empty() && chain.len() <= boards.len() {
        let Some(parent) = boards.get(&p) else { break };
        chain.push(parent.clone());
        p = parent.parent_id.clone();
    }
    let ground = native_paint(background, "#f5f7fb");
    let a = ground.a as f64 / 255.0;
    // ARM64 hierarchy_boards.go:267: the second product of each channel fuses.
    let (mut r, mut g, mut blue) = (
        go_fmadd(245.0, 1.0 - a, ground.r as f64 * a),
        go_fmadd(247.0, 1.0 - a, ground.g as f64 * a),
        go_fmadd(251.0, 1.0 - a, ground.b as f64 * a),
    );
    for item in chain.iter().rev() {
        let Some(owner) = nodes.get(item.source_id.as_str()).copied() else {
            continue;
        };
        if item.kind == "ungrouped" {
            continue;
        }
        let fill = native_paint(tints.get(&item.id).map_or("", String::as_str), "#edf1f7");
        let mut a = fill.a as f64 / 255.0 * owner.opacity;
        if !hierarchy_physical_plate(item, Some(owner), fill.a) {
            a *= 0.22;
        }
        // ARM64 hierarchy_boards.go:279.
        (r, g, blue) = (
            go_fmadd(r, 1.0 - a, fill.r as f64 * a),
            go_fmadd(g, 1.0 - a, fill.g as f64 * a),
            go_fmadd(blue, 1.0 - a, fill.b as f64 * a),
        );
    }
    readable_surface_ink(
        &format!("#{:02x}{:02x}{:02x}", r as u8, g as u8, blue as u8),
        1.0,
    )
}

pub fn hierarchy_board_tint(owner: Option<&Node>, fallback: &str) -> String {
    // Source containers already have their theme colors resolved.
    match owner {
        Some(owner) if !owner.fill.is_empty() => owner.fill.clone(),
        _ => fallback.into(),
    }
}

impl MeshBuilder {
    /// Source layouts reserve label space in two dimensions. Low relief
    /// preserves those clearances in projection.
    pub fn hierarchy_node(&mut self, node: &Node, tint: &str) {
        let start = self.triangles.len();
        let first_link = self.links.len();
        let relief = hierarchy_node_relief(node);
        let prior_support = self.node_support_drop;
        self.node_support_drop = self
            .hierarchy_supports
            .get(&node.board_id)
            .copied()
            .unwrap_or(0.0)
            / relief;
        self.node(node, tint, Some(relief));
        self.node_support_drop = prior_support;
        let floor = node.position.y - node.size.y / 2.0;
        for t in &mut self.triangles[start..] {
            for v in &mut t.v {
                v.position.y = floor + (v.position.y - floor) * relief;
                v.normal = nunit(nv(v.normal.x, v.normal.y / relief, v.normal.z));
            }
        }
        let before_ink = self.triangles.len();
        if !native_structured_node(node) && !native_markdown_card(node) {
            self.classic_ink_edges(node, start);
        }
        if node.metadata.original.animated {
            let len = self.triangles.len();
            if let Some(last) = self.animated_nodes.last_mut() {
                if last.first == start && last.last == before_ink {
                    last.last = len;
                }
            }
        }
        for link in &mut self.links[first_link..] {
            for p in &mut link.points {
                p.y = floor + (p.y - floor) * relief;
            }
        }
        self.mark_paint_owner(start);
    }

    /// A source container encloses its actual descendants.
    pub fn hierarchy_board(
        &mut self,
        board: &Board,
        owner: Option<&Node>,
        tint: &str,
        opacity: f64,
    ) {
        let Some(owner) = owner else {
            return;
        };
        if board.kind == "ungrouped" || opacity <= 0.0 || board.size.x <= 0.0 || board.size.z <= 0.0
        {
            return;
        }
        let first = self.triangles.len();
        let mut fill = native_paint(tint, "#edf1f7");
        let platform = hierarchy_physical_plate(board, Some(owner), fill.a);
        if !platform && board.kind != "sequence-group" {
            fill.a = (fill.a as f64 * 0.22).round() as u8;
        }
        let mut face_owner = owner.clone();
        face_owner.size = board.size;
        let mut source_fill = rich_color(fill);
        if d2_rs_scenebuild::is_gradient(tint) {
            // Authored gradients are source paint, not a representative swatch.
            source_fill = tint.into();
            if !platform && board.kind != "sequence-group" {
                match native_gradient_opacity(tint, 0.22) {
                    Ok(v) => source_fill = v,
                    Err(error) => {
                        self.fail(error);
                        return;
                    }
                }
            }
        }
        let source = native_face_source(&face_owner, &source_fill);
        let profiles = match native_shape_profiles(&source) {
            Ok(p) => p,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let (tex, face) = self.native_face(&source);
        if self.err.is_some() {
            return;
        }
        let (sx, sz) = (
            board.size.x / source.width as f64,
            board.size.z / source.height as f64,
        );
        let scale = if self.scale <= 0.0 { 0.01 } else { self.scale };
        let y = hierarchy_surface_y(board);
        let mut depth = 0.0;
        if platform {
            depth = 0.13;
            if board.kind == "group" || board.kind == "terrace" {
                depth = board.size.y;
            }
        }
        if owner.metadata.original.three_dee {
            // ARM64 hierarchy_boards.go:345.
            depth = go_fmadd(THREE_DEE_OFFSET as f64, scale, depth);
        }
        let side: MaterialRef = native_material(tint, 0.62, 0.12, opacity);
        {
            let mut s = side.borrow_mut();
            s.color = tint_dark(s.color, 0.85);
        }
        for copy_index in [1i64, 0] {
            if copy_index == 1 && !owner.metadata.original.multiple {
                continue;
            }
            let c = copy_index as f64;
            // ARM64 hierarchy_boards.go:354/359/365.
            let top = go_fmsub(c, 0.00001, y);
            let place_x = |v: f64| go_fmadd(c * MULTIPLE_OFFSET as f64, scale, v);
            let place_z = |v: f64| go_fmadd(-c * MULTIPLE_OFFSET as f64, scale, v);
            if depth > 0.0 {
                for profile in &profiles {
                    let world: std::vec::Vec<Vec> = profile
                        .iter()
                        .map(|p| {
                            nv(
                                place_x(board.position.x - board.size.x / 2.0 + p.x * sx),
                                top,
                                place_z(board.position.z - board.size.z / 2.0 + p.z * sz),
                            )
                        })
                        .collect();
                    self.extruded_profile(&world, top - depth, None, &side);
                }
            }
            let mut paint = face.clone();
            paint.center = nv(
                place_x(board.position.x - board.size.x / 2.0 + face.center.x * sx),
                top,
                place_z(board.position.z - board.size.z / 2.0 + face.center.z * sz),
            );
            paint.width = face.width * sx;
            paint.depth = face.depth * sz;
            let start = self.triangles.len();
            self.surface_texture(tex.as_ref(), &paint, opacity);
            for t in &mut self.triangles[start..] {
                t.no_depth_write = false;
                t.cast_shadow = platform || owner.metadata.original.shadow;
                if let Some(m) = &t.material {
                    let mut m = m.borrow_mut();
                    m.unlit = !platform;
                    if board.kind == "sequence-group" {
                        m.multiply = true;
                    }
                }
                if board.kind == "sequence-group" {
                    t.no_depth_write = true;
                }
            }
        }
        super::geometry::native_physical_shadows(
            &mut self.triangles[first..],
            owner.metadata.original.shadow,
        );
        if owner.metadata.original.animated && self.triangles.len() > first {
            self.animated_nodes.push(NativeAnimatedNode {
                first,
                last: self.triangles.len(),
            });
        }
    }

    pub fn hierarchy_rect(
        &mut self,
        center: Vec,
        width: f64,
        depth: f64,
        y: f64,
        mat: &MaterialRef,
    ) {
        let a = nv(center.x - width / 2.0, y, center.z - depth / 2.0);
        let c = nv(center.x + width / 2.0, y, center.z - depth / 2.0);
        let d = nv(center.x + width / 2.0, y, center.z + depth / 2.0);
        let e = nv(center.x - width / 2.0, y, center.z + depth / 2.0);
        self.flat(a, e, d, mat, false);
        self.flat(a, d, c, mat, false);
    }
}
