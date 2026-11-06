// SPDX-License-Identifier: MPL-2.0
//! Actual upstream board calls plus the meshBuilder's append/error contracts.
// Oracle tier: tests reading regenerated fixtures are gated (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
#[path = "../examples/support/primitive_protocol.rs"]
mod protocol;
use d2_rs_isometric::img::geometry_core::{MAX_NATIVE_TRIANGLES, MeshGeometry};
use d2_rs_isometric::img::mesh::{Material, Vertex, material, nv};
use d2_rs_scene::Nrgba;

#[cfg(feature = "oracle-fixtures")]
#[test]
fn actual_golden_board_tessellations() {
    let calls: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/mesh-primitives.json"
    ))
    .unwrap();
    fn equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
        use serde_json::Value;
        match (a, b) {
            (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len() && a.iter().all(|(k, a)| b.get(k).is_some_and(|b| equal(a, b)))
            }
            _ => a == b,
        }
    }
    assert!(!calls.as_array().unwrap().is_empty());
    for call in calls.as_array().unwrap() {
        let result = protocol::replay(call).unwrap();
        assert!(equal(&call["want"], &result), "{}", call["op"]);
    }
}

#[test]
fn retained_prefix_consumes_the_same_triangle_budget() {
    let mut mesh = MeshGeometry::with_prefix_count(MAX_NATIVE_TRIANGLES - 1);
    let m = material(Material {
        color: Nrgba::new(1, 2, 3, 255),
        ..Default::default()
    });
    mesh.box_(nv(0.0, 0.0, 0.0), nv(1.0, 1.0, 1.0), &m, 0.0);
    assert_eq!(mesh.triangles.len(), 1);
    assert_eq!(
        mesh.err.as_deref(),
        Some("isometric geometry exceeds 1000000 triangles")
    );
    mesh.fail("a later failure");
    mesh.flat(
        nv(0.0, 0.0, 0.0),
        nv(1.0, 0.0, 0.0),
        nv(0.0, 0.0, 1.0),
        &m,
        false,
    );
    assert_eq!(mesh.triangles.len(), 1);
    assert_eq!(
        mesh.err.as_deref(),
        Some("isometric geometry exceeds 1000000 triangles")
    );
}

#[test]
fn invisible_and_absent_materials_do_not_consume_exhausted_budget() {
    let mut mesh = MeshGeometry::with_prefix_count(MAX_NATIVE_TRIANGLES);
    let m = material(Material::default());
    let v = Vertex::default();
    mesh.triangle(v, v, v, &m, true);
    mesh.triangle_opt(v, v, v, None, true);
    assert!(mesh.triangles.is_empty());
    assert!(mesh.err.is_none());
}
