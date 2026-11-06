// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use d2_rs_core::{Size, ast::Shared};
use d2_rs_language::{ParseOptions, compiler, parse};

struct ProbeMeasurer;

impl compiler::GraphMeasurer for ProbeMeasurer {
    fn measure(&self, _label: &str) -> Size {
        Size {
            width: 10.0,
            height: 10.0,
        }
    }

    fn measure_latex_at_size(&self, _label: &str, _font_size: u32) -> Size {
        Size {
            width: 77.0,
            height: 31.0,
        }
    }

    fn measure_latex_label_at_size(&self, _label: &str, _font_size: u32) -> Size {
        Size {
            width: 17.0,
            height: 11.0,
        }
    }
}

struct SqlTableProbeMeasurer;

impl compiler::GraphMeasurer for SqlTableProbeMeasurer {
    fn measure(&self, _label: &str) -> Size {
        Size {
            width: 10.0,
            height: 10.0,
        }
    }

    fn measure_styled_label_at_size(
        &self,
        label: &str,
        _font_size: u32,
        _bold: bool,
        _italic: bool,
    ) -> Size {
        Size {
            width: (label.chars().count() * 11) as f64,
            height: 10.0,
        }
    }
}

#[test]
fn inherited_label_and_icon_near_use_language_border_center_positions() {
    let parsed = parse(
        "border-positions.d2",
        br#"classes: {
  left: {
    label.near: border-left-center
    icon.near: border-right-center
  }
  right: {
    label.near: border-right-center
    icon.near: border-left-center
  }
}
a: {class: left; icon: https://example.com/icon.svg}
b: {class: right; icon: https://example.com/icon.svg}
"#,
        None,
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    let graph = compiler::lower_graph(&compiler, root).unwrap();
    let a = graph
        .nodes()
        .find(|(_, node)| node.external_id == "a")
        .unwrap()
        .1;
    let b = graph
        .nodes()
        .find(|(_, node)| node.external_id == "b")
        .unwrap()
        .1;
    assert_eq!(
        a.label_position,
        d2_rs_core::LabelPosition::BorderLeftMiddle
    );
    assert_eq!(
        a.icon_position,
        Some(d2_rs_core::LabelPosition::BorderRightMiddle)
    );
    assert_eq!(
        b.label_position,
        d2_rs_core::LabelPosition::BorderRightMiddle
    );
    assert_eq!(
        b.icon_position,
        Some(d2_rs_core::LabelPosition::BorderLeftMiddle)
    );
}

#[test]
fn lowering_preserves_latex_language_and_uses_math_measurement_hooks() {
    let parsed = parse(
        "-",
        br#"x: {
  label: E=mc^2
  language: tex
}"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, labels, styles) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer).unwrap();
    let node_id = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "x").then_some(id))
        .expect("x node");
    assert_eq!(styles[&node_id].language.as_deref(), Some("latex"));
    assert_eq!(labels[&node_id], "E=mc^2");
    assert_eq!(
        graph.node(node_id).unwrap().size,
        Size {
            width: 77.0,
            height: 31.0
        }
    );
    assert_eq!(
        graph.node(node_id).unwrap().label_size,
        Some(Size {
            width: 17.0,
            height: 11.0
        })
    );
}

#[test]
fn lowering_drops_unresolved_plain_object_links_but_keeps_remote_links() {
    let parsed = parse(
        "object-links.d2",
        br#"plain: { link: example.com; tooltip: hint }
remote: { link: https://example.com/path }
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, _, styles) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer).unwrap();
    let style_for = |name: &str| {
        let id = graph
            .nodes()
            .find_map(|(id, node)| (node.external_id == name).then_some(id))
            .expect("fixture node");
        &styles[&id]
    };
    assert_eq!(style_for("plain").link, None);
    assert_eq!(style_for("plain").tooltip.as_deref(), Some("hint"));
    assert_eq!(
        style_for("remote").link.as_deref(),
        Some("https://example.com/path")
    );
}

