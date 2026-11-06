// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Direct-engine graphjson representation and graph reconstruction.
mod decode;
mod preflight;
mod schema;
mod wire;

mod reconstruct;
mod serialize;

/// Lay out one seed and encode the completed live graph in the original
/// graphjson representation. This compatibility boundary does not run D2's
/// post-selection compound-candidate adapter.
pub fn graphjson_layout(input: &[u8], seed: i64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    graphjson_preflight(input)?;
    let serialized = decode::graph(input)?;
    let (graph, nodes, edges) = reconstruct::to_core_graph_with_records(&serialized)?;
    let snapshot =
        crate::internal::layout_snapshot(&graph, seed, crate::internal::LayoutStage::Normalize);
    if let Some(error) = snapshot.layout_error() {
        return Err(error.clone().into());
    }
    let arena = snapshot
        .graph
        .as_ref()
        .ok_or("layout did not retain its graph")?;
    let result = serialize::graph(arena, &nodes, &edges, &serialized)?;
    Ok(serde_json::to_vec(&result)?)
}

/// Parse ordered, case-insensitive Go field updates into the wire representation.
/// This is not a graph codec: topology validation, ownership restoration and
/// complete syntax diagnostics remain pending.
pub fn graphjson_wire_roundtrip(input: &[u8]) -> Result<Vec<u8>, serde_json::Error> {
    let graph = decode::graph(input)?;
    serde_json::to_vec(&graph)
}

/// Validate a serialized graph before reconstruction or layout.
///
/// Validation happens against a private decoded value. Callers receive no
/// partially published graph when decoding or validation fails.
pub fn graphjson_preflight(input: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use crate::internal::limits::{Context, ContextError};
    struct Background;
    impl Context for Background {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }
    let graph = decode::graph(input)?;
    if input.trim_ascii() == b"null" {
        return Err("cannot deserialize a nil graph".into());
    }
    Ok(preflight::validate(Some(&Background), &graph)?)
}

