// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ir::{Arena, Node, Scalar};
#[test]
fn typed_nil_scalar_preserves_go_method_contracts() {
    // d2ir/d2ir.go: (*Scalar).Primary and Map do not dereference the pointer;
    // Parent, Copy, AST and LastRef do. A non-nil zero Scalar is different.
    let mut arena = Arena::default();
    let nil = arena.nil_scalar();
    assert_eq!(nil, arena.nil_scalar());
    let zero = arena.insert(None, Node::Scalar(Scalar::default()));
    assert_ne!(nil, zero);
    assert!(arena.is_nil_scalar(nil));
    assert!(!arena.is_nil_scalar(zero));
    assert_eq!(arena.primary(nil), None);
    assert_eq!(arena.primary(zero), Some(zero));
    assert_eq!(arena.map(nil), None);
    assert!(arena.ast(zero).is_none());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arena.parent(nil))).is_err());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arena.ast(nil))).is_err());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arena.copy(nil, None))).is_err()
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| arena.last_ref(nil))).is_err()
    );
}
