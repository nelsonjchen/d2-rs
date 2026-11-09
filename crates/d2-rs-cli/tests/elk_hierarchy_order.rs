// SPDX-License-Identifier: MPL-2.0
use std::io::Write;
use std::process::{Command, Stdio};

fn assert_elk_svg(source: &str, expected: &[(&str, &str)]) {
    for &(direction, digest) in expected {
        let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
            .args(["--layout=elk", "--omit-version", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        write!(
            child.stdin.take().unwrap(),
            "direction: {direction}\n{source}"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = ring::digest::digest(&ring::digest::SHA256, &output.stdout)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual, digest, "{direction}: {source}");
    }
}

#[test]
fn elk_nested_crossings_keep_saved_long_edge_targets() {
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    // elk-go v0.2.0 intermediate/sort_by_input_model.go LongEdgeTargetNode
    // checks the helper's saved target before walking its outgoing edges.
    assert_elk_svg(
        "b.n3 -> a.c.a.n17\nb.a.b.n13 -> c.n2\na.b.n40 -> b.n3: e24\n\
         a.c.a.n17 -> b.a.n38\nc.a.n44 -> a.b.a.n5\nn28 -> a.b.a.n21\n",
        &[
            (
                "down",
                "1eb90ad32eb6a92eb3a60a14af4dce275b99c70becdc895c19f70bf09c3b3c54",
            ),
            (
                "up",
                "1b7bff058ec8907ecee9067527014f222ef085b7389f89c561addefe05b51cbe",
            ),
            (
                "right",
                "64c1655f7a7e2e9bb8f9461a5872de7d49d3ba86166b55acbb1346ba440f7df0",
            ),
            (
                "left",
                "acdfb85c8ad2ac9c633edca3537b25618cb040e433489fcad5ca406833b23b9b",
            ),
        ],
    );
}

#[test]
fn elk_parallel_labels_and_hidden_loop_keep_bottom_up_sweep() {
    // Pinned elk-go p3order/layer_sweep_type_decider.go UseBottomUp walks
    // NodeOutgoingEdges: the self-loop's detached ports do not contribute
    // influence. Go counts 10 random paths and eight hierarchical paths.
    assert_elk_svg(
        "s: {o -> p: charge; o -> p: refund; o -> p: void; p -> p: retry}\n\
         g -> s.o\ng -> s.a\n",
        &[
            (
                "down",
                "9856fcf10d792e3a60a6df68d66689a413dbcf451d99e206e8a5513a773cff47",
            ),
            (
                "up",
                "db506f93ee8c6d7ad8e73d7258b63d7eb899608979110bc262c329a98e659ff1",
            ),
            (
                "right",
                "8ab4f18dca987d843028fda14c92e887aac169ef6e0397335c80b110920ac5a2",
            ),
            (
                "left",
                "b0d9cc79fc56b2d98f094abf73e32ee6d2af775d485665201ef2768227d8b8cd",
            ),
        ],
    );
}

#[test]
fn elk_hidden_loops_do_not_prevent_boundary_layer_constraints() {
    // Pinned elk-go intermediate/layer_constraint_preprocessor.go tests the
    // attached ports after hiding boundary edges. A detached self-loop does
    // not stop the adjacent node inheriting FIRST/LAST from that boundary.
    assert_elk_svg(
        "n0: {n1; n2}\nn0 -> n0\nn0 -> n0.n1\nn0 -> n0.n2\nn0.n2 -> n0.n2\n",
        &[
            (
                "down",
                "34bf0638a0a862c380b9213c1e85545462a4c78d80a10aeb655739a163daaa19",
            ),
            (
                "right",
                "51e1a052b6ee45a915a27744708bc81de36335c19ceb2a10345d16d84d61af6b",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1; n2}\nn3\nn0.n2 -> n3: e0\nn0 -> n3: e1\n\
         n0.n2 -> n0.n2: e2\nn3 -> n0.n1: e3\n",
        &[
            (
                "down",
                "d5fa4ed2df22fdd43ceef7bd06ad2a7d8e04186994480169f6a0debb08494fb8",
            ),
            (
                "right",
                "84d8d8cfef7bb8627a9bb7f2cff174aaacad4ec2966f8cc872c333e1d2d71840",
            ),
        ],
    );
}

#[test]
fn elk_compound_loop_keeps_detached_hierarchy_port_in_routing_order() {
    // Pinned elk-go selfloop_preprocessor.go keeps hierarchical ports attached;
    // loops/ordering/port_restorer.go preserves them when PortsHidden is false.
    // RoutingDirector includes the temporarily unconnected port in its penalty.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1}\nn0 -> n0\nn0.n1 -> n0\n",
        &[
            (
                "down",
                "f255163bafbd7d53915a02530221b90971569eddbdf0316eaed728a9e99f76dd",
            ),
            (
                "up",
                "f08a8b1894ff3b3bf7c6a4d76d447f2360ccdfdec60597ace4524c81e8948d13",
            ),
            (
                "right",
                "0e4104a87d2c584d3d3154e5ae01c56e1e3d72b4cc893ae7a1641e1adfab59da",
            ),
            (
                "left",
                "7f46ef5b5d0b10aac8baeb0ded6448071fb5e2e87fa6603f253460bbe1c8317b",
            ),
        ],
    );
}

#[test]
fn elk_compound_loop_preserves_all_transferred_port_positions() {
    // Pinned elk-go hierarchical_ports.go transferHierarchyGraphToNode copies
    // every external dummy, including self-loop ports. label_and_node_size.go
    // ProcessLabelAndNodeSizes preserves their fixed along-side coordinates.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0\nn1: {n2}\nn0 -> n1\nn1 -> n1\nn1 -> n1.n2\n",
        &[
            (
                "down",
                "686d56a7b2c546059435471f4560261cb81abfa72877a1181b608fdcea8da1cf",
            ),
            (
                "up",
                "9c402c7028fe6b12fa6238c241413a616b96f80ec89637482c36feca95551888",
            ),
            (
                "right",
                "b40952945fe7c437917c45416ad1625b16fb0f17b440ed4b42895e570b9b58f1",
            ),
            (
                "left",
                "668542354bcc500803b303cfb94e56fff3f600fa5e86e904039edd6426cee8be",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n2}}\nn0.n1 -> n0.n1\nn0.n1.n2 -> n0.n1\n",
        &[
            (
                "down",
                "1a63278a812eb3a1be4f4a26bf4e651a440d95447a5d92aeb50635b86dba2dba",
            ),
            (
                "up",
                "5f464743af8611dda14651b83575ed72a5b626180de6a3decb0408fff338a647",
            ),
            (
                "right",
                "ad67734c13aa77a4f2ed7be03961e13365f747317b004447d9d05236f3adc656",
            ),
            (
                "left",
                "7300ec5058099fcd2d4056c268c300b58f6e9410af31d697f2dce85c1c44b3dd",
            ),
        ],
    );
}

#[test]
fn elk_long_edge_dummies_follow_compound_port_adjacency() {
    // Pinned elk-go intermediate/long_edge.go SplitLongEdges walks node.Ports
    // and each outgoing edge slice. Compound splitting and cycle reversal
    // leave their order different from the edge arena insertion order.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1; n2: {n3}}\nn4\nn4 -> n0.n2.n3\nn0 -> n0.n2\n\
         n0.n2.n3 -> n4\nn0.n2 -> n0.n1\n",
        &[
            (
                "down",
                "4529b44b72636cc90b1eb5cb171850fddb90652e158d040a5d62b588e2f71a8a",
            ),
            (
                "up",
                "a6d1ce5bb9322b39f80a681d94d287c4415997f87ca14aba5fb98a22e083d39b",
            ),
            (
                "right",
                "4ea674d914a55335ec59bd3f92a48c666b397a28b24f9d700159bb21071d198c",
            ),
            (
                "left",
                "1e0c02a25b5cee0dfecc4da8f21ce5aafc478ea3586ad50a36bda10a04017efc",
            ),
        ],
    );
}