#[test]
fn paired_board_link_and_tooltip_reserve_hover_width_after_link_validation() {
    struct FittedProbe;
    impl compiler::GraphMeasurer for FittedProbe {
        fn measure(&self, _: &str) -> Size {
            Size {
                width: 10.0,
                height: 10.0,
            }
        }
        fn supports_shape_fitting(&self) -> bool {
            true
        }
    }
    // run59's pdf/pptx-paged-{file,stdout} Architecture failures: Go gives
    // the Detail node 64 extra pixels before layout, even for internal links.
    let parsed = parse(
        "paired-board-link.d2",
        br#"a: {link: layers.detail; tooltip: Open detail}
fixed: {link: layers.detail; tooltip: Open detail; width: 123}
unresolved: {link: missing; tooltip: Open detail}
single: {link: layers.detail}
classes.paired: {link: layers.detail; tooltip: Open detail}
inherited: {class: paired}
layers: {detail: {b}}
"#,
        None,
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    let (graph, _, styles) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &FittedProbe).unwrap();
    let node = |name: &str| {
        graph
            .nodes()
            .find(|(_, node)| node.external_id == name)
            .unwrap()
    };
    assert_eq!(node("a").1.size.width, 119.0);
    assert_eq!(node("inherited").1.size.width, 119.0);
    assert_eq!(node("fixed").1.size.width, 123.0);
    assert_eq!(node("unresolved").1.size.width, 55.0);
    assert_eq!(node("single").1.size.width, 55.0);
    assert!(styles[&node("a").0].link.is_some());
    assert!(styles[&node("unresolved").0].link.is_none());
}

#[test]
fn lowering_attaches_edges_to_sql_table_columns() {
    let parsed = parse(
        "-",
        br#"users: {
  shape: sql_table
  id: int
  name: string
}

orders: {
  shape: sql_table
  id: int
  user_id: int
}
orders.user_id -> users.id
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, _, styles) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &SqlTableProbeMeasurer)
            .unwrap();
    let users = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "users").then_some(id))
        .expect("users table");
    let orders = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "orders").then_some(id))
        .expect("orders table");
    assert_eq!(styles[&users].table_max_name_width, 44);
    assert_eq!(styles[&orders].table_max_name_width, 77);
    let (edge_id, edge) = graph.edges().next().expect("foreign-key edge");
    assert_eq!(edge.source, orders);
    assert_eq!(edge.target, users);
    assert_eq!(
        graph.edge_table_columns(edge_id),
        d2_rs_core::EdgeTableColumns {
            source: Some(1),
            target: Some(0),
        }
    );
}

#[test]
fn lowering_promotes_bare_edge_links_to_visible_labels() {
    let parsed = parse(
        "link-only.d2",
        br#"a -> b: {
  link: https://example.com/route
}
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let (edge_id, _) = graph.edges().next().expect("link-only edge");
    assert_eq!(graph.edge_link(edge_id), Some("https://example.com/route"));
    assert_eq!(
        graph.edge_label(edge_id).map(|label| label.text.as_str()),
        Some("https://example.com/route")
    );
}

#[test]
fn lowering_preserves_edge_icon_target_metadata() {
    let parsed = parse(
        "edge-icon.d2",
        br#"a -> b: {
  icon: https://example.com/route.svg
}
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let (edge_id, _) = graph.edges().next().expect("edge with icon");
    assert_eq!(
        graph.edge_icon(edge_id),
        Some("https://example.com/route.svg")
    );
    assert_eq!(graph.edge_icon_position(edge_id), None);
    // Pinned Source rejects explicit icon-position as an edge-map key; the
    // original input remains in the complete pristine diagnostic fixture.
}

#[test]
fn explicit_empty_edge_label_is_not_replaced_by_link() {
    let parsed = parse(
        "empty-link-label.d2",
        br##"a -> b: {
  label: ""
  link: https://example.com/route
}
"##,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let (edge_id, _) = graph.edges().next().expect("empty-label edge");
    assert_eq!(graph.edge_link(edge_id), Some("https://example.com/route"));
    assert!(graph.edge_label(edge_id).is_none());
}

