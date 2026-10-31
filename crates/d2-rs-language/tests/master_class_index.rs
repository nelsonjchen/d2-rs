// SPDX-License-Identifier: MPL-2.0
#[path = "../examples/master-class-index-scoreboard.rs"]
#[allow(dead_code)]
mod runner;
use d2_rs_core::{LayoutError, ast, ir};
use d2_rs_language::{compiler, parse};
use serde_json::Value;
use std::{cell::Cell, rc::Rc};

#[test]
#[cfg(feature = "oracle-fixtures")]
fn every_original_lookup_and_retained_index_contract() {
    let fixture: Value =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/master-class-index.json"))
            .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut expected = case["got"].clone();
        let encoded = expected["results"].take();
        expected["results"] = encoded["indexes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|index| encoded["values"][index.as_u64().unwrap() as usize].clone())
            .collect();
        assert_eq!(
            runner::evaluate(case["request"].clone()),
            expected,
            "{}",
            case["name"]
        );
    }
}

fn compile(source: &str) -> (compiler::Compiler, ir::NodeId) {
    let parsed = parse("class-index.d2", source.as_bytes(), None);
    assert!(parsed.error.is_none());
    let (c, root) = compiler::compile_ir(ast::Shared::new(parsed.map)).unwrap();
    assert!(c.errors.is_empty());
    (c, root)
}

#[test]
fn deadline_discards_partial_index_and_retains_original_cause() {
    let (c, root) = compile("classes: {one: {}; two: {}; three: {}}\na");
    let checks = Rc::new(Cell::new(0));
    let ticks = checks.clone();
    let mut lookup = compiler::ClassLookup::new(
        &c.arena,
        Some(Box::new(move || {
            ticks.set(ticks.get() + 1);
            (ticks.get() >= 2).then_some(LayoutError::DeadlineExceeded)
        })),
    );
    assert!(matches!(
        lookup.get(root, b"one"),
        Err(LayoutError::DeadlineExceeded)
    ));
    assert_eq!(lookup.indexed_root_count(), 0);
    assert_eq!(checks.get(), 2);
    assert!(matches!(
        lookup.get(root, b"three"),
        Err(LayoutError::DeadlineExceeded)
    ));
    assert_eq!(checks.get(), 2);
    let mut fresh = compiler::ClassLookup::new(&c.arena, None);
    assert!(fresh.get(root, b"three").unwrap().is_some());
}

#[test]
fn empty_index_checks_context_and_cache_hits_reuse_the_completed_index() {
    let (c, root) = compile("classes: {one: {}}\na");
    let checks = Rc::new(Cell::new(0));
    let ticks = checks.clone();
    let mut lookup = compiler::ClassLookup::new(
        &c.arena,
        Some(Box::new(move || {
            ticks.set(ticks.get() + 1);
            None
        })),
    );
    assert!(lookup.get(root, b"one").unwrap().is_some());
    assert_eq!(checks.get(), 2); // one member plus final publication check
    assert!(lookup.get(root, b"ONE").unwrap().is_some());
    assert_eq!(lookup.get(root, b"missing").unwrap(), None);
    assert_eq!(checks.get(), 2);
    let (empty, empty_root) = compile("a");
    let mut canceled = compiler::ClassLookup::new(
        &empty.arena,
        Some(Box::new(|| Some(LayoutError::Cancelled))),
    );
    assert!(matches!(
        canceled.get(empty_root, b"missing"),
        Err(LayoutError::Cancelled)
    ));
    assert_eq!(canceled.indexed_root_count(), 0);
}

fn field(arena: &mut ir::Arena, parent: ir::NodeId, name: ast::StringBox) -> ir::NodeId {
    let id = arena.insert(
        Some(parent),
        ir::Node::Field(ir::Field {
            name,
            ..Default::default()
        }),
    );
    arena.append_field(parent, id);
    id
}
fn map(arena: &mut ir::Arena, parent: Option<ir::NodeId>) -> ir::NodeId {
    arena.insert(parent, ir::Node::Map(ir::Map::default()))
}

#[test]
fn lookup_uses_root_namespace_unicode_folding_and_first_nil_shadow() {
    let mut arena = ir::Arena::default();
    let root = arena.new_root();
    let classes = field(&mut arena, root, ast::raw_string_box("classes", true));
    let definitions = map(&mut arena, Some(classes));
    if let ir::Node::Field(field) = arena.node_mut(classes) {
        field.composite = Some(definitions);
    }
    // Quoted reserved names must be invisible to an unquoted class query.
    let quoted = field(
        &mut arena,
        definitions,
        ast::StringBox {
            double_quoted_string: Some(ast::Shared::new(ast::DoubleQuotedString::flat("shape"))),
            ..Default::default()
        },
    );
    let quoted_map = map(&mut arena, Some(quoted));
    if let ir::Node::Field(field) = arena.node_mut(quoted) {
        field.composite = Some(quoted_map);
    }
    let nil = field(&mut arena, definitions, ast::raw_string_box("ſhadow", true));
    let later = field(&mut arena, definitions, ast::raw_string_box("SHADOW", true));
    let later_map = map(&mut arena, Some(later));
    if let ir::Node::Field(field) = arena.node_mut(later) {
        field.composite = Some(later_map);
    }
    let unicode = field(&mut arena, definitions, ast::raw_string_box("ΣK", true));
    let unicode_map = map(&mut arena, Some(unicode));
    if let ir::Node::Field(field) = arena.node_mut(unicode) {
        field.composite = Some(unicode_map);
    }
    let child = map(&mut arena, Some(nil));
    let mut lookup = compiler::ClassLookup::new(&arena, None);
    assert_eq!(lookup.get(child, b"shadow").unwrap(), None);
    assert_eq!(lookup.get(root, b"shape").unwrap(), None);
    assert_eq!(
        lookup.get(root, "ςk".as_bytes()).unwrap(),
        Some(unicode_map)
    );
    assert_eq!(lookup.indexed_root_count(), 1);
}
