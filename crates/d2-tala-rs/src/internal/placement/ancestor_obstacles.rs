// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! node_placement.go72–94: fixed boxes belong to one recursive invocation.
use super::*;

struct ObstacleFrame {
    fixed_children: Vec<Rect>,
    parent: Option<Option<NodeId>>,
    shift: Point,
}

/// The geometry solver runs in postorder, but Go captures each parent's
/// fixed boxes before child recursion. Keep that preorder lifetime distinct
/// from the later, refreshed graph projections used by the solver.
#[derive(Default)]
pub(super) struct AncestorObstacleFrames {
    frames: BTreeMap<Option<NodeId>, ObstacleFrame>,
}

impl AncestorObstacleFrames {
    pub(super) fn enter(&mut self, graph: &ArenaGraph, root: Option<NodeId>) {
        if self.frames.contains_key(&root) {
            return;
        }
        let fixed_children = graph
            .containers
            .get(&root)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| {
                let node = &graph.nodes[id.0 as usize];
                Some(Rect {
                    origin: node.fixed_top_left?,
                    size: node.rect.size,
                })
            })
            .collect();
        let parent = root.map(|id| graph.nodes[id.0 as usize].container);
        let mut shift = Point::default();
        if let Some(root) = root
            && let Some(fixed) = graph.nodes[root.0 as usize].fixed_top_left
            && let Some(parent) = parent.and_then(|key| self.frames.get_mut(&key))
            && parent.fixed_children.len() > 1
        {
            shift = Point {
                x: -fixed.x - CONTAINER_PADDING,
                y: -fixed.y - CONTAINER_PADDING,
            };
            if shift.x != 0.0 || shift.y != 0.0 {
                for obstacle in &mut parent.fixed_children {
                    obstacle.origin.x += shift.x;
                    obstacle.origin.y += shift.y;
                }
            }
        }
        self.frames.insert(
            root,
            ObstacleFrame {
                fixed_children,
                parent,
                shift,
            },
        );
    }

    pub(super) fn incoming(&self, root: Option<NodeId>) -> Option<&[Rect]> {
        let frame = self.frames.get(&root)?;
        match frame.parent {
            None => Some(&[]),
            Some(parent) => self
                .frames
                .get(&parent)
                .map(|frame| frame.fixed_children.as_slice()),
        }
    }

    pub(super) fn exit(&mut self, root: Option<NodeId>) {
        let Some(frame) = self.frames.remove(&root) else {
            return;
        };
        if (frame.shift.x != 0.0 || frame.shift.y != 0.0)
            && let Some(parent) = frame.parent.and_then(|key| self.frames.get_mut(&key))
        {
            // Go's deferred inverse mutates the same points. Restoring a
            // saved value would erase observable floating-point roundoff.
            for obstacle in &mut parent.fixed_children {
                obstacle.origin.x -= frame.shift.x;
                obstacle.origin.y -= frame.shift.y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> ArenaGraph {
        let mut input = Graph::default();
        for name in ["a", "b"] {
            input.add_node(crate::Node::new(
                name,
                Size {
                    width: 100.0,
                    height: 100.0,
                },
            ));
        }
        let mut graph = ArenaGraph::from_input(&input);
        graph.nodes[0].fixed_top_left = Some(Point { x: 0.0, y: 0.0 });
        graph.nodes[1].fixed_top_left = Some(Point { x: 150.0, y: 0.0 });
        graph
    }

    #[test]
    fn later_siblings_keep_the_pre_recursion_fixed_dimensions() {
        let mut graph = graph();
        let mut frames = AncestorObstacleFrames::default();
        frames.enter(&graph, None);
        frames.enter(&graph, Some(NodeId(0)));
        frames.exit(Some(NodeId(0)));
        graph.nodes[0].rect.size = Size {
            width: 250.0,
            height: 430.0,
        };
        frames.enter(&graph, Some(NodeId(1)));
        let obstacles = frames.incoming(Some(NodeId(1))).unwrap();
        assert_eq!(
            obstacles[0].size,
            Size {
                width: 100.0,
                height: 100.0
            }
        );
        assert_eq!(
            obstacles[0].origin,
            Point {
                x: -210.0,
                y: -60.0
            }
        );
        frames.exit(Some(NodeId(1)));
        assert_eq!(
            frames.frames[&None].fixed_children[0].origin,
            Point::default()
        );
    }

    #[test]
    fn inverse_translation_preserves_roundoff_in_shared_parent_boxes() {
        let mut graph = graph();
        graph.nodes[0].fixed_top_left = Some(Point { x: 0.1, y: 0.1 });
        graph.nodes[1].fixed_top_left = Some(Point { x: 0.3, y: 0.3 });
        let mut frames = AncestorObstacleFrames::default();
        frames.enter(&graph, None);
        frames.enter(&graph, Some(NodeId(0)));
        frames.exit(Some(NodeId(0)));
        let retained = frames.frames[&None].fixed_children[1].origin;
        assert_ne!(retained.x.to_bits(), 0.3_f64.to_bits());
        frames.enter(&graph, Some(NodeId(1)));
        frames.exit(Some(NodeId(1)));
        assert_eq!(frames.frames[&None].fixed_children[1].origin, retained);
    }
}