#[test]
fn markdown_substitution_exposes_dotted_and_nested_aliases() {
    let parsed = parse(
        "fixture.d2",
        br#"vars: {
  nested: {
    x: nested
  }
}
a: |md hello ${nested.x} |
b: |md flat ${x} |
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, labels, _) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer).unwrap();
    let node_id = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "a").then_some(id))
        .expect("markdown node");
    assert_eq!(labels[&node_id], "hello nested");
    let flat_id = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "b").then_some(id))
        .expect("flat markdown node");
    // Master collectVariables recursively exposes both nested.x and x
    // (d2ir/compile.go:798–835); the original pinned Go witness agrees.
    assert_eq!(labels[&flat_id], "flat nested");
}

#[test]
fn source_config_is_extracted_with_typed_values_and_ordered_overrides() {
    let parsed = parse(
        "config.d2",
        br#"vars: {
  d2-config: {
    sketch: true
    theme-id: 303
    dark-theme-id: 201
    pad: 42
    center: false
    layout-engine: elk
    theme-overrides: {b1: blue; B2: red}
    dark-theme-overrides: {N1: black}
    data: {
      foo: bar
      number: 17
      enabled: true
      nested: {x: y}
      values: [one; two; {ignored: value}; [three]]
    }
  }
}
a
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let config = compiler::extract_config_checked(&compiler, root)
        .unwrap()
        .expect("source config");
    assert_eq!(config.sketch, Some(true));
    assert_eq!(config.theme_id, Some(303));
    assert_eq!(config.dark_theme_id, Some(201));
    assert_eq!(config.pad, Some(42));
    assert_eq!(config.center, Some(false));
    assert_eq!(config.layout_engine.as_deref(), Some("elk"));
    assert_eq!(config.theme_overrides["B1"], "blue");
    assert_eq!(config.theme_overrides["B2"], "red");
    assert_eq!(config.dark_theme_overrides["N1"], "black");
    assert_eq!(config.data["foo"], serde_json::json!("bar"));
    assert_eq!(config.data["number"], serde_json::json!("17"));
    assert_eq!(config.data["enabled"], serde_json::json!("true"));
    // v0.9.0's d2compiler.compileConfig records non-array composites as an
    // nil []interface{} (JSON null) and includes only scalar array entries.
    assert_eq!(config.data["nested"], serde_json::Value::Null);
    assert_eq!(config.data["values"], serde_json::json!(["one", "two"]));
}

#[test]
fn source_config_theme_override_validation_matches_v090_rules() {
    let source = br#"vars: {
  d2-config: {
    theme-overrides: {
      B1: potato
      potato: B1
    }
    dark-theme-overrides: {b2: aliceblue; AA4: '#aB3'}
  }
}
a
"#;
    let parsed = parse("colors.d2", source, Some(ParseOptions::default()));
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    assert!(
        compiler.errors.is_empty(),
        "IR diagnostics: {:?}",
        compiler.errors
    );
    let errors: Vec<_> = compiler::extract_config_checked(&compiler, root)
        .unwrap_err()
        .into_iter()
        .map(|error| error.to_string())
        .collect();
    assert_eq!(
        errors,
        [
            "colors.d2:4:7: expected \"B1\" to be a valid named color (\"orange\") or a hex code (\"#f0ff3a\")",
            "colors.d2:5:7: \"potato\" is not a valid theme code",
        ]
    );
}

#[test]
fn lowering_retains_implicit_root_grid_properties() {
    let parsed = parse(
        "root-grid.d2",
        br#"grid-rows: 2
grid-columns: 3
grid-gap: 12
vertical-gap: 8
horizontal-gap: 16
a
b
c
d
e
f
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    assert_eq!(graph.root_grid_rows, Some(2));
    assert_eq!(graph.root_grid_columns, Some(3));
    assert_eq!(graph.root_grid_gap, Some(12.0));
    assert_eq!(graph.root_vertical_gap, Some(8.0));
    assert_eq!(graph.root_horizontal_gap, Some(16.0));
}

#[test]
fn lowering_preserves_column_directed_grid_declaration_order() {
    let parsed = parse(
        "column-grid.d2",
        br#"grid-columns: 2
grid-rows: 2
a
b
c
d
container: {
  grid-columns: 2
  grid-rows: 2
  x
  y
}

"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    assert_eq!(graph.root_grid_row_directed, Some(false));
    let container = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "container").then_some(id))
        .expect("container node");
    assert_eq!(graph.grid_row_directed[container.0 as usize], Some(false));
}

