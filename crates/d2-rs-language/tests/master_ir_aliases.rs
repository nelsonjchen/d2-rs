// SPDX-License-Identifier: MPL-2.0
//! Public d2ir/expansion.go aliases and nil-map contracts at master 6a7b48378.
use d2_rs_core::{LayoutError, ast, ir::*};
use d2_rs_language::compiler::{Compiler, VariableExpansionError};

fn aliases(distinct_scalar_nodes: bool) -> (Compiler, NodeId, NodeId) {
    aliases_with_budget(distinct_scalar_nodes, 0)
}

fn aliases_with_budget(distinct_scalar_nodes: bool, limit: i64) -> (Compiler, NodeId, NodeId) {
    use d2_rs_language::compiler::{CompileLimits, CompileOptions, compile_ir_with_limits};
    let (mut c, _) = compile_ir_with_limits(
        ast::Shared::new(ast::Map::default()),
        CompileOptions::default(),
        CompileLimits {
            max_variable_expansion: limit,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(c.errors.is_empty());
    let root = c.arena.new_root();
    let value = ast::ScalarBox::from_child(ast::raw_string_box("payload", true).unbox());
    let scalar = c.arena.insert(
        None,
        Node::Scalar(Scalar {
            value: value.clone(),
        }),
    );
    let other = if distinct_scalar_nodes {
        c.arena.insert(None, Node::Scalar(Scalar { value }))
    } else {
        scalar
    };
    let fields = [scalar, other]
        .into_iter()
        .map(|primary| {
            c.arena.insert(
                Some(root),
                Node::Field(Field {
                    primary: Some(primary),
                    ..Default::default()
                }),
            )
        })
        .collect();
    let Node::Map(map) = c.arena.node_mut(root) else {
        unreachable!()
    };
    map.fields = Some(fields);
    (c, root, scalar)
}

#[test]
fn nil_map_rejection_precedes_context_and_keeps_budget() {
    let c = Compiler::default();
    for copy in [false, true] {
        let context =
            Some(Box::new(|| -> Option<LayoutError> { panic!("nil map polled context") }) as _);
        let result = if copy {
            c.reserve_variable_expansion_copy(None, context)
        } else {
            c.reserve_variable_expansion_aliases(None, context)
        };
        assert!(matches!(result,Err(VariableExpansionError::NilMap {copy:got}) if got==copy));
        assert_eq!(c.variable_expansion_usage(), (65_536, 0));
    }
}

#[test]
fn repeated_nodes_and_distinct_nodes_sharing_scalar_values_are_charged() {
    for distinct in [false, true] {
        let (c, root, scalar) = aliases(distinct);
        let cost = c.variable_expansion_node_units(scalar) - u64::from(distinct);
        assert!(cost > 0);
        c.reserve_variable_expansion_aliases(root, None).unwrap();
        assert_eq!(c.variable_expansion_usage().1, cost);
        c.reserve_variable_expansion_aliases(root, None).unwrap();
        assert_eq!(
            c.variable_expansion_usage().1,
            2 * cost,
            "reservations share compilation budget"
        );
    }
}

#[test]
fn alias_reservation_keeps_prior_charges_and_typed_context_cause() {
    let (c, root, scalar) = aliases(false);
    let cost = c.variable_expansion_node_units(scalar);
    c.reserve_variable_expansion_aliases(root, None).unwrap();
    let error = c
        .reserve_variable_expansion_aliases(
            root,
            Some(Box::new(|| Some(LayoutError::DeadlineExceeded))),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        VariableExpansionError::Context(LayoutError::DeadlineExceeded)
    ));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(c.variable_expansion_usage().1, cost);
    c.reserve_variable_expansion_aliases(root, None).unwrap();
    assert_eq!(c.variable_expansion_usage().1, 2 * cost);
}

#[test]
fn refused_alias_reservation_keeps_earlier_successful_charges() {
    let (control, _, scalar) = aliases(false);
    let cost = control.variable_expansion_node_units(scalar);
    let (c, root, _) = aliases_with_budget(false, cost as i64 + 1);
    c.reserve_variable_expansion_aliases(root, None).unwrap();
    assert_eq!(c.variable_expansion_usage().1, cost);
    assert!(
        matches!(c.reserve_variable_expansion_aliases(root,None),Err(VariableExpansionError::Limit(limit)) if limit==cost+1)
    );
    assert_eq!(c.variable_expansion_usage().1, cost);
}