#[test]
fn elk_nested_loop_bends_keep_node_local_rounding_before_padding() {
    // Pinned elk-go PostprocessSelfLoops translates node-local bends before
    // Transferrer.edgeLayoutOffsets adds Graph.Offset+Padding. Keeping that
    // order preserves the residual that cancels a fractional graph offset.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1; n2}\nn0.n1 -> n0\nn0.n1 -> n0.n1\n\
         n0.n1 -> n0.n2\nn0.n2 -> n0.n1\n",
        &[
            (
                "down",
                "74c8e1188a878c8ac5d77bc2e086b4ec179e84678e4fcea181025858a2e5fcb6",
            ),
            (
                "up",
                "a4ff2bec5c076d1af1f178b281a6c50b379bdf5b6f071660e5751a26537499f2",
            ),
            (
                "right",
                "0fc56f9cfe1022d06478cabe7601d96eae1ee282ef236fea40b372bf8008f1d0",
            ),
            (
                "left",
                "e2c6d1a09044bdaa5da084fb031dab64d80386b72f5358f95aea709e4ede00b5",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1; n2}\nn0.n1 -> n0\nn0.n1 -> n0.n2\n\
         n0.n2 -> n0\nn0.n2 -> n0.n2\n",
        &[
            (
                "down",
                "c9633ef10728987b85bb5ce799ebed211a52c3d21cd48a1c0a7670bb079ecdbc",
            ),
            (
                "up",
                "a713d81fa2d9bfd3cc23119888ce59cc6effca6622cbe68d5b0538280ecf70ac",
            ),
            (
                "right",
                "e13e59727876b77b28a13316f09c4eea70ef8d82f029c124620b29a4f241093e",
            ),
            (
                "left",
                "2817c9a77ba0e432ff03451d151ce663fdbfb4f87f009807be161d32ea351eec",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n2}}\nn0 -> n0.n1\nn0.n1 -> n0\nn0.n1 -> n0.n1\n",
        &[
            (
                "down",
                "d178acaa2c96de89d1a209000144d29faffdc67c65d28946dc7750aca0c1ba58",
            ),
            (
                "up",
                "f818041ed1a439506cb2fee6e5a9e22e0f015683b17621cbb308f6491583283d",
            ),
            (
                "right",
                "f94e25a8e0c649cdf9b92dfd36d02baa0b3afa59350998a14053a556ab40d1c3",
            ),
            (
                "left",
                "eac594569796764d30aa772040a57af882c89c7de3dfb81983dbb227d562c745",
            ),
        ],
    );
}