#[test]
fn lowering_prefers_nearest_scope_for_relative_edge_endpoints() {
    let parsed = parse(
        "scoped-edge.d2",
        br#"start
Check: {
  start -> Enter
}
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let node_id = |name: &str| {
        graph
            .nodes()
            .find_map(|(id, node)| (node.external_id == name).then_some(id))
            .expect("graph node")
    };
    let check_start = node_id("Check.start");
    let check_enter = node_id("Check.Enter");
    let (_, edge) = graph
        .edges()
        .find(|(_, edge)| edge.source == check_start && edge.target == check_enter)
        .expect("scoped edge");
    assert_eq!(edge.source, check_start);
    assert_eq!(edge.target, check_enter);
}

#[test]
fn lowering_reproduces_go_sort_slice_order_for_equal_wildcard_ranges() {
    let parsed = parse(
        "wildcard-order.d2",
        br#"l: {
  default -> layout -> here
  default: { dagre -- elk -- tala: or }
  default.* -> layout: runs
}
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let names = graph
        .nodes()
        .map(|(_, node)| node.external_id.as_str())
        .collect::<Vec<_>>();
    let edge_names = graph
        .edges()
        .map(|(_, edge)| (names[edge.source.0 as usize], names[edge.target.0 as usize]))
        .collect::<Vec<_>>();
    assert_eq!(
        edge_names,
        vec![
            ("l.default", "l.layout"),
            ("l.layout", "l.here"),
            ("l.default.dagre", "l.default.elk"),
            ("l.default.elk", "l.default.tala"),
            ("l.default.dagre", "l.layout"),
            ("l.default.elk", "l.layout"),
            ("l.default.tala", "l.layout"),
        ]
    );
}

#[test]
fn dagre_node_order_uses_the_first_ast_reference_position() {
    let parsed = parse(
        "-",
        b"container: {\n  first: { label: first }\n  second: two\n  first.child: child\n}\n",
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let order = graph
        .node_order_override
        .iter()
        .map(|id| graph.node(*id).unwrap().external_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "container",
            "container.first",
            "container.second",
            "container.first.child",
        ]
    );
}

// Upstream e2e input, copied from the pinned tree by recipe
// `lang-upstream-e2e-input` (docs/FIXTURES.md).
#[cfg(feature = "oracle-fixtures")]
#[test]
fn dagre_node_order_matches_go_127_mixed_variable_reference_sort() {
    let parsed = parse(
        "grid_label_positions.d2",
        d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/upstream/grid_label_positions.d2"),
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    let order = graph
        .node_order_override
        .iter()
        .map(|id| graph.node(*id).unwrap().external_id.as_str())
        .collect::<Vec<_>>();
    let blank_t = order.iter().position(|path| *path == "blank_t").unwrap();
    let blank_ot_blank2 = order
        .iter()
        .position(|path| *path == "blank_ot.blank2")
        .unwrap();
    assert!(
        blank_t < blank_ot_blank2,
        "Go 1.27 SortObjectsByAST puts blank_t first via its earlier edge reference: {order:?}"
    );
}

#[test]
fn explicit_empty_node_label_survives_graph_lowering() {
    let parsed = parse(
        "empty-node-label.d2",
        br#"a: ""
b: {label: ""}
c
"#,
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let graph = compiler::lower_graph_with_measurer(&compiler, root, &ProbeMeasurer).unwrap();
    for (id, node) in graph.nodes() {
        let expected = if node.external_id == "c" { "c" } else { "" };
        assert_eq!(
            graph.node_labels.get(&id).map(String::as_str),
            Some(expected),
            "{}",
            node.external_id
        );
    }
}

#[test]
fn structured_content_keeps_its_minimum_size_with_declared_dimensions() {
    fn size(source: &str) -> Size {
        let parsed = parse("-", source.as_bytes(), Some(ParseOptions::default()));
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let (graph, _, _) =
            compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer)
                .unwrap();
        graph.nodes().next().unwrap().1.size
    }
    // Go GetDefaultSize removes INNER_LABEL_PADDING for nonzero authored
    // dimensions, but retains the structured content envelope.
    for (content, minimum) in [
        (
            "shape: sql_table; id: int; name: text",
            Size {
                width: 70.0,
                height: 30.0,
            },
        ),
        (
            "shape: class; field: int",
            Size {
                width: 112.0,
                height: 90.0,
            },
        ),
        (
            "label: |go\nhello\n|",
            Size {
                width: 10.0,
                height: 10.0,
            },
        ),
    ] {
        let baseline = size(&format!("a: {{\n{content}\n}}"));
        assert!(baseline.width > 1.0 && baseline.height > 1.0);
        assert_eq!(
            size(&format!("a: {{\n{content}\nwidth: 1; height: 1\n}}")),
            minimum
        );
        assert_eq!(
            size(&format!("a: {{\n{content}\nwidth: 1000; height: 1000\n}}")),
            Size {
                width: 1000.0,
                height: 1000.0
            }
        );
    }
    assert_eq!(
        size("a: {width: 1; height: 1}"),
        Size {
            width: 1.0,
            height: 1.0
        }
    );
}

#[test]
fn lowering_preserves_edges_between_columns_of_the_same_table() {
    let parsed = parse("-", b"users: {shape: sql_table; id: int; parent_id: int}\nusers.id -> users.parent_id: parent\n", Some(ParseOptions::default()));
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, _, _) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &SqlTableProbeMeasurer)
            .unwrap();
    assert_eq!(graph.nodes().len(), 1);
    assert_eq!(graph.edges().len(), 1);
    let (edge_id, edge) = graph.edges().next().unwrap();
    assert_eq!(edge.source, edge.target);
    assert_eq!(
        graph.edge_table_columns(edge_id),
        d2_rs_core::EdgeTableColumns {
            source: Some(0),
            target: Some(1),
        }
    );
    assert_eq!(graph.edge_label(edge_id).unwrap().text, "parent");
}