/// Reconstruct the measured graph represented by a v0.9.0 graphjson record.
///
/// The decoder, preflight validator, and reconstruction all complete before
/// the resulting [`d2_rs_core::Graph`] is returned. This gives adapters an
/// atomic publication boundary: malformed or inconsistent input cannot leave
/// a partially reconstructed graph behind. Serialized aggregate metadata is
/// retained on the graph for the placement stages that consume it; aggregate
/// vessel records are not inserted as ordinary routing obstacles.
pub fn graphjson_to_graph(input: &[u8]) -> Result<d2_rs_core::Graph, Box<dyn std::error::Error>> {
    use crate::internal::limits::{Context, ContextError};
    struct Background;
    impl Context for Background {
        fn done(&self) -> Option<bool> {
            None
        }
        fn error(&self) -> Option<ContextError> {
            None
        }
    }
    let graph = decode::graph(input)?;
    if input.trim_ascii() == b"null" {
        return Err("cannot deserialize a nil graph".into());
    }
    preflight::validate(Some(&Background), &graph)?;
    reconstruct::to_core_graph(&graph).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::wire::SerializedGraph;
    #[test]
    fn diagnostic_paths_keep_input_spelling_and_number_tokens() {
        for (input, expected) in [
            (
                r#"{"NODES":[{"ID":1e0}]}"#,
                "json: cannot unmarshal number 1e0 into Go struct field SerializedGraph.NODES.0.ID of type int64",
            ),
            (
                r#"{"nodes":[{"fontSize":1.5}]}"#,
                "json: cannot unmarshal number 1.5 into Go struct field SerializedGraph.nodes.0.fontSize of type int",
            ),
            (
                r#"{"nodes":[true]}"#,
                "json: cannot unmarshal bool into SerializedGraph.nodes.0 of type graphjson.SerializedNode",
            ),
        ] {
            let err = super::decode::graph(input.as_bytes()).err().unwrap();
            assert_eq!(err.to_string(), expected);
        }
    }
    #[test]
    fn repeated_array_fields_reuse_elements_until_reset() {
        let g = super::decode::graph(br#"{"nodes":[{"id":1,"width":10},{"id":2,"width":20}],"NODES":[{"id":3}],"nodes":[{},{}]}"#).unwrap();
        let nodes = g.nodes.unwrap();
        assert_eq!((nodes[0].id, nodes[0].width), (3, 10.0));
        assert_eq!((nodes[1].id, nodes[1].width), (2, 20.0));
        for reset in ["[]", "null"] {
            let input =
                format!(r#"{{"nodes":[{{"id":1,"width":10}}],"nodes":{reset},"nodes":[{{}}]}}"#);
            let g = super::decode::graph(input.as_bytes()).unwrap();
            assert_eq!(g.nodes.unwrap()[0].width, 0.0);
        }
    }
    #[test]
    fn overwritten_invalid_value_is_still_rejected() {
        assert!(super::decode::graph(br#"{"CellSize":"bad","CellSize":1}"#).is_err());
    }

    #[test]
    fn reconstructs_signed_ids_routes_and_ordered_containers() {
        use d2_rs_core::{Direction, EdgeId, NodeId};

        let input = br#"{
            "nodes":[
                {"id":-7,"d2id":"left","width":80,"height":40,"topLeft":{"x":3,"y":4},"nears":[11],"shapeType":"Diamond"},
                {"id":11,"d2id":"right","width":90,"height":50,"fixedTopLeft":{"x":120,"y":4},"shapeType":"Circle"}
            ],
            "edges":[{"id":9,"from":-7,"to":11,"points":[{"x":83,"y":24},{"x":120,"y":24}],"labelPercentage":0.25,"label":{"Text":"go","Position":"OUTSIDE_TOP_CENTER","Width":12,"Height":8},"sourceArrowhead":"","targetArrowhead":"triangle","minWidth":7,"minHeight":8}],
            "containers":{"0":[-7],"-7":[11]},
            "directions":{"0":"RIGHT"}
        }"#;
        let graph = super::graphjson_to_graph(input).unwrap();
        assert_eq!(
            graph
                .nodes()
                .map(|(id, _)| graph.tala_id_overrides[id.0 as usize])
                .collect::<Vec<_>>(),
            vec![Some(-7), Some(11)]
        );
        assert_eq!(graph.node(NodeId(0)).unwrap().parent, None);
        assert_eq!(graph.node(NodeId(1)).unwrap().parent, Some(NodeId(0)));
        let edge = graph.edge_label(EdgeId(0)).unwrap();
        assert_eq!(edge.percentage, 0.25);
        assert_eq!(graph.edge_route(EdgeId(0)).len(), 2);
        assert_eq!(graph.tala_edge_ids[0], Some(9));
        assert_eq!(graph.direction, Direction::Right);
        // Raw graphjson stores an effective identity, including its distinct
        // empty no-arrow value, rather than an absent D2 arrowhead record.
        assert_eq!(graph.edge_arrowheads(EdgeId(0)).source.as_deref(), Some(""));
        assert_eq!(
            graph.edge_arrowheads(EdgeId(0)).target.as_deref(),
            Some("triangle")
        );
    }

    #[test]
    fn interns_serialized_sequence_vessel_separately_from_membership() {
        use d2_rs_core::NodeId;
        let input = br#"{
            "nodes":[
                {"id":1,"d2id":"first","width":80,"height":40},
                {"id":2,"d2id":"second","width":80,"height":40}
            ],
            "edges":[{"id":7,"from":1,"to":2}],
            "sequences":{"99":{"Nodes":[1,2],"EdgeAbductions":[{"Edge":7,"OriginallyFromNode":1,"OriginallyToNode":2,"CurrentFromNode":99,"CurrentToNode":2}],"Container":0}},
            "sequenceVessels":{"99":{"id":99,"width":180,"height":100,"topLeft":{"x":12,"y":24}}}
        }"#;
        let graph = super::graphjson_to_graph(input).unwrap();
        assert_eq!(graph.nodes().count(), 3);
        assert_eq!(
            graph.tala_decoded_members.as_ref().unwrap().0,
            vec![NodeId(0), NodeId(1)]
        );
        assert_eq!(
            graph.tala_decoded_nil_owners,
            [NodeId(2)].into_iter().collect()
        );
        assert_eq!(graph.serialized_sequences.len(), 1);
        let sequence = &graph.serialized_sequences[0];
        assert_eq!(sequence.vessel_tala_id, 99);
        assert_eq!(sequence.member_tala_ids, vec![1, 2]);
        assert_eq!(sequence.edge_abductions.len(), 1);
        assert_eq!(sequence.edge_abductions[0].edge_id, 7);
        assert_eq!(
            sequence.vessel_position,
            Some(d2_rs_core::Point { x: 12.0, y: 24.0 })
        );
        assert_eq!(
            sequence.vessel_size,
            d2_rs_core::Size {
                width: 180.0,
                height: 100.0
            }
        );
        assert_eq!(graph.tala_id_overrides, vec![Some(1), Some(2), Some(99)]);
    }

    #[test]
    fn reconstructs_auxiliary_hubs_trees_hierarchies_and_costs_atomically() {
        let input = br#"{
            "nodes":[
                {"id":1,"d2id":"hub","width":80,"height":40},
                {"id":2,"d2id":"spoke","width":80,"height":40}
            ],
            "edges":[],
            "hubs":{"1":[2]},
            "trees":{"0":[{"Node":{"id":3,"width":20,"height":20},"SentinelEdge":{"id":-2,"originalId":0,"from":3,"to":3},"Orientation":4}]},
            "hierarchies":[{"Levels":{"1":0,"2":1}}],
            "directions":{"0":"LEFT","1":"UP"},
            "CellSize":37.5,
            "crossingCost":1.25,
            "turnCost":2.5,
            "nonCenterPortCost":3.75
        }"#;
        let graph = super::graphjson_to_graph(input).unwrap();
        assert_eq!(
            graph.serialized_hubs,
            vec![d2_rs_core::SerializedHubState {
                hub_tala_id: 1,
                spoke_tala_ids: vec![2]
            }]
        );
        let tree = &graph.serialized_trees[&0][0];
        assert_eq!(
            (
                tree.root_sentinel_tala_id,
                tree.node_tala_id,
                tree.sentinel_edge_tala_id,
                tree.orientation
            ),
            (0, 3, 0, 4)
        );
        assert_eq!(
            graph.serialized_hierarchies,
            vec![d2_rs_core::SerializedHierarchyState {
                levels: [(1, 0), (2, 1)].into_iter().collect()
            }]
        );
        assert_eq!(
            graph.serialized_directions,
            [
                (0, d2_rs_core::Direction::Left),
                (1, d2_rs_core::Direction::Up)
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(graph.serialized_cell_size, Some(37.5));
        assert_eq!(graph.serialized_routing_costs, Some((1.25, 2.5, 3.75)));
    }

    #[test]
    fn reconstruction_rejects_missing_near_and_auxiliary_references() {
        let cases: &[(&[u8], &str)] = &[
            (
                br#"{"nodes":[{"id":1,"nears":[9]}]}"#,
                "could not find node 1 near node 9",
            ),
            (
                br#"{"nodes":[{"id":1}],"hubs":{"9":[1]}}"#,
                "hub 9 is missing from g.Nodes",
            ),
            (
                br#"{"nodes":[{"id":1}],"hierarchies":[{"Levels":{"9":0}}]}"#,
                "hierarchy references unknown node 9",
            ),
            (
                br#"{"nodes":[{"id":1}],"clusters":{"9":{"Nodes":[1,1]}}}"#,
                "cluster 9 has no node or serialized vessel",
            ),
            (
                br#"{"nodes":[{"id":1},{"id":2}],"sequences":{"9":{"Nodes":[1,2]}},"sequenceVessels":{"9":{"id":8}}}"#,
                "sequence vessel key 9 does not match node ID 8",
            ),
            (
                br#"{"nodes":[{"id":1},{"id":2}],"edges":[{"id":7,"from":1,"to":2}],"clusters":{"9":{"Nodes":[1,2],"EdgeAbductions":[{"Edge":8,"OriginallyFromNode":1,"OriginallyToNode":2,"CurrentFromNode":9,"CurrentToNode":2}]}},"clusterVessels":{"9":{"id":9}}}"#,
                "couldn't find edge abduction edge 8 for cluster 9",
            ),
            (
                br#"{"nodes":[{"id":1},{"id":2}],"edges":[{"id":7,"from":1,"to":2}],"sequences":{"9":{"Nodes":[1,2],"EdgeAbductions":[{"Edge":7,"OriginallyFromNode":1,"OriginallyToNode":99,"CurrentFromNode":9,"CurrentToNode":2}]}},"sequenceVessels":{"9":{"id":9}}}"#,
                "couldn't find edge abduction OriginallyTo 99 for sequence 9",
            ),
            (
                br#"{"nodes":[{"id":1}],"sequences":{"9":{}},"sequenceVessels":{"9":{"id":9}}}"#,
                "sequence 9 has 0 steps; want at least 2",
            ),
        ];
        for (input, expected) in cases {
            let error = super::graphjson_to_graph(input).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
    // Values reproduced by the pristine Go wire oracle in upstream fixtures.
    // Compare bits, not formatted decimal strings or approximate equality.
    #[test]
    fn cached_cost_decimals_preserve_reference_float_bits() {
        for (decimal, bits) in [
            ("234.89740799999998", 0x406d5cb790fb6566_u64),
            ("198.18086399999999", 0x4068c5c9a34ca0c2_u64),
            ("111.67735124819399", 0x405beb59b90cb978_u64),
            ("124.63576111343967", 0x405f28b04f6192af_u64),
            ("1030.2862499999999", 0x409019251eb851eb_u64),
            ("183.15737768378318", 0x4066e5093cec9f02_u64),
        ] {
            let input = format!("{{\"crossingCost\":{decimal}}}");
            let graph: SerializedGraph = serde_json::from_str(&input).unwrap();
            assert_eq!(graph.crossing_cost.to_bits(), bits, "{decimal}");
        }
    }
    #[test]
    fn unknown_fields_are_rejected_at_nested_boundaries() {
        assert!(
            serde_json::from_str::<SerializedGraph>(r#"{"nodes":[{"id":1,"misspelled":true}]}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<SerializedGraph>(r#"{"clusters":{"1":{"misspelled":true}}}"#)
                .is_err()
        );
    }
    #[test]
    fn signed_identity_and_nullable_values_are_retained() {
        let g: SerializedGraph=serde_json::from_str(r#"{"nodes":[{"id":-9223372036854775808,"width":null,"nears":[]}],"edges":[{"id":-1,"originalId":0,"points":[null]}],"containers":{"-9223372036854775808":null}}"#).unwrap();
        let n = &g.nodes.as_ref().unwrap()[0];
        assert_eq!(n.id, i64::MIN);
        assert_eq!(n.width, 0.0);
        assert_eq!(n.nears, Some(vec![]));
        let e = &g.edges.as_ref().unwrap()[0];
        assert_eq!(e.original_id, Some(0));
        assert!(e.points.as_ref().unwrap()[0].is_none());
        assert!(g.containers.as_ref().unwrap()[&i64::MIN].is_none());
    }
    #[test]
    fn grouping_and_temporary_endpoint_fields_survive_wire_serialization() {
        let g: SerializedGraph=serde_json::from_str(r#"{"CellSize":40,"clusters":{"9":{"Nodes":[1,2],"EdgeAbductions":[{"Edge":-1,"OriginallyFromNode":1,"OriginallyToNode":2,"CurrentFromNode":9,"CurrentToNode":2}],"Container":9,"FixedSize":true}},"hierarchies":[{"Levels":{"1":2}}],"trees":{"0":[{"Node":{"id":3},"SentinelEdge":{"id":-2,"originalId":0},"Orientation":4}]}}"#).unwrap();
        let value = serde_json::to_value(&g).unwrap();
        assert_eq!(
            value["clusters"]["9"]["EdgeAbductions"][0]["CurrentFromNode"],
            9
        );
        assert_eq!(value["hierarchies"][0]["Levels"]["1"], 2);
        assert_eq!(value["trees"]["0"][0]["SentinelEdge"]["originalId"], 0);
    }
}

/// Complete codec output for a prepared, unlaid-out diagnostic graph.
#[cfg(feature = "diagnostic-traces")]
pub(super) fn diagnostic_graphjson_state(
    arena: &crate::internal::ArenaGraph,
    input: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let serialized = decode::graph(&serde_json::to_vec(input)?)?;
    let (_, nodes, edges) = reconstruct::to_core_graph_with_records(&serialized)?;
    Ok(serde_json::to_value(serialize::graph(
        arena,
        &nodes,
        &edges,
        &serialized,
    )?)?)
}

#[cfg(test)]
mod route_storage_integration_tests {
    use super::*;
    use crate::internal::{ArenaGraph, EdgeId, Point};
    #[test]
    fn decoded_routes_keep_nullable_slots_growth_and_serialization() {
        let input = br#"{"nodes":[{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20}],"edges":[{"id":3,"from":1,"to":2,"points":[{"x":1,"y":2},null,{"x":3,"y":4}]}],"containers":{"0":[1,2]}}"#;
        let wire = decode::graph(input).unwrap();
        let (mut core, nodes, edges) = reconstruct::to_core_graph_with_records(&wire).unwrap();
        let mut arena = ArenaGraph::from_input(&core);
        arena.initialize_shared_topology();
        let route = &arena.edge_records[0].route;
        assert_eq!((route.len(), route.capacity()), (3, 4));
        assert!(route.header().get(1).unwrap().is_none());
        assert!(route.header().read_capacity_slot(3).unwrap().is_none());
        let output = serialize::graph(&arena, &nodes, &edges, &wire).unwrap();
        let points = output.edges.unwrap()[0].points.clone().unwrap();
        assert_eq!(points.len(), 3);
        assert!(points[1].is_none());
        assert_eq!(points[2].as_ref().unwrap().x, 3.0);
        core.set_edge_route(EdgeId(0), vec![Point { x: 7.0, y: 8.0 }]);
        let replaced = ArenaGraph::from_input(&core);
        assert_eq!(replaced.edge_records[0].route.len(), 1);
        assert_eq!(
            replaced.edge_records[0].route.point(0),
            Point { x: 7.0, y: 8.0 }
        );
    }
}

#[cfg(test)]
mod decoded_membership_tests {
    use super::*;
    use crate::internal::{ArenaGraph, OwnerGraphId};
    use d2_rs_core::{EdgeId, NodeId};

    fn arena(input: &[u8]) -> ArenaGraph {
        ArenaGraph::from_input(&graphjson_to_graph(input).unwrap())
    }

    #[test]
    fn empty_decoded_members_do_not_publish_tree_only_records() {
        let g = arena(br#"{"nodes":[],"edges":[],"trees":{"0":[{"Node":{"id":3,"width":20,"height":20},"SentinelEdge":{"id":-2,"from":3,"to":3},"Orientation":4}]}}"#);
        assert_eq!((g.nodes.len(), g.edge_records.len()), (1, 1));
        assert!(g.node_order.is_empty() && g.edge_order.is_empty());
        assert!(g.nodes[0].edges.is_empty());
        assert!(g.containers.is_empty());
        let header = g.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        assert!(!header.is_nil());
        assert_eq!((header.len(), header.capacity()), (0, 0));
        assert_eq!(g.topology_node_owner(NodeId(0)), Some(OwnerGraphId(0)));
    }

    #[test]
    fn tree_only_edges_do_not_acquire_graph_membership_or_incidence() {
        let g = arena(br#"{"nodes":[{"id":1,"width":20,"height":20}],"edges":[{"id":7,"from":1,"to":1}],"trees":{"1":[{"Node":{"id":3,"width":20,"height":20},"SentinelEdge":{"id":8,"from":3,"to":1},"Orientation":4}]}}"#);
        assert_eq!(g.node_order, vec![NodeId(0)]);
        assert_eq!(g.edge_order, vec![EdgeId(0)]);
        assert_eq!(g.nodes[0].edges, vec![EdgeId(0)]);
        assert!(g.nodes[1].edges.is_empty());
        assert_eq!(g.edge_records[1].from, NodeId(1));
        assert_eq!(g.edge_records[1].to, NodeId(0));
    }

    #[test]
    fn map_only_vessels_are_interned_without_owner_or_membership() {
        for (groups, vessels) in [
            ("clusters", "clusterVessels"),
            ("sequences", "sequenceVessels"),
        ] {
            let input = serde_json::json!({
                "nodes": [{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20}],
                "containers":{"0":[1,2]},
                groups: {"999":{"Nodes":[1,2]}},
                vessels: {"999":{"id":999,"width":40,"height":50,"nears":[9876]}},
                "hubs":{"999":[1,999]}
            });
            let g = arena(&serde_json::to_vec(&input).unwrap());
            assert_eq!(g.nodes.len(), 3);
            assert_eq!(g.node_order, vec![NodeId(0), NodeId(1)]);
            assert_eq!(
                g.nodes[2].rect.size,
                d2_rs_core::Size {
                    width: 40.0,
                    height: 50.0
                }
            );
            assert_eq!(g.topology_node_owner(NodeId(2)), None);
            assert_eq!(g.topology_node_owner(NodeId(0)), Some(OwnerGraphId(0)));
            assert!(g.nodes[2].edges.is_empty());
            assert_eq!(g.hubs.get(&NodeId(2)), Some(vec![NodeId(0), NodeId(2)]));
            assert_eq!(g.containers.get(&None), Some(vec![NodeId(0), NodeId(1)]));
        }
    }

    #[test]
    fn existing_vessel_pointer_ignores_shadow_record_and_unused_map_entries() {
        let core = graphjson_to_graph(br#"{"nodes":[{"id":1,"width":20,"height":30},{"id":2,"width":20,"height":20}],"clusters":{"1":{"Nodes":[2]}},"clusterVessels":{"1":{"id":99,"width":800,"height":900},"8":{"id":8}},"sequences":{"1":{"Nodes":[1,2]}},"sequenceVessels":{"1":{"id":77}}}"#).unwrap();
        assert!(core.tala_decoded_nil_owners.is_empty());
        assert_eq!(core.nodes().count(), 2);
        assert_eq!(
            core.serialized_clusters[0].vessel_size,
            d2_rs_core::Size {
                width: 20.0,
                height: 30.0
            }
        );
        assert_eq!(
            core.serialized_sequences[0].vessel_size,
            core.serialized_clusters[0].vessel_size
        );
    }

    #[test]
    fn groups_resolve_only_previously_interned_vessels() {
        let core = graphjson_to_graph(br#"{"nodes":[{"id":1},{"id":2}],"clusters":{"9":{"Nodes":[1,2]}},"clusterVessels":{"9":{"id":9}},"sequences":{"10":{"Nodes":[9,10],"Container":9}},"sequenceVessels":{"10":{"id":10}},"hubs":{"9":[10]}}"#).unwrap();
        assert_eq!(core.nodes().count(), 4);
        assert_eq!(core.tala_decoded_nil_owners.len(), 2);
        for input in [
            r#"{"nodes":[{"id":1}],"clusters":{"9":{"Nodes":[10]},"10":{"Nodes":[1]}},"clusterVessels":{"9":{"id":9},"10":{"id":10}}}"#,
            r#"{"nodes":[{"id":1}],"clusters":{"9":{"Nodes":[10]}},"clusterVessels":{"9":{"id":9}},"sequences":{"10":{"Nodes":[1,9]}},"sequenceVessels":{"10":{"id":10}}}"#,
        ] {
            let error = graphjson_to_graph(input.as_bytes())
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                "error while deserializing: cluster 9 has node 10 which is missing from g.Nodes"
            );
        }
    }

    #[test]
    fn early_references_cannot_see_auxiliary_records() {
        for extra in [
            serde_json::json!({"edges":[{"id":7,"from":1,"to":9}]}),
            serde_json::json!({"containers":{"0":[9]}}),
            serde_json::json!({"hubs":{"9":[1]}}),
        ] {
            let mut input = serde_json::json!({"nodes":[{"id":1}],"trees":{"0":[{"Node":{"id":9},"SentinelEdge":{"id":8,"from":9,"to":1}}]}});
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(graphjson_to_graph(&serde_json::to_vec(&input).unwrap()).is_err());
        }
        let input = br#"{"nodes":[{"id":1}],"edges":[{"id":7,"from":1,"to":1}],"clusters":{"9":{"Nodes":[1],"EdgeAbductions":[{"Edge":7,"CurrentFromNode":8}]}},"clusterVessels":{"9":{"id":9},"8":{"id":8}}}"#;
        assert_eq!(
            graphjson_to_graph(input).unwrap_err().to_string(),
            "couldn't find edge abduction CurrentFrom 8 for cluster 9"
        );
    }

    #[test]
    fn tree_records_reuse_existing_pointers_and_check_nears_before_publication() {
        let input = br#"{"nodes":[{"id":1,"width":20,"height":30}],"edges":[{"id":-7,"from":1,"to":1,"originalId":0}],"trees":{"1":[{"Node":{"id":1,"width":20,"height":30},"SentinelEdge":{"id":-7,"from":1,"to":1,"originalId":0}}]}}"#;
        let core = graphjson_to_graph(input).unwrap();
        assert_eq!(core.nodes().count(), 1);
        assert_eq!(core.serialized_trees[&1][0].sentinel_edge_tala_id, 0);
        let g = ArenaGraph::from_input(&core);
        assert_eq!(g.nodes[0].rect.size.width, 20.0);
        assert_eq!(g.edge_records.len(), 1);
        let input = br#"{"nodes":[],"trees":{"0":[{"Node":{"id":9,"nears":[9]},"SentinelEdge":{"id":7,"from":9,"to":9}}]}}"#;
        assert_eq!(
            graphjson_to_graph(input).unwrap_err().to_string(),
            "could not find tree node near of ID: 9"
        );
    }

    #[test]
    fn zero_id_records_remain_rejected_before_interning() {
        for input in [br#"{"nodes":[{"id":0},{"id":1}],"containers":{"0":[1]}}"#.as_slice(),
            br#"{"nodes":[{"id":1}],"clusters":{"0":{"Nodes":[1]}},"clusterVessels":{"0":{"id":0}}}"#] {
            assert!(graphjson_to_graph(input).unwrap_err().to_string().contains("uses reserved node ID 0"));
        }
    }

    #[test]
    fn decoded_container_map_does_not_invent_unspecified_children() {
        let g = arena(br#"{"nodes":[{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20},{"id":3,"width":20,"height":20}],"containers":{"0":[],"2":[3],"1":[]}}"#);
        assert_eq!(g.containers.len(), 3);
        assert_eq!(g.containers.get(&None), Some(vec![]));
        assert_eq!(g.containers.get(&Some(NodeId(0))), Some(vec![]));
        assert_eq!(g.containers.get(&Some(NodeId(1))), Some(vec![NodeId(2)]));
        assert!(g.nodes[0].is_container && g.nodes[1].is_container);
        assert_eq!(g.nodes[2].container, Some(NodeId(1)));
    }

    #[test]
    fn filtered_members_keep_original_make_capacity_and_nil_tail() {
        let g = arena(br#"{"nodes":[{"id":1,"width":20,"height":20},{"id":2,"width":20,"height":20},{"id":9,"width":20,"height":20}],"clusters":{"9":{"Nodes":[1,2]}}}"#);
        let header = g.owner_graph_member_header(OwnerGraphId(0)).unwrap();
        assert_eq!((header.len(), header.capacity()), (1, 3));
        assert_eq!(header.read_capacity_slot(0), Some(Some(NodeId(2))));
        assert_eq!(header.read_capacity_slot(1), Some(None));
        assert_eq!(header.read_capacity_slot(2), Some(None));
        assert_eq!(g.node_order, vec![NodeId(2)]);
    }

    #[test]
    fn decoded_container_headers_follow_buffered_append_and_heap_growth() {
        // Go 1.27 codec.go uses growsliceBuf with four pointer slots. These
        // expectations cover the small size classes and the escape boundary.
        for (count, capacity) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 8)] {
            let nodes: Vec<_> = (1..=count)
                .map(|id| serde_json::json!({"id": id, "width": 20, "height": 20}))
                .collect();
            let input = serde_json::json!({
                "nodes": nodes,
                "containers": {"0": (1..=count).collect::<Vec<_>>()},
            });
            let g = arena(&serde_json::to_vec(&input).unwrap());
            let header = g.containers.get_header(&None).unwrap();
            assert!(!header.is_nil());
            assert_eq!((header.len(), header.capacity()), (count, capacity));
            for index in 0..capacity {
                assert_eq!(
                    header.read_capacity_slot(index),
                    Some((index < count).then_some(NodeId(index as u32))),
                );
            }
        }
    }
}

#[cfg(test)]
mod cluster_creation_tests;

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tree_publication_tests;
