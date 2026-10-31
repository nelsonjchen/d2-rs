// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Coordinate padding, scale restoration, and final-frame normalization.
//!
//! Nodes, route points, descendants, labels, and icons translate together so
//! temporary negative or padded working coordinates do not leak to output.

use super::*;

impl ArenaGraph {
    // Recovered Graph.pad used by Pipeline.Rescale.
    pub(super) fn pad(&mut self) {
        for node in self.node_order.clone() {
            if self.position(node).is_some() {
                self.translate_owner_point(node, 1000.0, 1000.0);
            }
        }
    }

    /// Direct translation of recovered `Graph.normalize` for represented
    /// node and route geometry. TALA also includes edge-label top-lefts in the
    /// minimum once label metadata is present.
    pub(super) fn normalize(&mut self) {
        self.normalize_inner(false)
            .expect("normalization source invariant");
    }
    pub(super) fn normalize_seed(&mut self) -> Result<(), crate::LayoutError> {
        self.normalize_inner(true)
    }
    fn normalize_inner(&mut self, recover: bool) -> Result<(), crate::LayoutError> {
        fn required<T>(value: Option<T>, recover: bool) -> Result<T, crate::LayoutError> {
            match value {
                Some(v) => Ok(v),
                None if recover => Err(crate::LayoutError::SeedInvariant),
                None => panic!("runtime error: invalid memory address or nil pointer dereference"),
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_NORMALIZE") {
            eprint!("GRID_NORMALIZE_RUST phase=before");
            for node in &self.nodes {
                if matches!(node.tala_id, 233_611_931 | 1_149_337_423 | 1_782_109_120) {
                    eprint!(" {}={:?}", node.tala_id, node.position);
                }
            }
            eprintln!();
        }
        // Go's normalize scans the live Graph.Nodes slice.  Stable Rust keeps
        // absorbed sequence/cluster members for final serialization, but they
        // are not part of that active slice and must not choose the origin.
        let live_nodes = self.node_order.iter().copied().collect::<BTreeSet<_>>();
        let active_node = |node: &ArenaNode| live_nodes.contains(&node.input_id);
        let has_fixed_node = self
            .nodes
            .iter()
            .filter(|node| active_node(node))
            .any(|node| node.fixed_top_left.is_some());
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_NORMALIZE") {
            eprintln!(
                "NORMALIZE_RUST_FIXED has_fixed={} {:?}",
                has_fixed_node,
                self.nodes
                    .iter()
                    .filter(|node| active_node(node) && node.fixed_top_left.is_some())
                    .map(|node| (node.tala_id, node.fixed_top_left))
                    .collect::<Vec<_>>()
            );
        }
        let (mut minimum_x, mut minimum_y) = if has_fixed_node {
            (1000.0, 1000.0)
        } else {
            (f64::INFINITY, f64::INFINITY)
        };

        if !has_fixed_node {
            for node in self.nodes.iter().filter(|node| active_node(node)) {
                #[cfg(feature = "diagnostic-traces")]
                if crate::internal::trace_env_enabled("WEFTAN_TRACE_NORMALIZE") {
                    eprintln!(
                        "NORMALIZE_RUST_NODE tala={} pos={:?} owner={} sequence={:?} cluster={:?}",
                        node.tala_id,
                        node.position,
                        self.nodes[self.active_aggregate_owner(node.input_id).0 as usize].tala_id,
                        node.sequence,
                        node.cluster,
                    );
                }
                let position = required(self.position(node.input_id), recover)?;
                minimum_x = minimum_x.min(position.x);
                minimum_y = minimum_y.min(position.y);
            }
            // Graph.Edges membership is separate from retained edge allocations.
            for edge_id in &self.edge_order.read() {
                let edge = &self.edge_records[edge_id.0 as usize];
                for point in &edge.route.read() {
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_NORMALIZE") {
                        eprintln!("NORMALIZE_RUST_POINT {},{}", point.x, point.y);
                    }
                    minimum_x = minimum_x.min(point.x.floor());
                    minimum_y = minimum_y.min(point.y.floor());
                }
                if let Some(label) = &edge.label {
                    let top_left = required(
                        labels::edge_label_top_left(
                            edge,
                            label.position,
                            label.percentage,
                            label.size.width,
                            label.size.height,
                        ),
                        recover,
                    )?;
                    #[cfg(feature = "diagnostic-traces")]
                    if crate::internal::trace_env_enabled("WEFTAN_TRACE_NORMALIZE") {
                        eprintln!("NORMALIZE_RUST_LABEL {},{}", top_left.x, top_left.y);
                    }
                    minimum_x = minimum_x.min(top_left.x.floor());
                    minimum_y = minimum_y.min(top_left.y.floor());
                }
            }
        }

        // The recovered implementation is only called on a laid-out graph.
        // Keep the empty arena finite and unchanged for the public probe API.
        if !minimum_x.is_finite() || !minimum_y.is_finite() {
            return Ok(());
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_NORMALIZE") {
            eprintln!("NORMALIZE_RUST min={},{}", minimum_x, minimum_y);
        }
        for node in self.node_order.clone() {
            required(self.position(node), recover)?;
            self.translate_owner_point(node, -minimum_x, -minimum_y);
        }
        for edge_id in self.edge_order.read() {
            let edge = &mut self.edge_records[edge_id.0 as usize];
            for mut point in edge.route.iter_mut() {
                point.x -= minimum_x;
                point.y -= minimum_y;
            }
        }
        #[cfg(feature = "diagnostic-traces")]
        if crate::internal::trace_env_enabled("WEFTAN_TRACE_GRID_NORMALIZE") {
            eprint!("GRID_NORMALIZE_RUST phase=after");
            for node in &self.nodes {
                if matches!(node.tala_id, 233_611_931 | 1_149_337_423 | 1_782_109_120) {
                    eprint!(" {}={:?}", node.tala_id, node.position);
                }
            }
            eprintln!();
        }
        Ok(())
    }
}