#[test]
fn lowering_truncates_class_member_edges_to_the_class_object() {
    let parsed = parse(
        "-",
        b"record: {shape: class; id: int; parent: Record; id -> parent: relation}\n",
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let (graph, _, _) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer).unwrap();
    assert_eq!(graph.nodes().len(), 1);
    assert_eq!(graph.edges().len(), 1);
    let (edge_id, edge) = graph.edges().next().unwrap();
    assert_eq!(edge.source, edge.target);
    assert_eq!(
        graph.edge_table_columns(edge_id),
        d2_rs_core::EdgeTableColumns {
            source: None,
            target: None
        }
    );
    assert_eq!(graph.edge_label(edge_id).unwrap().text, "relation");
}

#[test]
fn structured_member_resolution_preserves_quoted_path_components() {
    for shape in ["class", "sql_table"] {
        let source = format!(
            "\"rec.ord\": {{shape: {shape}; \"i.d\": int; parent: int}}\n\"rec.ord\".\"i.d\" -> \"rec.ord\".parent\n"
        );
        let parsed = parse("-", source.as_bytes(), Some(ParseOptions::default()));
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let (graph, _, _) =
            compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer)
                .unwrap();
        assert_eq!(graph.nodes().len(), 1);
        assert_eq!(graph.edges().len(), 1);
        let (id, edge) = graph.edges().next().unwrap();
        assert_eq!(edge.source, edge.target);
        let columns = graph.edge_table_columns(id);
        assert_eq!(columns.source, (shape == "sql_table").then_some(0));
        assert_eq!(columns.target, (shape == "sql_table").then_some(1));
    }
}

