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
