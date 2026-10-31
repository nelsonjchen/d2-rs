// SPDX-License-Identifier: MPL-2.0
//! Native-only invalid values cannot be carried by JSON fixture files.
use d2_rs_isometric::{
    scene::build_scene,
    target::{Connection, Diagram, GeoPoint, Legend, Shape},
};

const NUMERIC_ERROR: &str = "isometric: nonfinite or excessive numeric value";

#[test]
fn nonfinite_route_is_rejected_before_geometry() {
    for invalid in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let diagram = Diagram {
            connections: Some(vec![Connection {
                route: Some(vec![Some(GeoPoint { x: invalid, y: 0.0 })]),
                ..Default::default()
            }]),
            ..Default::default()
        };
        assert_eq!(build_scene(&diagram, None).unwrap_err(), NUMERIC_ERROR);
        assert_eq!(
            diagram.connections()[0].route()[0].unwrap().x.to_bits(),
            invalid.to_bits()
        );
    }
}

#[test]
fn aspect_ratio_admission_covers_root_nodes_and_legend() {
    for position in ["root", "node", "legend"] {
        let shape = Shape {
            content_aspect_ratio: Some(f64::NAN),
            ..Default::default()
        };
        let mut diagram = Diagram::default();
        match position {
            "root" => diagram.root = shape,
            "node" => diagram.shapes = Some(vec![shape]),
            "legend" => {
                diagram.legend = Some(Legend {
                    shapes: Some(vec![shape]),
                    ..Default::default()
                })
            }
            _ => unreachable!(),
        }
        assert_eq!(build_scene(&diagram, None).unwrap_err(), NUMERIC_ERROR);
    }
}

#[test]
fn field_names_do_not_consume_authored_text_budget() {
    let mut diagram = Diagram::default();
    diagram.description = "x".repeat(8 << 20);
    assert!(build_scene(&diagram, None).is_ok());
    diagram.description.push('x');
    assert_eq!(
        build_scene(&diagram, None).unwrap_err(),
        "isometric: text exceeds 8388608 bytes"
    );
}