#[test]
fn structured_headers_keep_inner_label_padding_when_they_determine_width() {
    struct HeaderMeasurer;
    impl compiler::GraphMeasurer for HeaderMeasurer {
        fn measure(&self, label: &str) -> Size {
            Size {
                width: label.chars().count() as f64 * 10.0,
                height: 10.0,
            }
        }
    }
    for (shape, compartment_padding) in [("class", 100.0), ("sql_table", 20.0)] {
        let source = format!("a: {{shape: {shape}; label: a_very_long_header; x: int}}\n");
        let parsed = parse("-", source.as_bytes(), Some(ParseOptions::default()));
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let (graph, _, _) =
            compiler::lower_graph_with_measurer_and_labels(&compiler, root, &HeaderMeasurer)
                .unwrap();
        let (_, node) = graph.nodes().next().unwrap();
        assert_eq!(
            node.size.width,
            node.label_size.unwrap().width + 5.0 + compartment_padding
        );
    }
}

#[test]
fn structured_members_resolve_in_the_nearest_lexical_scope() {
    for shape in ["sql_table", "class"] {
        let source = format!(
            "table: {{id}}\nouter: {{table: {{shape: {shape}; id: int}}; sink; table.id -> sink: local}}\n"
        );
        let parsed = parse("-", source.as_bytes(), Some(ParseOptions::default()));
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let (graph, _, _) =
            compiler::lower_graph_with_measurer_and_labels(&compiler, root, &ProbeMeasurer)
                .unwrap();
        assert_eq!(graph.edges().len(), 1);
        let (id, edge) = graph.edges().next().unwrap();
        assert_eq!(graph.node(edge.source).unwrap().external_id, "outer.table");
        assert_eq!(graph.node(edge.target).unwrap().external_id, "outer.sink");
        assert_eq!(
            graph.edge_table_columns(id).source,
            (shape == "sql_table").then_some(0)
        );
    }
}

#[test]
fn explicit_false_bold_changes_target_style_but_keeps_source_measurement_default() {
    struct StyledProbe(std::cell::RefCell<Vec<bool>>);
    impl compiler::GraphMeasurer for StyledProbe {
        fn measure(&self, _: &str) -> Size {
            Size {
                width: 10.0,
                height: 10.0,
            }
        }
        fn measure_styled_label_at_size(&self, _: &str, _: u32, bold: bool, _: bool) -> Size {
            self.0.borrow_mut().push(bold);
            self.measure("")
        }
    }
    let parsed = parse(
        "-",
        b"a: label { style.bold: false }",
        Some(ParseOptions::default()),
    );
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions::default(),
    )
    .unwrap();
    let measurer = StyledProbe(std::cell::RefCell::new(Vec::new()));
    let (graph, _, styles) =
        compiler::lower_graph_with_measurer_and_labels(&compiler, root, &measurer).unwrap();
    let id = graph
        .nodes()
        .find_map(|(id, node)| (node.external_id == "a").then_some(id))
        .unwrap();
    assert!(!styles[&id].bold);
    assert!(!measurer.0.borrow().is_empty());
    assert!(measurer.0.borrow().iter().all(|bold| *bold));
}

