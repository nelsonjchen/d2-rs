// SPDX-License-Identifier: MPL-2.0
//! Contracts ported from Go 9e73ae201, 9a9992812, ec0f409db,
//! d84b19844, 19acc76d7, e1922797b and 0c552c45d.
use d2_rs_core::{
    Size,
    ast::Shared,
    ir::{Node, NodeId},
};
use d2_rs_language::{
    compiler::{self, CompileLimits, CompileOptions, Compiler},
    parse,
};

fn compile(source: &str, limits: CompileLimits) -> (Compiler, NodeId) {
    let parsed = parse("main.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    compiler::compile_ir_with_limits(Shared::new(parsed.map), CompileOptions::default(), limits)
        .unwrap()
}
fn check(c: &Compiler) {
    assert!(c.errors.is_empty(), "{:?}", c.errors);
}
struct Unmeasured;
impl compiler::GraphMeasurer for Unmeasured {
    fn measure(&self, _: &str) -> Size {
        Size::default()
    }
}

#[test]
fn explicit_sql_and_class_types_matching_names() {
    let (c, root) = compile(
        "time: {shape: sql_table; id; created {constraint: primary_key}; date: date; key: key; key {constraint: unique}}\nEvent: {shape: class; id; date: date; +status: status; save(); now(): now()}\n",
        CompileLimits::default(),
    );
    check(&c);
    let (graph, _, styles) =
        compiler::lower_graph_with_measurer_and_labels(&c, root, &Unmeasured).unwrap();
    let style = |name| {
        &styles[&d2_rs_core::NodeId(
            graph
                .nodes
                .iter()
                .position(|n| n.external_id == name)
                .unwrap() as u32,
        )]
    };
    assert_eq!(
        style("time")
            .table_rows
            .iter()
            .map(|r| r.1.as_str())
            .collect::<Vec<_>>(),
        ["", "", "date", "key"]
    );
    assert_eq!(
        style("Event")
            .class_fields
            .iter()
            .map(|r| r.1.as_str())
            .collect::<Vec<_>>(),
        ["", "date", "status"]
    );
    assert_eq!(
        style("Event")
            .class_methods
            .iter()
            .map(|r| r.1.as_str())
            .collect::<Vec<_>>(),
        ["void", "now()"]
    );
}
#[test]
fn recursive_sql_relationships_before_and_after_selectors() {
    let styles = "(*** -- ***)[*].style.bold: false\n(*** -> ***)[*].style.italic: false\n(*** <- ***)[*].style.bold: false\n(*** <-> ***)[*].style.italic: false\n";
    let tables = "users: {shape: sql_table; id; name; email}\norders: {shape: sql_table; id; user_id; total}\n";
    for (source, count) in [
        (format!("{styles}{tables}"), 0),
        (format!("{styles}{tables}orders.user_id -> users.id\n"), 1),
        (format!("{tables}orders.user_id -> users.id\n{styles}"), 1),
    ] {
        let (c, root) = compile(
            &source,
            CompileLimits {
                max_edge_expansion: 4,
                ..Default::default()
            },
        );
        check(&c);
        let (graph, _, style) =
            compiler::lower_graph_with_measurer_and_labels(&c, root, &Unmeasured).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), count);
        assert!(style.values().all(|s| s.table_columns.len() == 3));
        // IR endpoints retain column paths; both arrow orientations style the edge.
        let Node::Map(m) = c.arena.node(root) else {
            unreachable!()
        };
        for edge in m.edges.iter().flatten() {
            for property in ["bold", "italic"] {
                let map = c.arena.map(*edge).unwrap();
                let field = c
                    .arena
                    .get_field(
                        map,
                        &[
                            d2_rs_core::ast::raw_string_box("style", true),
                            d2_rs_core::ast::raw_string_box(property, true),
                        ],
                    )
                    .unwrap();
                let Node::Scalar(s) = c.arena.node(c.arena.primary(field).unwrap()) else {
                    unreachable!()
                };
                assert_eq!(s.value.scalar_string(), "false");
            }
        }
    }
}
#[test]
fn recursive_selector_distinct_pairs_and_actual_work() {
    for (source, limits, message) in [
        (
            "a -> b\na -> c\n(*** -> ***)[*].style.bold: true\n",
            CompileLimits {
                max_edge_expansion: 1,
                ..Default::default()
            },
            Some("limit of 1 endpoint pairs"),
        ),
        (
            "a -> b\na -> c\n(*** -- ***)[*].style.bold: true\n",
            CompileLimits {
                max_edge_expansion: 1,
                max_edge_expansion_work: 1,
                ..Default::default()
            },
            Some("work limit of 1 endpoint-pair examinations"),
        ),
        (
            "a -> b\nb <- a\n(*** -> ***)[*].style.bold: true\n",
            CompileLimits {
                max_edge_expansion: 1,
                ..Default::default()
            },
            None,
        ),
    ] {
        let (c, _) = compile(source, limits);
        if let Some(message) = message {
            assert!(
                c.errors.iter().any(|e| e.message.contains(message)),
                "{:?}",
                c.errors
            );
        } else {
            check(&c);
        }
    }
}
#[test]
fn parallel_selector_planning_is_bounded_and_uncharged() {
    let undirected = "(*** -- ***)[*].style.bold: true\n";
    let directed = "(*** -> ***)[*].style.bold: true\n";
    for (source, limits, count) in [
        (
            format!("{undirected}{}", "a -> b\n".repeat(365)),
            CompileLimits::default(),
            365,
        ),
        (
            format!("{}{undirected}", "a -> b\n".repeat(8)),
            CompileLimits {
                max_edge_expansion_work: 4,
                max_glob_expansion: 1,
                ..Default::default()
            },
            8,
        ),
        (
            format!(
                "a.style: {{fill: red; stroke: black}}\nb.style: {{fill: blue; stroke: black}}\n{}{undirected}",
                "a -> b\n".repeat(8)
            ),
            CompileLimits {
                max_edge_expansion_work: 4,
                ..Default::default()
            },
            8,
        ),
        (
            format!("box: {{\n{}}}\n{directed}", "a -> b\n".repeat(16)),
            CompileLimits {
                max_edge_expansion_work: 32,
                ..Default::default()
            },
            16,
        ),
    ] {
        let (c, root) = compile(&source, limits);
        check(&c);
        assert_eq!(c.arena.edge_count_recursive(root), count);
    }
}
#[test]
fn recursive_styles_follow_imports_and_removal() {
    let parsed = parse("main.d2", b"first_group: @library.group\nremoved: @library.group\nremoved: null\nsecond_group: {...@library}\n(*** -> ***)[*].style.bold: true\n", None);
    let (c, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        CompileOptions {
            import_opener: Some(Box::new(|_| {
                Ok(Box::new(std::io::Cursor::new(b"group: {a -> b}".to_vec())))
            })),
            ..Default::default()
        },
    )
    .unwrap();
    check(&c);
    let graph = compiler::lower_graph(&c, root).unwrap();
    assert_eq!(graph.edges.len(), 2);
    assert!((0..graph.edges.len()).all(|i| graph.edge_label_bold(d2_rs_core::EdgeId(i as u32))));
}
#[test]
fn elk_layer_constraint_normalization_and_diagnostics() {
    for value in ["first", "FiRsT", "LAST", "none"] {
        let (c, root) = compile(
            &format!("a.elk-layer-constraint: {value}"),
            CompileLimits::default(),
        );
        check(&c);
        let graph = compiler::lower_graph(&c, root).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].elk_layer_constraint, value.to_uppercase());
    }
    for (source, message) in [
        ("elk-layer-constraint: first", "can only be set on shapes"),
        (
            "a -> b: {elk-layer-constraint: first}",
            "can only be set on shapes",
        ),
        (
            "a.elk-layer-constraint: middle",
            "expects first, last or none, got \"middle\"",
        ),
        ("a.elk-layer-constraint: {}", "expects first, last or none"),
    ] {
        let (c, root) = compile(source, CompileLimits::default());
        check(&c);
        let error = compiler::lower_graph(&c, root).unwrap_err().to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
}
