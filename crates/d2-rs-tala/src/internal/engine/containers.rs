// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Container inference and hierarchy bookkeeping.
//!
//! Full layout consumes declared parent relationships; standalone routing can
//! instead infer containment from positioned, shape-eligible boxes.

use super::*;
use d2_rs_core::stable_sort::StableSort;

impl ArenaGraph {
    /// Recovered prearranged branch of `Graph.AddContainers`.
    ///
    /// Standalone edge routing receives positioned boxes rather than trusting
    /// the serialized hierarchy. TALA sorts those boxes from smallest to
    /// largest, assigns each node to the first covering larger box whose shape
    /// can contain children, and accepts an eight-unit near-containment by
    /// clipping the child back inside the candidate.
    pub(super) fn infer_prearranged_containers(&mut self) {
        self.containers.clear();
        self.containers.insert(None, Vec::new());
        for node in &mut self.nodes {
            node.container = None;
            node.is_container = false;
            node.scoring_container_parent = None;
            node.scoring_container_ancestors.clear();
        }
        if self.nodes.is_empty() {
            return;
        }

        let mut nodes_by_area = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.input_id,
                    node.rect.size.width * node.rect.size.height,
                    index,
                )
            })
            .collect::<Vec<_>>();
        nodes_by_area.stable_sort_by(|left, right| {
            let area_order = if (left.1 - right.1).abs() < 0.0001 {
                std::cmp::Ordering::Equal
            } else {
                left.1.total_cmp(&right.1)
            };
            // The recovered equal-area comparator returns `i > j`, which
            // reverses the original Graph.Nodes order for an equal run.
            area_order.then_with(|| right.2.cmp(&left.2))
        });

        let largest = nodes_by_area.last().expect("nonempty node list").0;
        self.containers.append_node(None, largest);

        for index in 0..nodes_by_area.len() - 1 {
            let node = nodes_by_area[index].0;
            let mut selected_container = None;
            for &(candidate, _, _) in &nodes_by_area[index + 1..] {
                if !self.nodes[candidate.0 as usize].shape.can_contain() {
                    continue;
                }
                let Some(node_position) = self.position(node) else {
                    continue;
                };
                let Some(candidate_position) = self.position(candidate) else {
                    continue;
                };
                let node_size = self.nodes[node.0 as usize].rect.size;
                let candidate_size = self.nodes[candidate.0 as usize].rect.size;
                let covers = node_position.x >= candidate_position.x
                    && node_position.y >= candidate_position.y
                    && node_position.x + node_size.width
                        <= candidate_position.x + candidate_size.width
                    && node_position.y + node_size.height
                        <= candidate_position.y + candidate_size.height;
                if covers {
                    selected_container = Some(candidate);
                    break;
                }
                let nearly_covers = node_position.x >= candidate_position.x - 8.0
                    && node_position.y >= candidate_position.y - 8.0
                    && node_position.x + node_size.width
                        <= candidate_position.x + candidate_size.width + 8.0
                    && node_position.y + node_size.height
                        <= candidate_position.y + candidate_size.height + 8.0;
                if !nearly_covers {
                    continue;
                }

                let clipped_position = Point {
                    x: node_position.x.max(candidate_position.x),
                    y: node_position.y.max(candidate_position.y),
                };
                let clipped_size = Size {
                    width: node_size
                        .width
                        .min(candidate_position.x + candidate_size.width - clipped_position.x),
                    height: node_size
                        .height
                        .min(candidate_position.y + candidate_size.height - clipped_position.y),
                };
                self.nodes[node.0 as usize].position = Some(clipped_position);
                self.nodes[node.0 as usize].fixed_top_left = Some(clipped_position);
                self.nodes[node.0 as usize].rect.size = clipped_size;
                selected_container = Some(candidate);
                break;
            }
            self.nodes[node.0 as usize].container = selected_container;
            self.containers.append_node(selected_container, node);
        }

        for container in self.containers.keys() {
            self.containers
                .sort_by_key(&container, |child| self.nodes[child.0 as usize].tala_id);
            if let Some(container) = container {
                self.nodes[container.0 as usize].is_container = true;
            }
        }
        for index in 0..self.nodes.len() {
            let mut current = self.nodes[index].container;
            self.nodes[index].scoring_container_parent =
                current.map(|container| self.nodes[container.0 as usize].tala_id);
            while let Some(container) = current {
                let container_tala_id = self.nodes[container.0 as usize].tala_id;
                let parent = self.nodes[container.0 as usize].container;
                self.nodes[index]
                    .scoring_container_ancestors
                    .push(container_tala_id);
                current = parent;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("TALA_TRACE_PREARRANGED_CONTAINERS") {
            let nodes = self
                .nodes
                .iter()
                .map(|node| {
                    serde_json::json!({
                        "id": node.tala_id,
                        "container": node.container.map(|container| self.nodes[container.0 as usize].tala_id),
                        "x": node.position.map(|position| position.x),
                        "y": node.position.map(|position| position.y),
                        "width": node.rect.size.width,
                        "height": node.rect.size.height,
                    })
                })
                .collect::<Vec<_>>();
            eprintln!(
                "{}",
                serde_json::json!({"event":"prearranged-containers","nodes":nodes})
            );
        }
    }

    pub(super) fn apply_canvas_positions(&mut self) {
        let positioned: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.container.is_none() && node.canvas_position.is_some())
            .map(|node| node.input_id)
            .collect();
        for node in positioned {
            let flow_roots: Vec<_> = self
                .nodes
                .iter()
                .filter(|candidate| {
                    candidate.input_id != node
                        && candidate.container.is_none()
                        && candidate.canvas_position.is_none()
                        && candidate.position.is_some()
                })
                .map(|candidate| candidate.input_id)
                .collect();
            if flow_roots.is_empty() {
                continue;
            }
            let minimum_x = flow_roots
                .iter()
                .filter_map(|root| self.position(*root).map(|position| position.x))
                .fold(f64::INFINITY, f64::min);
            let minimum_y = flow_roots
                .iter()
                .filter_map(|root| self.position(*root).map(|position| position.y))
                .fold(f64::INFINITY, f64::min);
            if self.nodes[node.0 as usize].canvas_position != Some(CanvasPosition::TopCenter) {
                self.place_non_top_center_canvas_node(node, &flow_roots);
                continue;
            }
            if self.directions[&None] == Direction::Right {
                let Some(bottom_root) = flow_roots.iter().copied().max_by(|left, right| {
                    let bottom = |id: NodeId| {
                        let position = self.position(id).unwrap();
                        position.y + self.nodes[id.0 as usize].rect.size.height
                    };
                    bottom(*left)
                        .total_cmp(&bottom(*right))
                        .then_with(|| left.cmp(right))
                }) else {
                    continue;
                };
                let bottom_position = self.position(bottom_root).unwrap();
                let bottom_size = self.nodes[bottom_root.0 as usize].rect.size;
                self.move_node_abs_with_children(
                    node,
                    Point {
                        x: bottom_position.x + bottom_size.width + crate::NODE_GAP,
                        y: bottom_position.y + bottom_size.height,
                    },
                );
                continue;
            }
            let height = self.nodes[node.0 as usize].rect.size.height;
            let delta = Point {
                x: 0.0,
                y: height + crate::NODE_GAP,
            };
            for root in flow_roots {
                self.translate_node_with_children(root, delta);
            }
            self.move_node_abs_with_children(
                node,
                Point {
                    x: minimum_x,
                    y: minimum_y,
                },
            );
        }
    }

    fn place_non_top_center_canvas_node(&mut self, node: NodeId, flow_roots: &[NodeId]) {
        let Some(position) = self.nodes[node.0 as usize].canvas_position else {
            return;
        };
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for root in flow_roots {
            let Some(origin) = self.position(*root) else {
                continue;
            };
            let size = self.nodes[root.0 as usize].rect.size;
            min_x = min_x.min(origin.x);
            min_y = min_y.min(origin.y);
            max_x = max_x.max(origin.x + size.width);
            max_y = max_y.max(origin.y + size.height);
        }
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            return;
        }
        let size = self.nodes[node.0 as usize].rect.size;
        let gap = crate::NODE_GAP;
        let center_x = (min_x + max_x) / 2.0;
        let center_y = (min_y + max_y) / 2.0;
        let origin = match position {
            CanvasPosition::TopLeft => Point {
                x: min_x - size.width - gap,
                y: min_y - size.height - gap,
            },
            CanvasPosition::TopCenter => Point {
                x: center_x - size.width / 2.0,
                y: min_y - size.height - gap,
            },
            CanvasPosition::TopRight => Point {
                x: max_x + gap,
                y: min_y - size.height - gap,
            },
            CanvasPosition::CenterLeft => Point {
                x: min_x - size.width - gap,
                y: center_y - size.height / 2.0,
            },
            CanvasPosition::CenterRight => Point {
                x: max_x + gap,
                y: center_y - size.height / 2.0,
            },
            CanvasPosition::BottomLeft => Point {
                x: min_x - size.width - gap,
                y: max_y + gap,
            },
            CanvasPosition::BottomCenter => Point {
                x: center_x - size.width / 2.0,
                y: max_y + gap,
            },
            CanvasPosition::BottomRight => Point {
                x: max_x + gap,
                y: max_y + gap,
            },
        };
        self.move_node_abs_with_children(node, origin);
    }
}