#[test]
fn boolean_spellings_keep_source_measurement_and_export_stages_independent() {
    use std::cell::RefCell;
    struct StyledProbe(RefCell<Vec<(String, bool, bool)>>);
    impl compiler::GraphMeasurer for StyledProbe {
        fn measure(&self, _label: &str) -> Size {
            Size {
                width: 40.0,
                height: 20.0,
            }
        }
        fn measure_styled_label_at_size(
            &self,
            label: &str,
            _size: u32,
            bold: bool,
            italic: bool,
        ) -> Size {
            self.0.borrow_mut().push((label.to_owned(), bold, italic));
            self.measure(label)
        }
    }
    // Object.Text tests literal true; Edge.Text and exporter.applyStyles use
    // ParseBool. A later export face must not change the measured label box.
    let source = b"n: Node boolean {shape: text; style.bold: 1; style.italic: 1; style.underline: 1}\na -> b: Edge boolean {style.bold: 1; style.italic: 0; style.underline: 1}\nc: {style.3d: 1; style.multiple: 1}\n";
    let parsed = parse("boolean-stages.d2", source, None);
    assert!(parsed.error.is_none());
    let (compiled, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    let probe = StyledProbe(RefCell::new(Vec::new()));
    let lowered = compiler::lower_graph_with_measurer_and_labels(&compiled, root, &probe).unwrap();
    assert!(
        probe
            .0
            .borrow()
            .contains(&("Node boolean".into(), false, false))
    );
    assert!(
        probe
            .0
            .borrow()
            .contains(&("Edge boolean".into(), true, true))
    );
    let n = lowered
        .0
        .nodes()
        .find(|(_, n)| n.external_id == "n")
        .unwrap()
        .0;
    let style = &lowered.2[&n];
    assert!(style.bold && style.italic && style.underline);
    let c = lowered
        .0
        .nodes()
        .find(|(_, n)| n.external_id == "c")
        .unwrap()
        .1;
    assert!(c.is_3d && c.is_multiple);
    let edge = lowered.0.edges().next().unwrap().0;
    assert!(lowered.0.edge_label_bold(edge));
    assert_eq!(lowered.0.edge_style(edge).italic.as_deref(), Some("0"));
    assert_eq!(lowered.0.edge_style(edge).underline.as_deref(), Some("1"));
}

// Original Go v0.9.0 Object.SizeToContent applies authored dimensions before
// LimitAR. Measurements are captured with the pinned bundled fonts.
#[test]
#[cfg(feature = "oracle-fixtures")]
fn authored_shape_dimensions_follow_source_aspect_order_and_shape_case() {
    let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/authored-shape-sizing-v090.json"
    ))
    .unwrap();
    for row in rows.as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        let parsed = parse("fixture.d2", input.as_bytes(), None);
        assert!(parsed.error.is_none(), "{input}: {:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let node = graph
            .nodes()
            .find(|(_, node)| node.external_id == "a")
            .unwrap()
            .1;
        assert_eq!(node.size.width, row["width"].as_f64().unwrap(), "{input}");
        assert_eq!(node.size.height, row["height"].as_f64().unwrap(), "{input}");
        if node.fixed_width || node.fixed_height {
            assert_eq!(node.declared_size, Some(node.size), "{input}");
        }
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn sequence_default_shapes_and_language_measurement_match_source() {
    let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/sequence-default-shapes-v090.json"
    ))
    .unwrap();
    for row in rows.as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        let parsed = parse("fixture.d2", input.as_bytes(), None);
        assert!(parsed.error.is_none(), "{input}: {:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let node = graph
            .nodes()
            .find(|(_, node)| node.external_id == row["id"].as_str().unwrap())
            .unwrap()
            .1;
        assert_eq!(node.size.width, row["width"].as_f64().unwrap(), "{input}");
        assert_eq!(node.size.height, row["height"].as_f64().unwrap(), "{input}");
        assert_eq!(
            serde_json::to_value(node.shape).unwrap(),
            row["shape"],
            "{input}"
        );
        let label = node.label_size.unwrap();
        assert_eq!(label.width, row["label_width"].as_f64().unwrap(), "{input}");
        assert_eq!(
            label.height,
            row["label_height"].as_f64().unwrap(),
            "{input}"
        );
    }
}

