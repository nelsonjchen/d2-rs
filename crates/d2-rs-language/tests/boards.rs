// SPDX-License-Identifier: MPL-2.0

use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};
use d2_rs_text::measure::{bundled_source_code_pro_regular, bundled_source_sans_pro_bold};

fn compiled(source: &str) -> (compiler::Compiler, d2_rs_core::ir::NodeId) {
    let parsed = parse("boards.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none(), "parse failed: {:?}", parsed.error);
    compiler::compile_ir(Shared::new(parsed.map)).expect("compile failed")
}

#[test]
fn resolves_nested_board_paths_without_flattening_the_root() {
    let (compiler, root) =
        compiled("root: { label: root }\nlayers: { architecture: { service: { label: api } } }\n");
    let board = compiler::resolve_board_target(&compiler, root, "layers.architecture")
        .expect("board target should resolve");
    let graph = compiler::lower_graph(&compiler, board).expect("board should lower");
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].external_id, "service");
}

#[test]
fn accepts_nested_scenario_and_step_targets() {
    let (compiler, root) = compiled(
        "layers: { architecture: { scenarios: { deployed: { steps: { done: { service: ok } } } } } }\n",
    );
    let board = compiler::resolve_board_target(
        &compiler,
        root,
        "layers.architecture.scenarios.deployed.steps.done",
    )
    .expect("nested board target should resolve");
    let graph = compiler::lower_graph(&compiler, board).expect("board should lower");
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].external_id, "service");
}

#[test]
fn enumerates_concrete_boards_in_source_order() {
    let (compiler, root) = compiled(
        "layers: {\n  one: {\n    scenarios: { deployed: { a: ok } }\n  }\n  two: { b: ok }\n}\n",
    );
    let paths = compiler::board_targets(&compiler, root)
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        ["layers.one", "layers.one.scenarios.deployed", "layers.two"]
    );
}

#[test]
fn mapless_layers_participate_in_traversal_and_target_selection() {
    // d2compiler.compileBoardsField substitutes &d2ir.Map{} when f.Map() is nil,
    // then applies the field's primary label. The raw IR remains mapless.
    let (compiler, root) = compiled("a\nlayers: { empty; caption: Title; drawn: {b} }");
    let boards = compiler::board_targets(&compiler, root);
    assert_eq!(
        boards
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["layers.empty", "layers.caption", "layers.drawn"]
    );
    for (index, name) in ["empty", "caption"].into_iter().enumerate() {
        let board = boards[index].1;
        assert!(compiler.arena.map(board).is_none());
        for path in [
            name.to_owned(),
            format!("layers.{name}"),
            format!("{name}.{name}"),
        ] {
            assert_eq!(
                compiler::resolve_board_target(&compiler, root, &path).unwrap(),
                board
            );
        }
        assert!(!compiler::board_is_renderable(&compiler, board).unwrap());
        assert!(
            compiler::lower_graph(&compiler, board)
                .unwrap()
                .nodes
                .is_empty()
        );
        assert!(compiler.arena.map(board).is_none());
    }
    assert_eq!(compiler::board_label(&compiler, boards[0].1), None);
    assert_eq!(
        compiler::board_label(&compiler, boards[1].1).as_deref(),
        Some("Title")
    );
}

#[test]
fn board_traversal_uses_original_wrapper_order_in_both_directions() {
    let (compiler, root) = compiled(
        "steps: {s: {step}}\nscenarios: {c: {scenario}}\nlayers: {l: {layer; steps: {n: {nested}}}}",
    );
    let paths = compiler::board_targets(&compiler, root)
        .into_iter()
        .map(|v| v.0)
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        ["layers.l", "layers.l.steps.n", "scenarios.c", "steps.s"]
    );
    let post = compiler::board_targets_postorder(&compiler, root)
        .into_iter()
        .map(|v| v.0)
        .collect::<Vec<_>>();
    assert_eq!(
        post,
        ["layers.l.steps.n", "layers.l", "scenarios.c", "steps.s"]
    );
}

#[test]
fn rejects_object_paths_and_missing_boards() {
    let (compiler, root) = compiled("service: ok\nlayers: { architecture: { api: ok } }\n");
    assert!(matches!(
        compiler::resolve_board_target(&compiler, root, "service"),
        Err(compiler::BoardTargetError::NotFound(_))
    ));
    assert!(matches!(
        compiler::resolve_board_target(&compiler, root, "layers.missing"),
        Err(compiler::BoardTargetError::NotFound(_))
    ));
}

#[test]
fn board_targets_accept_shorthand_mixed_wrappers_and_current_board_names() {
    let (compiler, root) =
        compiled("layers: { index: { layers: { nest1: { scenarios: { nest2: { d } } } } } }");
    let expected = compiler::resolve_board_target(
        &compiler,
        root,
        "layers.index.layers.nest1.scenarios.nest2",
    )
    .unwrap();
    for target in [
        "index.nest1.nest2",
        "layers.index.nest1.scenarios.nest2",
        "index.layers.nest1.nest2.nest2",
    ] {
        assert_eq!(
            compiler::resolve_board_target(&compiler, root, target),
            Ok(expected)
        );
    }
    for target in ["layers", "index.nest1.d", "index.missing", "nest1"] {
        assert!(
            matches!(
                compiler::resolve_board_target(&compiler, root, target),
                Err(compiler::BoardTargetError::NotFound(_))
            ),
            "{target}"
        );
    }
}

#[test]
fn default_lowering_uses_pinned_d2_font_metrics() {
    let (compiler, root) = compiled("hello\n");
    let graph = compiler::lower_graph(&compiler, root).expect("graph should lower");
    let node = graph
        .nodes
        .iter()
        .find(|node| node.external_id == "hello")
        .expect("hello node should exist");
    let bold = bundled_source_sans_pro_bold().expect("bundled Source Sans Pro should parse");
    let mono = bundled_source_code_pro_regular().expect("bundled Source Code Pro should parse");
    let raw = bold.measure_with_fallback("hello", 16.0, &mono);
    // D2's rectangle path adds five pixels of inner label padding and forty
    // pixels of shape padding after the raw ruler bounds.
    assert_eq!(node.size.width, raw.width.ceil() + 45.0);
    assert_eq!(node.size.height, raw.height.ceil() + 45.0);
    assert_ne!(node.size.width, 8.0 * 5.0 + 32.0);
}
