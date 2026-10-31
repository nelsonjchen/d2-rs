// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg hierarchy_support.go: strong terraces and the supports
//! that extend bodies down to lowered plates. Contraction sites are marked
//! `ARM64:` (hierarchyTierHeight is inlined, so its product fuses).

use std::collections::HashMap;

use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use d2_rs_scene::{go_max, go_min};

use super::geometry::native_paint;
use super::hierarchy_boards::{hierarchy_base_surface_y, hierarchy_node_relief, hierarchy_surface_y, NodeIndex};
use super::native_shapes::native_canonical_height;
use super::native_solids::{native_solid_height, native_solid_node};
use crate::scene::{Board, Node};

const HIERARCHY_MAX_DESCENT: f64 = 0.60;

fn tier_factor(depth: i64) -> f64 {
    1.0 - gomath::pow(2.0 / 3.0, 0.max(depth) as f64)
}

/// Spend the most visible relief on the first source tier.
pub fn hierarchy_tier_height(depth: i64) -> f64 {
    HIERARCHY_MAX_DESCENT * tier_factor(depth)
}

/// Each independent visible root gets its own support plan.
pub fn hierarchy_strong_terraces(boards: &mut [Board], nodes: &NodeIndex, sequence_branch: &HashMap<String, bool>, in_sequence: &[bool]) {
    let n = boards.len();
    let index: HashMap<String, usize> = boards.iter().enumerate().map(|(i, b)| (b.id.clone(), i)).collect();
    let (mut depth, mut root) = (vec![0i64; n], vec![0usize; n]);
    let (mut physical, mut blocked) = (vec![false; n], vec![false; n]);
    let mut state = vec![0u8; n];
    #[allow(clippy::too_many_arguments)]
    fn visit(
        i: usize,
        boards: &[Board],
        index: &HashMap<String, usize>,
        nodes: &NodeIndex,
        state: &mut [u8],
        depth: &mut [i64],
        root: &mut [usize],
        physical: &mut [bool],
    ) {
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        root[i] = i;
        let board = &boards[i];
        if let Some(&p) = index.get(&board.parent_id) {
            if state[p] != 1 && board.kind != "platform" {
                visit(p, boards, index, nodes, state, depth, root, physical);
                depth[i] = depth[p];
                root[i] = root[p];
            }
        }
        let owner = nodes.get(board.source_id.as_str());
        physical[i] = board.kind == "group"
            && owner.is_some_and(|o| {
                o.sequence_role.is_empty() && o.opacity > 0.0 && o.stroke_dash == 0.0 && native_paint(&o.fill, "transparent").a > 0
            });
        if physical[i] {
            depth[i] += 1;
        }
        state[i] = 2;
    }
    for i in 0..n {
        visit(i, boards, &index, nodes, &mut state, &mut depth, &mut root, &mut physical);
        if sequence_branch.get(&boards[i].id).copied().unwrap_or(false) || in_sequence[i] {
            blocked[root[i]] = true;
        }
    }
    let mut deepest = vec![0i64; n];
    for i in 0..n {
        deepest[root[i]] = deepest[root[i]].max(depth[i]);
    }
    for i in 0..n {
        let r = root[i];
        if blocked[r] || deepest[r] == 0 {
            continue;
        }
        // ARM64 hierarchy_support.go:61: FMSUBD of the second tier product.
        boards[i].position.y = go_fmsub(HIERARCHY_MAX_DESCENT, tier_factor(deepest[r]), hierarchy_tier_height(depth[i]));
        if depth[i] == deepest[r] {
            boards[i].position.y = 0.0;
        }
        if physical[i] {
            // A presentation-only kind keeps source paint physical.
            boards[i].kind = "terrace".into();
        }
    }
    for i in 0..n {
        if boards[i].kind != "terrace" {
            continue;
        }
        // ARM64 hierarchy_support.go:75: FMADDD then FMSUBD.
        let mut bottom = go_fmsub(
            HIERARCHY_MAX_DESCENT,
            tier_factor(deepest[root[i]]),
            go_fmadd(HIERARCHY_MAX_DESCENT, tier_factor(depth[i] - 1), hierarchy_base_surface_y(&boards[i])),
        );
        if let Some(&p) = index.get(&boards[i].parent_id) {
            bottom = hierarchy_surface_y(&boards[p]);
        }
        boards[i].size.y = go_max(0.0, hierarchy_surface_y(&boards[i]) - bottom);
    }
}

pub fn hierarchy_support_offsets(boards: &[Board]) -> HashMap<String, f64> {
    boards.iter().filter(|b| b.position.y < 0.0).map(|b| (b.id.clone(), b.position.y)).collect()
}

/// Solid contact queries use the same cap-anchored extension as
/// construction.
#[derive(Clone, Copy, Debug)]
pub struct HierarchyExtension {
    pub top: f64,
    pub scale: f64,
}

pub fn hierarchy_node_extension(n: &Node, drop: f64) -> HierarchyExtension {
    let floor = n.position.y - n.size.y / 2.0;
    let mut height = native_canonical_height(n, 0.0);
    if native_solid_node(n) {
        height = native_solid_height(n);
    }
    let relief = hierarchy_node_relief(n);
    let relieved = height * relief;
    // ARM64 hierarchy_support.go:107: floor+height fuses height*relief.
    HierarchyExtension { top: go_fmadd(height, relief, floor), scale: 1.0 - go_min(0.0, drop) / go_max(0.001, relieved) }
}

impl HierarchyExtension {
    pub fn inverse_y(&self, y: f64) -> f64 {
        if y >= self.top || self.scale <= 1.0 {
            return y;
        }
        self.top + (y - self.top) / self.scale
    }

    pub fn point_y(&self, y: f64) -> f64 {
        if y >= self.top || self.scale <= 1.0 {
            return y;
        }
        // ARM64 hierarchy_support.go:121.
        go_fmadd(y - self.top, self.scale, self.top)
    }
}
