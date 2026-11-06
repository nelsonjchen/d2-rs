// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::internal::{Pipeline, shared_topology::OwnerGraphId};

fn fixture() -> serde_json::Value {
    serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/tree-publication-source-v090.json"
    ))
    .unwrap()
}

#[test]
fn original_complete_tree_publication_stages_before_placement() {
    let fixture = fixture();
    let input = decode::graph(&serde_json::to_vec(&fixture["input"]).unwrap()).unwrap();
    let (core, nodes, edges) = reconstruct::to_core_graph_with_records(&input).unwrap();
    let mut pipeline =
        Pipeline::new_untraced(&core, fixture["seed"].as_i64().unwrap(), false, false);
    for record in fixture["stages"].as_array().unwrap() {
        let stage = record["stage"].as_str().unwrap();
        match stage {
            "Prescale" => pipeline.run_prescale(),
            "PreprocessSequences" => pipeline.run_preprocess_sequences(),
            "Preprocess" => pipeline.run_preprocess(),
            "PreprocessTrees" => pipeline.run_preprocess_trees(),
            "PreprocessHierarchies" => {
                pipeline.run_preprocess_containers();
                pipeline.run_preprocess_hierarchies();
            }
            "PreprocessClusters" => pipeline.run_preprocess_clusters(),
            "PreprocessHubs" => pipeline.run_preprocess_hubs(),
            _ => panic!("unexpected source stage {stage}"),
        }
        let actual = serialize::graph(&pipeline.graph, &nodes, &edges, &input).unwrap();
        let expected = decode::graph(&serde_json::to_vec(&record["graph"]).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap(),
            "{stage}"
        );
    }
}

#[test]
fn tree_serialization_reads_live_fields_and_replaced_map_without_projection_fallback() {
    let fixture = fixture();
    let input = decode::graph(&serde_json::to_vec(&fixture["input"]).unwrap()).unwrap();
    let (core, nodes, edges) = reconstruct::to_core_graph_with_records(&input).unwrap();
    let mut pipeline = Pipeline::new_untraced(&core, 1, false, false);
    pipeline.run_prescale();
    pipeline.run_preprocess_sequences();
    pipeline.run_preprocess();
    pipeline.run_preprocess_trees();
    let map = pipeline
        .graph
        .owner_graph_tree_roots_map(OwnerGraphId(0))
        .unwrap();
    let sentinel = map.live_keys().next().expect("actual branching tree");
    let roots = pipeline
        .graph
        .owner_tree_roots_map_entry(&map, sentinel)
        .unwrap()
        .unwrap();
    let tree = roots.get(0).flatten().unwrap();
    for orientation in [0, 5, 8] {
        let mut fields = pipeline.graph.owner_tree(tree).unwrap().fields();
        fields.orientation = orientation;
        pipeline.graph.set_owner_tree_fields(tree, fields).unwrap();
        let actual = serialize::graph(&pipeline.graph, &nodes, &edges, &input).unwrap();
        assert_eq!(
            actual
                .trees
                .unwrap()
                .values()
                .next()
                .unwrap()
                .as_ref()
                .unwrap()[0]
                .orientation,
            orientation
        );
    }
    let nil = pipeline.graph.allocate_owner_tree_roots_map(false);
    pipeline
        .graph
        .set_owner_graph_tree_roots_map(OwnerGraphId(0), nil)
        .unwrap();
    let actual = serialize::graph(&pipeline.graph, &nodes, &edges, &input).unwrap();
    assert!(actual.trees.unwrap().is_empty());
    pipeline
        .graph
        .set_owner_graph_tree_roots_map(OwnerGraphId(0), map)
        .unwrap();
    pipeline
        .graph
        .write_owner_tree_roots_slot(&roots, 0, None)
        .unwrap();
    let error = serialize::graph(&pipeline.graph, &nodes, &edges, &input)
        .unwrap_err()
        .to_string();
    assert_eq!(error, "cannot serialize a nil tree");
}
