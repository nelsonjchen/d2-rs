// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 graphjson/codec.go: serialize current membership and retained groups.
//! Source-only identity/text/style metadata accompanies the reconstructed arena;
//! mutable geometry, endpoints, ordering and group fields come from that arena.
use super::wire::*;
use crate::internal::owner_groups::{OwnerGroupId, OwnerGroupKind};
use crate::internal::owner_tree_map::OwnerTreeId;
use crate::internal::shared_topology::{OwnerAbductionHeader, OwnerGraphId};
use crate::internal::{ArenaGraph, EdgeId, NodeId, Point};
use d2_rs_core::stable_sort::StableSort;
use std::collections::{BTreeMap, BTreeSet};

type Error = Box<dyn std::error::Error>;
fn point(p: Point) -> SerializedPoint {
    SerializedPoint { x: p.x, y: p.y }
}

struct Encoder<'a> {
    arena: &'a ArenaGraph,
    node_metadata: &'a [SerializedNode],
    edge_metadata: &'a [SerializedEdge],
    input: &'a SerializedGraph,
    edge_ids: BTreeMap<EdgeId, i64>,
    used_edge_ids: BTreeSet<i64>,
    next_edge_id: i64,
}
impl Encoder<'_> {
    fn node_id(&self, node: Option<NodeId>) -> Result<i64, Error> {
        node.map_or(Ok(0), |id| {
            self.arena
                .nodes
                .get(id.0 as usize)
                .map(|n| n.tala_id)
                .ok_or_else(|| "cannot serialize an unknown node".into())
        })
    }
    fn edge_id(&mut self, id: EdgeId) -> Result<i64, Error> {
        if let Some(value) = self.edge_ids.get(&id) {
            return Ok(*value);
        }
        let edge = self
            .arena
            .edge_records
            .get(id.0 as usize)
            .ok_or("cannot serialize an unknown edge")?;
        let value = if edge.tala_id != 0 {
            edge.tala_id
        } else {
            while self.used_edge_ids.contains(&self.next_edge_id) || self.next_edge_id == 0 {
                self.next_edge_id = self
                    .next_edge_id
                    .checked_sub(1)
                    .ok_or("serialization edge IDs exhausted")?;
            }
            let value = self.next_edge_id;
            self.next_edge_id = self
                .next_edge_id
                .checked_sub(1)
                .ok_or("serialization edge IDs exhausted")?;
            value
        };
        self.used_edge_ids.insert(value);
        self.edge_ids.insert(id, value);
        Ok(value)
    }
    fn node(&self, id: NodeId) -> Result<SerializedNode, Error> {
        let n = self
            .arena
            .nodes
            .get(id.0 as usize)
            .ok_or("cannot serialize an unknown node")?;
        let metadata = self
            .arena
            .topology_source_input_node(id)
            .and_then(|input| self.node_metadata.get(input.0 as usize))
            .or_else(|| {
                self.input
                    .cluster_vessels
                    .as_ref()
                    .and_then(|m| m.get(&n.tala_id))
            })
            .or_else(|| {
                self.input
                    .sequence_vessels
                    .as_ref()
                    .and_then(|m| m.get(&n.tala_id))
            });
        let mut nears = n
            .nears
            .raw_keys()
            .into_iter()
            .map(|id| self.node_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        nears.stable_sort();
        Ok(SerializedNode {
            id: n.tala_id,
            d2_id: metadata.and_then(|m| m.d2_id.clone()),
            is_invisible: n.is_invisible,
            width: n.rect.size.width,
            height: n.rect.size.height,
            top_left: self.arena.position(id).map(point),
            fixed_top_left: n.fixed_top_left.map(point),
            desired_width: n.desired_width,
            desired_height: n.desired_height,
            nears: Some(nears),
            font_size: n.font_size.map(i64::from),
            shape_type: self.arena.node_shape_type(id).into(),
            label: n.label_size.map(|size| SerializedLabel {
                text: metadata
                    .and_then(|m| m.label.as_ref())
                    .map_or_else(String::new, |l| l.text.clone()),
                position: n.label_position.d2_name().into(),
                width: size.width,
                height: size.height,
            }),
            icon: n.has_icon.then(|| SerializedIcon {
                position: n
                    .icon_position
                    .unwrap_or(crate::LabelPosition::Unset)
                    .d2_name()
                    .into(),
            }),
            num_columns: n.table_column_count.unwrap_or(0) as i64,
            is3_d: n.is_3d,
            is_multiple: n.is_multiple,
            force_hierarchy: n.force_hierarchy,
        })
    }
    fn edge(&mut self, id: EdgeId) -> Result<SerializedEdge, Error> {
        let serial_id = self.edge_id(id)?;
        let e = &self.arena.edge_records[id.0 as usize];
        let metadata = self.edge_metadata.get(e.input_id.0 as usize);
        let label = |l: &crate::EdgeLabel| SerializedLabel {
            text: l.text.clone(),
            position: l.position.d2_name().into(),
            width: l.size.width,
            height: l.size.height,
        };
        let arrow_label =
            |l: &crate::ArrowheadLabel, position: Option<crate::LabelPosition>| SerializedLabel {
                text: l.text.clone(),
                position: position
                    .unwrap_or(crate::LabelPosition::Unset)
                    .d2_name()
                    .into(),
                width: l.size.width,
                height: l.size.height,
            };
        Ok(SerializedEdge {
            id: serial_id,
            original_id: (serial_id != e.tala_id).then_some(e.tala_id),
            d2_id: metadata.and_then(|m| m.d2_id.clone()),
            from_node: self.node_id(Some(e.from))?,
            to_node: self.node_id(Some(e.to))?,
            is_invisible: e.invisible,
            points: Some(
                e.route
                    .header()
                    .iter_slots()
                    .map(|p| p.map(|p| point(p.get())))
                    .collect(),
            ),
            min_width: e.min_width as i64,
            min_height: e.min_height as i64,
            label_percentage: e.label.as_ref().map_or_else(
                || metadata.map_or(0.0, |m| m.label_percentage),
                |l| l.percentage,
            ),
            label: e.label.as_ref().map(label),
            from_table_column_index: e.source_table_column.map(|x| x as i64),
            to_table_column_index: e.target_table_column.map(|x| x as i64),
            style: metadata.map_or_else(SerializedStyle::default, |m| m.style.clone()),
            source_arrowhead: e.source_arrowhead.clone().unwrap_or_default(),
            target_arrowhead: e.target_arrowhead.clone().unwrap_or_default(),
            source_arrowhead_label: e
                .source_arrowhead_label
                .as_ref()
                .map(|l| arrow_label(l, e.source_label_positions[1])),
            target_arrowhead_label: e
                .target_arrowhead_label
                .as_ref()
                .map(|l| arrow_label(l, e.source_label_positions[2])),
        })
    }
    fn groups(&self, kind: OwnerGroupKind) -> Result<Vec<(NodeId, OwnerGroupId)>, Error> {
        let map = self.arena.owner_graph_group_map(OwnerGraphId(0), kind)?;
        let mut keys = self.arena.owner_group_map_keys(&map)?;
        keys.stable_sort_by_key(|n| self.node_id(*n).unwrap_or(0));
        keys.into_iter()
            .map(|key| {
                Ok((
                    key.ok_or("cannot serialize a nil vessel")?,
                    self.arena
                        .owner_group_map_entry(&map, key)?
                        .flatten()
                        .ok_or("cannot serialize a nil group")?,
                ))
            })
            .collect()
    }
    fn abductions(
        &mut self,
        header: Option<OwnerAbductionHeader>,
    ) -> Result<Vec<SerializedEdgeAbduction>, Error> {
        let mut output = Vec::new();
        for id in header.iter().flat_map(|h| h.iter_slots()) {
            let record = *self
                .arena
                .owner_edge_abduction(id.ok_or("cannot serialize a nil edge abduction")?)
                .ok_or("cannot serialize an unknown edge abduction")?;
            output.push(SerializedEdgeAbduction {
                edge: record
                    .source_edge
                    .map_or(Ok(0), |edge| self.edge_id(edge))?,
                originally_from_node: self.node_id(record.originally_from)?,
                originally_to_node: self.node_id(record.originally_to)?,
                current_from_node: self.node_id(record.current_from)?,
                current_to_node: self.node_id(record.current_to)?,
            });
        }
        Ok(output)
    }
    fn tree(&mut self, tree: OwnerTreeId, depth: usize) -> Result<SerializedTree, Error> {
        if depth > 256 {
            return Err("TALA SerializeGraph tree depth exceeds limit 256".into());
        }
        let fields = self
            .arena
            .owner_tree(tree)
            .ok_or("cannot serialize an unknown tree")?
            .fields();
        let children = (0..fields.children.len())
            .map(|index| {
                let child = fields
                    .children
                    .get(index)
                    .flatten()
                    .ok_or("cannot serialize a nil tree child")?;
                self.tree(child, depth + 1)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SerializedTree {
            node: self.node(fields.node.ok_or("cannot serialize a nil tree node")?)?,
            children: Some(children),
            sentinel_edge: self.edge(
                fields
                    .sentinel_edge
                    .ok_or("cannot serialize a tree with a nil sentinel edge")?,
            )?,
            orientation: fields.orientation,
        })
    }
}

pub(super) fn graph(
    arena: &ArenaGraph,
    node_metadata: &[SerializedNode],
    edge_metadata: &[SerializedEdge],
    input: &SerializedGraph,
) -> Result<SerializedGraph, Error> {
    let mut encoder = Encoder {
        arena,
        node_metadata,
        edge_metadata,
        input,
        edge_ids: BTreeMap::new(),
        used_edge_ids: arena
            .edge_records
            .iter()
            .filter_map(|e| (e.tala_id != 0).then_some(e.tala_id))
            .collect(),
        next_edge_id: -1,
    };
    let clusters = encoder.groups(OwnerGroupKind::Cluster)?;
    let sequences = encoder.groups(OwnerGroupKind::Sequence)?;
    // codec.go preallocates unset references in Graph.Edges order, before
    // aggregate abductions and tree sentinel edges can reference them.
    for &id in &arena.edge_order.read() {
        encoder.edge_id(id)?;
    }
    let mut nodes = Vec::new();
    let mut included = BTreeSet::new();
    let members = arena.original_owner_member_nodes();
    for &id in &members {
        nodes.push(encoder.node(id)?);
        included.insert(id);
    }
    for &(vessel, group) in clusters.iter().chain(&sequences) {
        included.insert(vessel);
        for node in arena.owner_group_fields(group)?.nodes.iter_slots() {
            let node = node.ok_or("cannot serialize a nil group member")?;
            if included.insert(node) {
                nodes.push(encoder.node(node)?);
            }
        }
    }
    let edges = arena
        .edge_order
        .read()
        .iter()
        .map(|&id| encoder.edge(id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut containers = BTreeMap::new();
    for parent in arena.containers.keys() {
        let header = arena
            .containers
            .get_header(&parent)
            .ok_or("missing container slice")?;
        let children = header
            .iter_slots()
            .map(|child| {
                let child = child.ok_or("cannot serialize a nil container child")?;
                if !included.contains(&child) {
                    return Err("cannot serialize a container child missing from nodes".into());
                }
                encoder.node_id(Some(child))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        containers.insert(encoder.node_id(parent)?, Some(children));
    }
    let mut serialized_clusters = BTreeMap::new();
    let mut cluster_vessels = BTreeMap::new();
    for (vessel, group) in clusters {
        let fields = arena.owner_group_fields(group)?;
        let policy = arena.owner_cluster_policy(group)?;
        let id = encoder.node_id(Some(vessel))?;
        let nodes = fields
            .nodes
            .iter_slots()
            .map(|n| encoder.node_id(n))
            .collect::<Result<Vec<_>, _>>()?;
        serialized_clusters.insert(
            id,
            SerializedCluster {
                nodes: Some(nodes),
                arrangement: policy.arrangement,
                desired_arrangement: policy.desired_arrangement,
                edge_abductions: Some(encoder.abductions(policy.edge_abductions)?),
                container: encoder.node_id(fields.container)?,
                fixed_size: policy.fixed_size,
                padding: policy.padding,
            },
        );
        cluster_vessels.insert(id, encoder.node(vessel)?);
    }
    let mut serialized_sequences = BTreeMap::new();
    let mut sequence_vessels = BTreeMap::new();
    for (vessel, group) in sequences {
        let fields = arena.owner_group_fields(group)?;
        let id = encoder.node_id(Some(vessel))?;
        serialized_sequences.insert(
            id,
            SerializedSequence {
                nodes: Some(
                    fields
                        .nodes
                        .iter_slots()
                        .map(|n| encoder.node_id(n))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                edge_abductions: Some(encoder.abductions(arena.owner_sequence_abductions(group)?)?),
                container: encoder.node_id(fields.container)?,
            },
        );
        sequence_vessels.insert(id, encoder.node(vessel)?);
    }
    let hubs = arena
        .hubs
        .header()
        .entries()
        .into_iter()
        .map(|(hub, spokes)| {
            Ok((
                encoder.node_id(hub)?,
                Some(
                    spokes
                        .iter_slots()
                        .map(|id| encoder.node_id(id))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            ))
        })
        .collect::<Result<_, Error>>()?;
    let mut trees = BTreeMap::new();
    // codec.go walks Graph.Trees, not the speculative numerical routing view.
    // Before PreprocessTrees this map is empty; decoded and later-mutated Tree
    // records retain their actual child headers, edges and orientation fields.
    let roots_map = arena.owner_graph_tree_roots_map(OwnerGraphId(0))?;
    let mut sentinels = roots_map.live_keys().collect::<Vec<_>>();
    sentinels.stable_sort_by_key(|n| n.map_or(0, |n| arena.nodes[n.0 as usize].tala_id));
    for sentinel in sentinels {
        let header = arena
            .owner_tree_roots_map_entry(&roots_map, sentinel)?
            .ok_or("cannot serialize a missing tree root header")?;
        let roots = (0..header.len())
            .map(|index| {
                let tree = header
                    .get(index)
                    .flatten()
                    .ok_or("cannot serialize a nil tree")?;
                encoder.tree(tree, 1)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !roots.is_empty() {
            trees.insert(encoder.node_id(sentinel)?, Some(roots));
        }
    }
    let mut hierarchy_ids = BTreeSet::new();
    let mut hierarchies = Vec::new();
    for id in members {
        let Some(h) = arena.nodes[id.0 as usize].hierarchy.clone() else {
            continue;
        };
        if hierarchy_ids.insert(h.record.allocation_key()) {
            let levels = arena
                .owner_hierarchy_fields(&h.record)
                .map_err(|e| e.to_string())?
                .levels
                .entries()
                .into_iter()
                .map(|(node, level)| encoder.node_id(node).map(|id| (id, level)))
                .collect::<Result<_, _>>()?;
            hierarchies.push(SerializedHierarchy {
                levels: Some(levels),
            });
        }
    }
    let directions = arena
        .scoring_directions
        .iter()
        .map(|(&id, d)| {
            Ok((
                encoder.node_id(id)?,
                match d {
                    crate::Direction::Right => "right",
                    crate::Direction::Down => "down",
                    crate::Direction::Left => "left",
                    crate::Direction::Up => "up",
                }
                .into(),
            ))
        })
        .collect::<Result<_, Error>>()?;
    let (crossing_cost, turn_cost, non_center_port_cost) = arena.cost_cache.raw();
    Ok(SerializedGraph {
        nodes: Some(nodes),
        edges: Some(edges),
        cell_size: arena.cell_size,
        is_root_hierarchy: arena.root_hierarchy,
        containers: Some(containers),
        hubs: Some(hubs),
        clusters: Some(serialized_clusters),
        sequences: Some(serialized_sequences),
        cluster_vessels: Some(cluster_vessels),
        sequence_vessels: Some(sequence_vessels),
        trees: Some(trees),
        hierarchies: Some(hierarchies),
        directions: Some(directions),
        crossing_cost,
        turn_cost,
        non_center_port_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    use serde_json::Value;
    #[test]
    fn node_encoding_reads_the_live_point_including_explicit_nil() {
        let mut input = crate::Graph::default();
        let node = input.add_node(crate::Node::new(
            "a",
            crate::Size {
                width: 20.,
                height: 20.,
            },
        ));
        let mut arena = ArenaGraph::from_input(&input);
        arena.set_position(node, Point { x: 1000., y: 1000. });
        arena.materialize_owner_node_point(node).unwrap();
        let retained = arena.owner_node_point(node).unwrap();
        arena.translate_owner_point(node, -1000., -1000.);
        let encode = |arena: &ArenaGraph| {
            Encoder {
                arena,
                node_metadata: &[],
                edge_metadata: &[],
                input: &SerializedGraph::default(),
                edge_ids: BTreeMap::new(),
                used_edge_ids: BTreeSet::new(),
                next_edge_id: -1,
            }
            .node(node)
            .unwrap()
        };
        assert_eq!(encode(&arena).top_left.unwrap().x, 0.);
        assert_eq!(retained.get(), Some(Point { x: 0., y: 0. }));
        let nil = arena.allocate_owner_node_point(None);
        arena.bind_owner_node_point(node, nil).unwrap();
        // An inline copy cannot revive a source nil pointer.
        arena.nodes[node.0 as usize].position = Some(Point { x: 99., y: 99. });
        assert!(encode(&arena).top_left.is_none());
    }
    // Compare every field and ordered array, allowing only JSON's common
    // integer/floating spelling for the same exactly represented number.
    #[cfg(feature = "oracle-fixtures")]
    fn exact(a: &Value, b: &Value, path: &str) {
        match (a, b) {
            (Value::Number(a), Value::Number(b)) => {
                if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
                    assert_eq!(a, b, "{path}");
                } else {
                    assert_eq!(
                        a.as_f64().unwrap().to_bits(),
                        b.as_f64().unwrap().to_bits(),
                        "{path}"
                    );
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(a.len(), b.len(), "{path}");
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    exact(a, b, &format!("{path}[{i}]"));
                }
            }
            (Value::Object(a), Value::Object(b)) => {
                assert_eq!(
                    a.keys().collect::<BTreeSet<_>>(),
                    b.keys().collect::<BTreeSet<_>>(),
                    "{path}"
                );
                for (key, a) in a {
                    exact(a, &b[key], &format!("{path}.{key}"));
                }
            }
            _ => assert_eq!(a, b, "{path}"),
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn arrowhead_label_codec_matches_every_source_position_and_unknown_names() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/arrowhead-label-codec-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 144);
        for case in cases {
            let wire =
                super::super::decode::graph(&serde_json::to_vec(&case["input"]).unwrap()).unwrap();
            let (core, nodes, edges) =
                super::super::reconstruct::to_core_graph_with_records(&wire).unwrap();
            let mut arena = ArenaGraph::from_input(&core);
            arena.initialize_shared_topology();
            // Observe the decoded codec state before layout chooses a cell size.
            arena.cell_size = wire.cell_size;
            let result = super::graph(&arena, &nodes, &edges, &wire).unwrap();
            exact(
                &case["source"],
                &serde_json::to_value(result).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[test]
    fn arrowhead_label_encoding_reads_live_positions_after_decode() {
        use crate::LabelPosition;
        let input = br#"{"nodes":[{"id":1},{"id":2}],"edges":[{"id":3,"from":1,"to":2,"sourceArrowheadLabel":{"Position":"UNLOCKED_TOP"},"targetArrowheadLabel":{"Position":"UNLOCKED_BOTTOM"}}],"containers":{"0":[1,2]}}"#;
        let wire = super::super::decode::graph(input).unwrap();
        let (core, nodes, edges) =
            super::super::reconstruct::to_core_graph_with_records(&wire).unwrap();
        let mut arena = ArenaGraph::from_input(&core);
        arena.initialize_shared_topology();
        arena.edge_records[0].source_label_positions[1] = Some(LabelPosition::UnlockedMiddle);
        arena.edge_records[0].source_label_positions[2] = Some(LabelPosition::Unset);
        let result = super::graph(&arena, &nodes, &edges, &wire).unwrap();
        let edge = &result.edges.as_ref().unwrap()[0];
        assert_eq!(
            edge.source_arrowhead_label.as_ref().unwrap().position,
            "UNLOCKED_MIDDLE"
        );
        assert_eq!(edge.target_arrowhead_label.as_ref().unwrap().position, "");
        // Input metadata is still the original spelling; output comes from the live graph.
        assert_eq!(
            edges[0].source_arrowhead_label.as_ref().unwrap().position,
            "UNLOCKED_TOP"
        );
        assert_eq!(
            edges[0].target_arrowhead_label.as_ref().unwrap().position,
            "UNLOCKED_BOTTOM"
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    /// Complete NodePlacement output from pristine Go, including nested
    /// container members first initialized through their cluster vessel.
    #[test]
    fn original_nested_cluster_initialization_serialization_matches() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/nested-cluster-initialization-source-v090.json"
        ))
        .unwrap();
        let serialized =
            super::super::decode::graph(fixture["input_text"].as_str().unwrap().as_bytes())
                .unwrap();
        let (core, nodes, edges) =
            super::super::reconstruct::to_core_graph_with_records(&serialized).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2);
        for case in cases {
            let seed = case["seed"].as_i64().unwrap();
            let snapshot = crate::internal::layout_snapshot(
                &core,
                seed,
                crate::internal::LayoutStage::NodePlacement,
            );
            assert!(snapshot.layout_error().is_none());
            let actual = super::graph(
                snapshot.graph.as_ref().unwrap(),
                &nodes,
                &edges,
                &serialized,
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::to_value(actual).unwrap(),
                &format!("nested cluster initialization seed {seed}"),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    /// All seven preprocessing boundaries, including the two historical
    /// cloud-container failures, from the unmodified Go codec and algorithms.
    #[test]
    fn original_cloud_and_sequence_preprocessing_serialization_matches() {
        use crate::internal::Pipeline;
        let cases: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/preprocess-stage-serialization-source-v090.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 8);
        for case in cases {
            let serialized =
                super::super::decode::graph(&serde_json::to_vec(&case["input"]).unwrap()).unwrap();
            let (core, nodes, edges) =
                super::super::reconstruct::to_core_graph_with_records(&serialized).unwrap();
            let mut pipeline = Pipeline::new(&core, case["seed"].as_i64().unwrap(), false, false);
            let stages = case["stages"].as_array().unwrap();
            assert_eq!(stages.len(), 7);
            for (index, name) in [
                "Prescale",
                "PreprocessSequences",
                "Preprocess",
                "PreprocessTrees",
                "PreprocessHierarchies",
                "PreprocessClusters",
                "PreprocessHubs",
            ]
            .iter()
            .enumerate()
            {
                match index {
                    0 => pipeline.run_prescale(),
                    1 => pipeline.run_preprocess_sequences(),
                    2 => pipeline.run_preprocess(),
                    3 => pipeline.run_preprocess_trees(),
                    4 => {
                        pipeline.run_preprocess_containers();
                        pipeline.run_preprocess_hierarchies();
                    }
                    5 => pipeline.run_preprocess_clusters(),
                    6 => pipeline.run_preprocess_hubs(),
                    _ => unreachable!(),
                }
                assert_eq!(stages[index]["stage"], *name);
                let actual = serde_json::to_value(
                    super::graph(&pipeline.graph, &nodes, &edges, &serialized).unwrap(),
                )
                .unwrap();
                exact(
                    &stages[index]["graph"],
                    &actual,
                    &format!("{} / {name}", case["name"]),
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_seeded_cluster_policy_and_sequence_vessel_transactions() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/seeded-cluster-sequence-publication-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_complete_graph_records_include_groups_and_metadata() {
        let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/live-graphjson-source-v090.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 12);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_hub_incidence_order_survives_tree_reconnection_and_clusters() {
        let cases: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/hub-incidence-source-v090.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 3);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_tree_mirror_and_rejected_direct_records() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/tree-mirror-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 4);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_tree_container_order_is_independent_of_graph_node_publication() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/tree-containers-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 7);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_rejected_cluster_flip_publishes_preferred_arrangement() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/cluster-policy-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_sequence_alignment_retains_first_crossing_cost() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/crossing-cache-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_label_obstacles_follow_live_graph_membership() {
        let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/label-obstacles-source-v090.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1);
        for case in cases {
            let bytes = super::super::graphjson_layout(
                &serde_json::to_vec(&case["input"]).unwrap(),
                case["seed"].as_i64().unwrap(),
            )
            .unwrap();
            exact(
                &case["source"],
                &serde_json::from_slice(&bytes).unwrap(),
                case["name"].as_str().unwrap(),
            );
        }
    }

    #[test]
    fn current_membership_and_live_geometry_override_input_arrays() {
        let input=super::super::decode::graph(br#"{"nodes":[{"id":7,"width":40,"height":30},{"id":2,"width":60,"height":50},{"id":9,"width":20,"height":10}],"edges":[{"id":-1,"from":7,"to":2},{"id":-2,"originalId":0,"from":2,"to":9},{"id":-3,"originalId":0,"from":9,"to":7}],"containers":{"0":[7,2,9]}}"#).unwrap();
        let (core, nodes, edges) =
            super::super::reconstruct::to_core_graph_with_records(&input).unwrap();
        let mut arena = ArenaGraph::from_input(&core);
        arena.initialize_shared_topology();
        arena.node_order = vec![NodeId(2), NodeId(0), NodeId(1)];
        arena
            .edge_order
            .replace_values(&[EdgeId(2), EdgeId(1), EdgeId(0)]);
        arena
            .containers
            .insert(None, vec![NodeId(2), NodeId(0), NodeId(1)]);
        arena.nodes[2].position = Some(Point { x: 19.0, y: 23.0 });
        arena.nodes[2].rect.size.width = 37.0;
        let out = graph(&arena, &nodes, &edges, &input).unwrap();
        let out_nodes = out.nodes.unwrap();
        assert_eq!(
            out_nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![9, 7, 2]
        );
        assert_eq!(out_nodes[0].width, 37.0);
        assert_eq!(
            out_nodes[0].top_left,
            Some(SerializedPoint { x: 19.0, y: 23.0 })
        );
        let out_edges = out.edges.unwrap();
        assert_eq!(
            out_edges.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![-2, -3, -1]
        );
        assert_eq!(
            out_edges.iter().map(|e| e.original_id).collect::<Vec<_>>(),
            vec![Some(0), Some(0), None]
        );
        assert_eq!(
            arena
                .edge_records
                .iter()
                .map(|e| e.tala_id)
                .collect::<Vec<_>>(),
            vec![-1, 0, 0]
        );
    }
}
