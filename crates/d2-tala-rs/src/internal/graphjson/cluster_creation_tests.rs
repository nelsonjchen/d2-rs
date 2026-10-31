// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Reads the regenerated `tala-stages-cluster-creation` capture (docs/FIXTURES.md).
#![cfg(feature = "oracle-fixtures")]

use super::*;
use crate::internal::Pipeline;

#[test]
fn preprocessing_publishes_complete_cluster_graph_before_placement() {
    // Original Go CreateVessel/AddCluster/abductClusterEdges, four seeds.
    // Complete serialized graphs preserve ordered membership, vessel identity,
    // hidden members, container headers, geometry and both edge abductions.
    let input_bytes: &[u8] = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/input.json");
    for (seed, clusters, hubs) in [
        (
            1,
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/1-PreprocessClusters.json"),
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/1-PreprocessHubs.json"),
        ),
        (
            2,
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/2-PreprocessClusters.json"),
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/2-PreprocessHubs.json"),
        ),
        (
            7,
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/7-PreprocessClusters.json"),
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/7-PreprocessHubs.json"),
        ),
        (
            42,
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/42-PreprocessClusters.json"),
            d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/cluster-creation/42-PreprocessHubs.json"),
        ),
    ] {
        let input = decode::graph(input_bytes).unwrap();
        let (core, nodes, edges) = reconstruct::to_core_graph_with_records(&input).unwrap();
        let mut pipeline = Pipeline::new_untraced(&core, seed, false, false);
        pipeline.run_prescale();
        pipeline.run_preprocess_sequences();
        pipeline.run_preprocess();
        pipeline.run_preprocess_trees();
        pipeline.run_preprocess_containers();
        pipeline.run_preprocess_hierarchies();
        pipeline.run_preprocess_clusters();
        for (stage, expected) in [("PreprocessClusters", clusters), ("PreprocessHubs", hubs)] {
            if stage == "PreprocessHubs" {
                pipeline.run_preprocess_hubs();
            }
            let actual = serialize::graph(&pipeline.graph, &nodes, &edges, &input)
                .expect("complete owner inventory");
            let expected = decode::graph(expected).unwrap();
            assert_eq!(
                String::from_utf8(serde_json::to_vec(&actual).unwrap()).unwrap(),
                String::from_utf8(serde_json::to_vec(&expected).unwrap()).unwrap(),
                "seed {seed} {stage}"
            );
        }
    }
}
