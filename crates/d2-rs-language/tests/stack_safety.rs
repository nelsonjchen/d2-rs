// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{ast::Shared, ir::Node};

#[test]
fn deeply_dotted_key_compiles_on_the_default_thread_stack() {
    // Pinned d2parser.Parse + d2ir.Compile accept this path and return every
    // field. Dotted paths have no AST brace nesting to trigger a parser limit.
    let depth = 4_000;
    let source = std::iter::repeat_n("a", depth)
        .collect::<Vec<_>>()
        .join(".");
    let parsed = d2_rs_language::parse("depth.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, mut map) =
        d2_rs_language::compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
    for index in 0..depth {
        let Node::Map(node) = compiler.arena.node(map) else {
            panic!("expected map at segment {index}");
        };
        let fields = node.fields.as_deref().unwrap();
        assert_eq!(fields.len(), 1, "segment {index}");
        let Node::Field(field) = compiler.arena.node(fields[0]) else {
            panic!("expected field at segment {index}");
        };
        assert_eq!(field.name.scalar_bytes(), b"a");
        if index + 1 == depth {
            assert!(field.composite.is_none());
        } else {
            map = field.composite.expect("missing path segment");
        }
    }

    // d2compiler.Compile also returns every graph object. A shorter chain is
    // enough to overflow the former recursive graph validator on this test
    // thread's ordinary stack, and bounds the unmeasured lowering work.
    let depth = 1_000;
    let parsed = d2_rs_language::parse("depth.d2", &source.as_bytes()[..2 * depth - 1], None);
    assert!(parsed.error.is_none(), "{:?}", parsed.error);
    let (compiler, root) = d2_rs_language::compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    assert!(compiler.errors.is_empty(), "{:?}", compiler.errors);
    let graph = d2_rs_language::compiler::lower_graph_with_measurer(&compiler, root, &|_: &str| {
        d2_rs_core::Size::default()
    })
    .unwrap();
    assert_eq!(graph.nodes.len(), depth);
    let metadata = graph.source_metadata.as_ref().unwrap();
    let first_key = &metadata.objects[&d2_rs_core::NodeId(0)].references[0].key;
    let mut path = String::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if !path.is_empty() {
            path.push('.');
        }
        path.push('a');
        assert_eq!(node.external_id, path);
        assert_eq!(node.parent.map(|id| id.0 as usize), index.checked_sub(1));
        let references = &metadata.objects[&d2_rs_core::NodeId(index as u32)].references;
        assert_eq!(references.len(), 1);
        assert!(std::sync::Arc::ptr_eq(first_key, &references[0].key));
        assert_eq!(references[0].key_path_index, index);
        assert_eq!(references[0].map_key_edge_index, -1);
    }
    // Inspection still expands every component, with original ranges and
    // ordering. Dropping the compiler must leave the graph independently owned.
    let Node::Map(map) = compiler.arena.node(root) else {
        unreachable!()
    };
    let expected_key =
        serde_json::json!(match compiler.arena.node(map.fields.as_ref().unwrap()[0]) {
            Node::Field(field) => field.references.as_ref().unwrap()[0]
                .read()
                .key_path
                .clone(),
            _ => unreachable!(),
        });
    drop(compiler);
    for id in [
        d2_rs_core::NodeId(0),
        d2_rs_core::NodeId((depth - 1) as u32),
    ] {
        let object = &metadata.objects[&id];
        let json = object.to_json();
        assert_eq!(json["references"][0]["key"], expected_key);
        assert_eq!(json["references"][0]["key_path_index"], id.0);
        assert_eq!(json["references"][0]["map_key_edge_index"], -1);
    }
}