#[test]
fn sequence_edges_resolve_actors_again_after_fields_are_compiled() {
    // d2compiler.compileMap compiles fields before edges. Connect then calls
    // ensureChildEdge/EnsureChild again for every path component, so a later
    // actor declaration can redirect a message while retaining the old span.
    // Source: pinned d2compiler/compile.go and d2graph/d2graph.go.
    for (source, target, retained) in [
        (
            "shape: sequence_diagram\nr: {}\nr -> m.a\na -> b",
            "a",
            true,
        ),
        (
            "shape: sequence_diagram\na -> b\nr: {}\nr -> m.a",
            "a",
            false,
        ),
        (
            "shape: sequence_diagram\nr: {}\nr -> m.a\nx -> b",
            "m.a",
            true,
        ),
        (
            "shape: sequence_diagram\nr: {}\nr -> a.a\na -> b",
            "a.a",
            true,
        ),
        (
            "shape: sequence_diagram\nr: {}\nr -> m.a.x\na -> b",
            "a.x",
            true,
        ),
        ("r: {}\nr -> m.a\na -> b", "m.a", true),
    ] {
        let parsed = parse("sequence-endpoints.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let message = graph
            .edges()
            .find(|(_, edge)| graph.node(edge.source).unwrap().external_id == "r")
            .unwrap()
            .1;
        assert_eq!(
            graph.node(message.target).unwrap().external_id,
            target,
            "{source}"
        );
        assert_eq!(
            graph
                .nodes()
                .any(|(_, node)| node.external_id == "m.a" || node.external_id == "a.a"),
            retained,
            "{source}"
        );
    }

    // Each EnsureChild call materializes its own missing child. A following
    // actor redirect does not remove the intermediate object, and newObject
    // does not copy references from the original lexical span.
    for (suffix, target, children) in [
        ("", "a.x.y", vec!["a.x", "a.x.y"]),
        ("\ny -> b", "y", vec!["a.x"]),
    ] {
        let source = format!("shape: sequence_diagram\nr: {{}}\nr -> m.a.x.y\na -> b{suffix}");
        let parsed = parse("sequence-endpoints.d2", source.as_bytes(), None);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let message = graph.edges().next().unwrap().1;
        assert_eq!(graph.node(message.target).unwrap().external_id, target);
        let created = graph
            .nodes()
            .filter(|(_, node)| node.external_id.starts_with("a."))
            .map(|(id, node)| {
                let (parent, _) = node.external_id.rsplit_once('.').unwrap();
                assert_eq!(
                    graph.node(node.parent.unwrap()).unwrap().external_id,
                    parent
                );
                assert!(graph.hierarchy_children_order[&node.parent].contains(&id));
                assert!(!graph.sequence_has_edge_reference(id));
                node.external_id.as_str()
            })
            .collect::<Vec<_>>();
        assert_eq!(created, children, "{source}");
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn connection_block_tags_override_order_and_measurement_match_original() {
    let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/connection-label-language-v090.json"
    ))
    .unwrap();
    for row in rows.as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        let parsed = parse("fixture.d2", input.as_bytes(), None);
        assert!(parsed.error.is_none(), "{input}: {:?}", parsed.error);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        let label = graph.edge_label(d2_rs_core::EdgeId(0)).unwrap();
        assert_eq!(label.text, row["text"].as_str().unwrap(), "{input}");
        assert_eq!(
            label.size.width,
            row["size"]["width"].as_f64().unwrap(),
            "{input}"
        );
        assert_eq!(
            label.size.height,
            row["size"]["height"].as_f64().unwrap(),
            "{input}"
        );
        assert_eq!(
            graph
                .edge_style(d2_rs_core::EdgeId(0))
                .language
                .as_deref()
                .unwrap_or(""),
            row["language"].as_str().unwrap(),
            "{input}"
        );
    }
}

#[test]
#[cfg(feature = "oracle-fixtures")]
fn original_go_config_data_nil_composites_and_formatted_scalars_match() {
    let requests: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/config-data-inputs-v090.json")).unwrap();
    let expected: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/config-data-source-v090.json"
    ))
    .unwrap();
    assert_eq!(requests.len(), 8);
    assert_eq!(expected.len(), 8);
    for (request, source) in requests.iter().zip(&expected) {
        assert_eq!(request["name"], source["name"]);
        let parsed = parse(
            "config.d2",
            request["source"].as_str().unwrap().as_bytes(),
            Some(ParseOptions::default()),
        );
        assert!(
            parsed.error.is_none(),
            "{}: {:?}",
            request["name"],
            parsed.error
        );
        let (compiler, root) = compiler::compile_ir_with_options(
            Shared::new(parsed.map),
            compiler::CompileOptions::default(),
        )
        .unwrap();
        let config = compiler::extract_config_checked(&compiler, root)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&config.data).unwrap(),
            source["data"],
            "{}",
            request["name"]
        );
    }
}