#[test]
fn elk_compound_bounds_add_padding_after_measuring_content() {
    // Complete SVGs from pristine pinned D2 6a7b48378, elk-go v0.2.0.
    // ComponentsCompactor measures the unpadded hull; HierarchicalNodeResizer
    // uses GraphActualSize before publishing translated node positions.
    assert_elk_svg(
        "n0: {n1; n2; n3}\nn0.n1 -> n0.n2\nn0.n3 -> n0.n1\nn0.n3 -> n0.n2\n",
        &[
            (
                "down",
                "a0e8b9986f2a0d68b6669c0e69155aebec139eba4e54a44dbf9ac9e9131074f3",
            ),
            (
                "up",
                "0eb2d1fd5f3e31dbc9dc0c27da9e509ce05b0c7699f8584860a5a71ad6c31bb0",
            ),
            (
                "right",
                "1a6c46da30cf437f7cc1ccee15175a65221cf6f03ec62020c8d0bdb4a3a06ec0",
            ),
            (
                "left",
                "cd1c0394e940b2dd406076377101b5a5ee8a08c0e527b43ae26b5338df0d9d88",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1; n2; n3}\nn0.n1 -> n0.n2\nn0.n3 -> n0.n1\nn0.n3 -> n0.n2\nn0 -> n0\n",
        &[
            (
                "down",
                "3d2de74dfefc33be2ae16e0b2e9beeba6e2dc83f88a3c2a55e0f74e7051cd776",
            ),
            (
                "up",
                "1ed3799d077eef3fa7acbeb09a19b5176b688f6fa30132ff2e4022319a7a8a8d",
            ),
            (
                "right",
                "eced95f67e91685129ec23197aa65e88a1b4c760d1fd60ecf5fc8f641f149d30",
            ),
            (
                "left",
                "c1bc32264b0ff9d3350fa588a2a6f1872fa3a8df1cbaf9c0648b5ea7d1187044",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\nn0: {n1; n2; n3}\nn0.n1 -> n0.n2\nn0.n3 -> n0.n1\nn0.n3 -> n0.n2\n}\n",
        &[
            (
                "down",
                "3f3c5fa30a880525da695e0db2d4673b29d7004188b5188dd1d7380d923228f2",
            ),
            (
                "up",
                "b7b0cc4e52b20ce1bfaf5074f11b36c346ef3ec3b818588ed60dcf6c4aa3e6bd",
            ),
            (
                "right",
                "3f21ad77d2bce8abc30706c9a76a4edb257a255a89f0c0f678cb4d30bd519a79",
            ),
            (
                "left",
                "9ddd4e9a17f7f1225e1f76486a7c596412201fadb5bfe88c322e2d7165bde3fd",
            ),
        ],
    );
}

#[test]
fn elk_compound_loop_dummies_keep_imported_port_order() {
    // Pinned elk-go compound.go ensureCompoundPortDummies visits imported
    // node ports before SelfLoopPortRestorer changes their perimeter order.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n1: {n2}\nn1 -> n1: e0\nn1 -> n1: e1\nn1.n2 -> n0: e2\n",
        &[
            (
                "down",
                "1be68a41b9850c2310ec4a1353cb208bfc65e83aaf68dab87175d633b0d2a17a",
            ),
            (
                "up",
                "495a8a533b8653779d654d75d59351306142f6bab96cad02d64f86f1938a552b",
            ),
            (
                "right",
                "b8585b0155fdfb8f1c57b490ca2b0fbd476bc7ae7abe98d7c0bb74ceb71f2f17",
            ),
            (
                "left",
                "f295d2b22e9cc42dd9f496189f46645caaaba6e4b6bfa301e76f7beb5501cf1b",
            ),
        ],
    );
    assert_elk_svg(
        "n1: {n2}\nn1 -> n1\nn1 -> n1\nn1.n2 -> n0\n",
        &[
            (
                "down",
                "07ceb4b9e3d3669a9ba41e93909caaa0d2d1830f8e16ba7bb03a1c414dfd3105",
            ),
            (
                "up",
                "299cb9555824a1c1bf34b2e31bccb4eef8c86d2c2257a8c8e8a27aad4319f685",
            ),
            (
                "right",
                "faef4b34ada0611ba1e92e8ece392018dae74b29d2470c3e409614a50e64aa9d",
            ),
            (
                "left",
                "ef593cd5d030acacf801aa03aced1c76673175e04d798b09df04a0b3dfe37042",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\n n1: {n2}\n n1 -> n1: retry\n n1 -> n1: fail\n n1 -> n1: recover\n n0 -> n1.n2: request\n}\n",
        &[
            (
                "down",
                "b3d9fc6e50d92bac23cdb4a0f69e011420c67cdd167b7244b4450a19206e55b2",
            ),
            (
                "up",
                "cf39c6c54569580f86b1f18f44827f960a5509461fa39c72dc47c7b083212982",
            ),
            (
                "right",
                "54b9119b534495d0dbc0b57030f5ae8dc8fb0a5cebd3fc47b51144040558d144",
            ),
            (
                "left",
                "a656e2cdae78576fa5da9ce4c04f83189d36bb11ead6a74b96ba4406a3579100",
            ),
        ],
    );
}

#[test]
fn elk_crossing_counts_keep_initial_hierarchy_sweep_membership() {
    // Pinned elk-go p3order/graph_info_holder.go freezes UseBottomUp during
    // holder initialization. countCurrentNumberOfCrossings includes the same
    // child scopes even after in-layer dependencies have changed node order.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1; n2: {n4}; n3}\nn0 -> n0.n2.n4: e0\nn0 -> n0.n2.n4: e1\nn0.n2 -> n0.n3: e2\nn0.n3 -> n0.n2: e3\n",
        &[
            (
                "down",
                "758c7eb7b2391b14d0e8f845808a1b2f44aac208cf9d94c0c7865904ce14d6ae",
            ),
            (
                "up",
                "58bff4a79e50b7bdbdf82b9252ba52c98af442e045cdf36b6d02d01de0c99960",
            ),
            (
                "right",
                "3e83e6298b2db504191f15070c3dced18072f04f96d54369dc3c6b03eedf3e59",
            ),
            (
                "left",
                "a59fd8670bcd3ecbcc5ecf34a89bf880f142352b192b13960bbbbeb1915ecbdd",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1; n2: {n4}; n3}\nn0 -> n0.n2.n4: request\nn0 -> n0.n2.n4: retry\nn0.n2 -> n0.n3: e2\nn0.n3 -> n0.n2: e3\n",
        &[
            (
                "down",
                "e9236cceeeeb884d8fc9721ff3c352d35ea65cb76df16808268cd4c519767595",
            ),
            (
                "up",
                "1fa9cd59c722631360edc63427a08492f6b5bf174c349f7d1b12d888b5ce740d",
            ),
            (
                "right",
                "06f3ff610e494aac9aaa573f4f1f5c3ac5375d01564c6a0e21fe27f1fa109195",
            ),
            (
                "left",
                "84cf222d65f36329ebb66292d7ad0c8bb51c9b15c4ba94db6d2b4c8fa426d579",
            ),
        ],
    );
}

#[test]
fn elk_ancestor_routes_use_transformed_hierarchy_offsets() {
    // Pinned elk-go compound.go changes each segment into the ancestor's
    // reference graph after GraphTransformer; Transferrer.applyEdge removes
    // the base offset from ancestor-source anchors before edgeLayoutOffsets.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "outer: {\nn0: {n1; n2: {n4}; n3}\nn0 -> n0.n2.n4: e0\nn0 -> n0.n2.n4: e1\nn0.n2 -> n0.n3: e2\nn0.n3 -> n0.n2: e3\n}\n",
        &[
            (
                "down",
                "798f1b36c2627e8ede7ed930e8c41fc8fecb470b2504495ce62b86ab53eeec2a",
            ),
            (
                "up",
                "84452efc90ba927d56fd11947b3aa2e4237e3f75dda1e8f073d6193b5338ac7c",
            ),
            (
                "right",
                "7a0692049c8347fddda22baa094b3e097e61343e2dfa568694d7442447f76b44",
            ),
            (
                "left",
                "2f0e51c52b8d71f34dc25b814a6ec9a6a248f5c84cc533db2b98647774477b84",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {inner: {\nn0: {n1; n2: {n4}; n3}\nn0 -> n0.n2.n4: e0\nn0 -> n0.n2.n4: e1\nn0.n2 -> n0.n3: e2\nn0.n3 -> n0.n2: e3\n}}\n",
        &[
            (
                "down",
                "8c82217d509b20fc03f79caf2173124c055b43320d3eab6e568f37c639893907",
            ),
            (
                "up",
                "9d52a1888a37277d5928347313fbffd635e1627864f39df35ffb1621302d6825",
            ),
            (
                "right",
                "ab1860bf6a76a09e0f5aa6980bdddd098de2c8021d757eb907b3b60ca8a41139",
            ),
            (
                "left",
                "7091d29f996160f6617ef8d5bddf7113ee0f7a3589075ca76e33f84db5fd85cc",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\nn0: {n1; n2: {n4}; n3}\nn0.n2.n4 -> n0: e0\nn0.n2.n4 -> n0: e1\nn0.n2 -> n0.n3: e2\nn0.n3 -> n0.n2: e3\n}\n",
        &[
            (
                "down",
                "d45c972a1dbb81910ae65ccf98cc16401f310d93204849a10cb7e4a395472b02",
            ),
            (
                "up",
                "fd3a7f4f402792b0886a7f7846079ebf4d399f1918c379068ec350689110a483",
            ),
            (
                "right",
                "cecb6832fa56b0652da4c0bff47a88d14776767409a436d614b359ea625cb519",
            ),
            (
                "left",
                "ba9324592707a27ebfc33546ec64f3fef0dd3952b1a7be15d2aa9e25e0854870",
            ),
        ],
    );
}

#[test]
fn elk_ancestor_routes_keep_fractional_source_frame_arithmetic() {
    // Transferrer.applyEdge sums node, port and anchor before edgeLayoutOffsets.
    // Ancestor edges use their contained graph as CoordinateSystemOrigin;
    // reflecting an already joined route changes fractional anchors and labels.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1: {n3; n4}; n2}\nn0.n2 -> n0.n1.n4: e0\nn0.n2 -> n0.n1.n3: e1\nn0.n1.n3 -> n0: e2\nn0.n2 -> n0.n1.n3: e3\n",
        &[
            (
                "down",
                "0f640a4990c6a7544b604577725d866c058777884af154dee9edcbb309fb7ecc",
            ),
            (
                "up",
                "78b4f400c771485e2eaa27c04f17bb333d08443f011b4dbe1f34e40a5325256e",
            ),
            (
                "right",
                "2beb03ec1b6f5baf0e1e1e9700521ec7e015253ad2aafd53f4f972df9320556c",
            ),
            (
                "left",
                "17b6196d54e233a82edd0bafa316fe5b4b1096c8b4316ffdbc7c9967f4702195",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n3; n4}; n2}\nn0.n1.n3 -> n0.n2: e0\nn0 -> n0.n1.n3: e1\nn0 -> n0.n1.n3: e2\nn0.n1 -> n0.n2: e3\n",
        &[
            (
                "down",
                "258267ee642e9ed60819c41841b1420f8ea958ca8a99f70959f9b9b797f87e4a",
            ),
            (
                "up",
                "e2e10cee981544437bc22fa0d6869b4665fd1e1ae92b04c9d1fe29aa09f0be1a",
            ),
            (
                "right",
                "32ce37ad38522aef3e3e67eb37329e13b5257d5f5a98cfe0ca59c70671de2d81",
            ),
            (
                "left",
                "63baf82ba1300ab75b2008493f1d98509fa37d0ce961c60866c997071c90ce96",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n2; n4}; n3}\nn0.n1.n2 -> n0: e0\nn0.n3 -> n0.n1.n2: e1\nn0.n3 -> n0.n1.n2: e2\nn0.n3 -> n0.n1: e3\n",
        &[
            (
                "down",
                "abb8b407d705bd23c2b9d6d66fe1315524b3116d0df12ce5b72f9eb62aac6852",
            ),
            (
                "up",
                "7d10cde05cf6ae092005388b6942a472c31fa5b920d386e8cb446a679c77ff26",
            ),
            (
                "right",
                "00924507cee2538dd2f7c9660784cc60463d15d9da9109db7f03a5d762769c1a",
            ),
            (
                "left",
                "06a00037546f47cdf3e3987de076b48da68035d1f7d7d39b7972047e104690eb",
            ),
        ],
    );
}

#[test]
fn elk_root_loop_slots_and_labels_keep_node_local_arithmetic() {
    // Pinned elk-go orthogonal_router.go computeRoutingSlotPositions and
    // routes bends before selfloop_postprocessor.go adds node.Position.
    // Complete SVGs from pinned D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1}\nn2: {n3}\nn4\nn0 -> n2.n3: e0\nn2 -> n2: e1\nn4 -> n2.n3: e2\nn2 -> n2: e3\n",
        &[
            (
                "down",
                "d834d8051aa6bb717b79423f2f9a1afdefbb16a963d2aa44f468000b06242d01",
            ),
            (
                "up",
                "388d511eabb98d2650f99d8b85b1112cfedf558ce6ec7be8e10658461566dab3",
            ),
            (
                "right",
                "b31a06f6d603289139615549f0fc1839742bd58f5813516e750ddd999116553d",
            ),
            (
                "left",
                "be422ea06de2ef7c7438cce4fd5281746c37a867f20f360ef3384b0ea5d01999",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1}\nn2: {n3}\nn4\nn0 -> n2.n3: request\nn2 -> n2: retry\nn4 -> n2.n3: response\nn2 -> n2: recover\n",
        &[
            (
                "down",
                "0410443514505a126376869f0cc789bd7d5547a9e628560b79bdb85ee69d8b4b",
            ),
            (
                "up",
                "f3df9ad094d5e1bd8d4224d1db80a16ecfa81b009615b78b1a0ec73bf4a2ace4",
            ),
            (
                "right",
                "0f7e1ae65dc916b01daee24c4e17770bf2f3d0974cd1c9297a11181992367d5f",
            ),
            (
                "left",
                "3db14c5bc98ab02d726a660e60fc2688e6c405d7a11d30ec72c183497e609e69",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\n  n0: {n1}\n  n2: {n3}\n  n4\n  n0 -> n2.n3: e0\n  n2 -> n2: e1\n  n4 -> n2.n3: e2\n  n2 -> n2: e3\n}\n",
        &[
            (
                "down",
                "05b2b42a069665a27986f31eabd273bb19ecef8ce3ba27c3071efc601ff7820e",
            ),
            (
                "up",
                "bfe5454f422b9459fdf2e95bed0183a8092edc0c0e0c631eb24d43082726d572",
            ),
            (
                "right",
                "8026b131ff121aa94a6b3f1d7e4a50e13e50b16d0924f578b8b88e847e239347",
            ),
            (
                "left",
                "3a6b18f120926f60c1d04d19bca01716e3cdd279d23f9005a4295d1d96dde6f9",
            ),
        ],
    );
}

#[test]
fn elk_hierarchy_routing_continues_the_ordering_random_stream() {
    // Complete SVGs from pristine pinned D2 6a7b48378, elk-go v0.2.0.
    // p3order GraphInfoHolder retains the graph Random property; the P5
    // orthogonal generator continues it when splitting critical cycles.
    assert_elk_svg(
        "n0: {n4}\nn1\nn2\nn3\nn1 -> n2\nn1 -> n3\nn0.n4 -> n2\nn0.n4 -> n3\n",
        &[
            (
                "down",
                "2f7b0ab8d636593465f43f39c20b89767064ca33db109fffd383251772d16043",
            ),
            (
                "up",
                "d21fc152a19be91bd57dedec3f357d87bd6c0a37b0ef825234abf58ca2f41192",
            ),
            (
                "right",
                "ed83cccf8ae4c50faea058cbaa44b787f9e3d460edef737a43723f2967cd4c76",
            ),
            (
                "left",
                "7a9738737d1a45fa5ae5154c4d997345fc5460a78a1cd642d746d2978ae34038",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n4}\nn1\nn2\nn3\nn1 -> n2\nn1 -> n0.n4\nn3 -> n2\nn3 -> n0.n4\n",
        &[
            (
                "down",
                "a0e64a6a18ac589ceef6d4c5e531ad897bfe7cecaf38a4b46a4d1e2eac95e6f9",
            ),
            (
                "up",
                "5b2916c1aad1785fa043b1d9af4623235ea695fef3efec93ae52c15868d74446",
            ),
            (
                "right",
                "e8e5ad05d821fca26c618d286679787070e3b39ed16e4fd4c4a2da83695705f2",
            ),
            (
                "left",
                "372e4b8688b56b8ec34eb136c75434ad3f750aacb8e9ad13d1621fdf16719ba8",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\nn0: {n4}\nn1\nn2\nn3\nn1 -> n2\nn1 -> n3\nn0.n4 -> n2\nn0.n4 -> n3\n}\n",
        &[
            (
                "down",
                "e59515fea3a9c22fda70a99dfcd8c09504b633427374d6181c5ea0c9627044cc",
            ),
            (
                "up",
                "cdea8a35f7c8743827b5a5a60321674218c55b65b14c88403d88ad9d1a4330c5",
            ),
            (
                "right",
                "9c0fe9af46d389b832b1fdcfff73e1504aabb299276e1c6230be84982ccf242b",
            ),
            (
                "left",
                "45c456ea964691038dd400d8682d55f65128df723228427b9bb70f2ecf2808b7",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n4: worker with a longer label}\nn1\nn2\nn3\nn1 -> n2\nn1 -> n3\nn0.n4 -> n2\nn0.n4 -> n3\n",
        &[
            (
                "down",
                "f47efc0469862eeb8dbed8b6cd0f5a2144ae196e9f1cc6acab1049c2d0f0f74e",
            ),
            (
                "up",
                "aaec5c6dbf507e8ecb2d176fdaf90c9f5f3d29a8bf0dc52ad66eb0e74c43bc96",
            ),
            (
                "right",
                "e14f6b155f4727094214dac64df86a4b5bfd3f0891f8d6795e8fa761fea23dd4",
            ),
            (
                "left",
                "2d3905466f0d3b50e142f4b60a800749a7eb6cd6b388f220e2c365466e2aa3c5",
            ),
        ],
    );
}

#[test]
fn elk_compound_content_bounds_preserve_negative_coordinate_extents() {
    // Pinned elk-go v0.2.0 components/components_compactor.go starts the hull's
    // bottomRight at negative infinity. P4 stores normalization in Graph.Offset;
    // zero is not an extra hull point when all routed geometry is negative.
    // Complete SVGs from pristine D2 6a7b48378, --layout=elk --omit-version.
    assert_elk_svg(
        "n0: {n1: {n2}; n3; n4}\nn0.n1 -> n0.n3\nn0.n1.n2 -> n0.n3\nn0.n1.n2 -> n0.n4\nn0.n3 -> n0.n1\n",
        &[
            (
                "down",
                "b31b3431ebf12a17b84660f69fdea5ac11503d902fabd0a49ef1c98cb2c99c69",
            ),
            (
                "up",
                "d6514676949068f413043536c34529188cfcc7e6c4c34e2302566de5bdd3bb93",
            ),
            (
                "right",
                "abd6e517e22e8495c7e993d3b1d1d142acfb8a872fc598fd78806e329e0730df",
            ),
            (
                "left",
                "aff74de036208818a0cbf1ddefceb974f59562fc48080609435b3cd35d6de0ac",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n2}; n3; n4}\nn0.n1 -> n0.n3\nn0.n1.n2 -> n0.n3\nn0.n1.n2 -> n0.n4\nn0.n3 -> n0.n1.n2\n",
        &[
            (
                "down",
                "6cd141e360f903e252e8163e44b9c65928b61103d4e8fec774fb48e7867fb0cd",
            ),
            (
                "up",
                "9879d6d9b59d36d9992d53fab4e55425c07a5c88ac40ac994df2ca21caa259ce",
            ),
            (
                "right",
                "7b2b222387c2623d7107ed2610206e34ed8c4b85435a9cc432ab5aa229ae517e",
            ),
            (
                "left",
                "1e456c27f2b294d64d72879ee422d3f0dcf3e839fe9806a4cf548b7439833538",
            ),
        ],
    );
    assert_elk_svg(
        "outer: {\nn0: {n1: {n2}; n3; n4}\nn0.n1 -> n0.n3\nn0.n1.n2 -> n0.n3\nn0.n1.n2 -> n0.n4\nn0.n3 -> n0.n1\n}\n",
        &[
            (
                "down",
                "c9c0f3dec6e13715ce1caa75271ab7ef9282735cdf2a101fc9aad34e3ee0aff6",
            ),
            (
                "up",
                "39cdb0b63903ea961c804e964eacae04cd8953286e80d30f2115623fbce211fe",
            ),
            (
                "right",
                "7c44d90d2bc42d75a870a5211cb550cc94de7ca508f04507fbc1d375f73a0fe8",
            ),
            (
                "left",
                "b7c90f93b831de68cc0acd3d1a1a3f30b459b93f6b501a866adf5e56714cf557",
            ),
        ],
    );
    assert_elk_svg(
        "n0: {n1: {n2: a longer label}; n3; n4}\nn0.n1 -> n0.n3\nn0.n1.n2 -> n0.n3\nn0.n1.n2 -> n0.n4\nn0.n3 -> n0.n1\n",
        &[
            (
                "down",
                "8f759b8ef313d173d6399603879d136a752b3a5f17bba65269797e319ef27ca6",
            ),
            (
                "up",
                "0ea1af90d05f74b4ff840289bbc6c2dfd5bb2b005edef61a9f93afc6382dcea2",
            ),
            (
                "right",
                "a5cf6f252c6b825c92d7dedb01b4b2717852367a0b120bf9b6ac7453a21ed964",
            ),
            (
                "left",
                "9350726923b4c663706b3352627448594b0b0399fee0196f7b80b73aa22ec76d",
            ),
        ],
    );
}