#[cfg(test)]
mod live_membership_tests {
    use super::*;

    #[test]
    fn normalization_uses_live_membership_with_retained_edge_allocations() {
        let mut input = Graph::default();
        let first = input.add_node(crate::Node::new(
            "first",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let second = input.add_node(crate::Node::new(
            "second",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        for _ in 0..3 {
            input.add_edge(crate::Edge {
                source: first,
                target: second,
            });
        }
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(
            first,
            Point {
                x: 1000.0,
                y: 1000.0,
            },
        );
        arena.set_position(
            second,
            Point {
                x: 1100.0,
                y: 1100.0,
            },
        );
        // Source Disconnect retains canonical allocations and public IDs.
        arena.disconnect_edge(EdgeId(0));
        arena.disconnect_edge(EdgeId(2));
        arena.edge_records[1]
            .route
            .replace(RoutePoints::literal(&(vec![Point { x: 900.5, y: 950.5 }])));
        arena.edge_records[2].route.replace(RoutePoints::literal(
            &(vec![Point {
                x: -500.0,
                y: -600.0,
            }]),
        ));
        assert_eq!(arena.edge_records[1].input_id, EdgeId(1));
        arena.normalize();
        assert_eq!(arena.position(first), Some(Point { x: 100.0, y: 50.0 }));
        assert_eq!(
            arena.edge_records[1].route.read(),
            vec![Point { x: 0.5, y: 0.5 }]
        );
        assert_eq!(
            arena.edge_records[2].route.read(),
            vec![Point {
                x: -500.0,
                y: -600.0
            }]
        );
    }

    #[test]
    fn padding_and_normalization_translate_retained_live_points_in_place() {
        let mut input = Graph::default();
        let live = input.add_node(crate::Node::new(
            "live",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let retired = input.add_node(crate::Node::new(
            "retired",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(live, Point { x: 10.0, y: 20.0 });
        arena.set_position(
            retired,
            Point {
                x: -100.0,
                y: -200.0,
            },
        );
        arena.initialize_shared_topology();
        arena.materialize_owner_node_point(live).unwrap();
        let retained = arena.owner_node_point(live).unwrap();
        arena.node_order = vec![live];
        arena.pad();
        assert_eq!(
            retained.get(),
            Some(Point {
                x: 1010.0,
                y: 1020.0
            })
        );
        arena.normalize();
        assert_eq!(retained.get(), Some(Point { x: 0.0, y: 0.0 }));
        assert!(retained.same_allocation(&arena.owner_node_point(live).unwrap()));
        assert_eq!(
            arena.position(retired),
            Some(Point {
                x: -100.0,
                y: -200.0
            })
        );
    }

    #[test]
    fn normalization_keeps_removed_entity_geometry_out_of_the_live_frame() {
        let mut input = Graph::default();
        let live = input.add_node(crate::Node::new(
            "live",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let retired = input.add_node(crate::Node::new(
            "retired",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge {
            source: live,
            target: retired,
        });
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(live, Point { x: 10.0, y: 20.0 });
        graph.set_position(
            retired,
            Point {
                x: -100.0,
                y: -200.0,
            },
        );
        graph.edge_records[edge.0 as usize]
            .route
            .replace(RoutePoints::literal(
                &(vec![Point {
                    x: -500.0,
                    y: -600.0,
                }]),
            ));
        graph.node_order.retain(|node| *node != retired);
        graph.edge_order.clear();
        graph.normalize();
        assert_eq!(graph.position(live), Some(Point { x: 0.0, y: 0.0 }));
        assert_eq!(
            graph.position(retired),
            Some(Point {
                x: -100.0,
                y: -200.0
            })
        );
        assert_eq!(
            graph.edge_records[edge.0 as usize].route.read(),
            vec![Point {
                x: -500.0,
                y: -600.0
            }]
        );
    }
    #[test]
    fn unset_label_normalization_fails_before_translation_unless_fixed() {
        let mut input = Graph::default();
        let source = input.add_node(crate::Node::new(
            "source",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let target = input.add_node(crate::Node::new(
            "target",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ));
        let edge = input.add_edge(crate::Edge { source, target });
        input.set_edge_label(
            edge,
            Some(crate::EdgeLabel {
                text: "unset".into(),
                size: Size {
                    width: 0.0,
                    height: 21.0,
                },
                position: crate::LabelPosition::Unset,
                percentage: 0.0,
            }),
        );
        let mut graph = ArenaGraph::from_input(&input);
        graph.set_position(
            source,
            Point {
                x: 1010.0,
                y: 1020.0,
            },
        );
        graph.set_position(
            target,
            Point {
                x: 1200.0,
                y: 1020.0,
            },
        );
        assert!(matches!(
            graph.normalize_seed(),
            Err(crate::LayoutError::SeedInvariant)
        ));
        assert_eq!(
            graph.position(source),
            Some(Point {
                x: 1010.0,
                y: 1020.0
            })
        );
        graph.nodes[source.0 as usize].fixed_top_left = Some(Point { x: 10.0, y: 20.0 });
        graph.normalize_seed().unwrap();
        assert_eq!(graph.position(source), Some(Point { x: 10.0, y: 20.0 }));
        assert!(matches!(
            labels::score_existing_label_placements_checked(&graph),
            Err(crate::LayoutError::SeedInvariant)
        ));
    }
}
