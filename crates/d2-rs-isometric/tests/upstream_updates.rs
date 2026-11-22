// SPDX-License-Identifier: MPL-2.0
//! Contracts ported from the isometric changes after 6a7b4837.
use d2_rs_isometric::{
    document::wrap_svg,
    img::{
        page::render_page_with_context,
        render::{Options, render_svg_with_context},
    },
    target::{Diagram, Shape, Text},
};

fn caption_diagram() -> Diagram {
    Diagram {
        shapes: Some(vec![Shape {
            id: "orders".into(),
            type_: "cylinder".into(),
            width: 100,
            height: 70,
            opacity: 1.0,
            stroke_width: 2,
            fill: "B4".into(),
            stroke: "N1".into(),
            text: Text {
                label: "Orders".into(),
                label_width: 48,
                label_height: 20,
                font_size: 16,
                ..Default::default()
            },
            label_position: "OUTSIDE_RIGHT_TOP".into(),
            link: "https://example.test/orders".into(),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

#[test]
fn padded_gradient_viewport_keeps_normalized_coordinates() {
    // 2b67cc567: both backing and gradient stretch with the wrapper.
    let native = r#"<svg width="400" height="250"><svg width="100%" height="100%" viewBox="0 0 1 1" preserveAspectRatio="none"><rect width="1" height="1"/></svg></svg>"#;
    let output =
        String::from_utf8(wrap_svg(native, "test", 100, None, false, true, true, "").unwrap())
            .unwrap();
    assert!(output.contains(r#"<svg x="-100" y="-100" width="600" height="450" viewBox="0 0 1 1" preserveAspectRatio="none">"#));
}

#[test]
fn detached_caption_has_separate_page_region_and_painted_svg_link() {
    // cea6ebe10: body links must not cover the gap to outside captions.
    let d = caption_diagram();
    let o = Options {
        width: 400,
        height: 250,
        fit_content: true,
        ..Default::default()
    };
    let page = render_page_with_context(None, Some(&d), Some(&o)).unwrap();
    assert_eq!(page.links.len(), 2);
    for link in &page.links {
        assert_eq!(link.url, "https://example.test/orders");
    }
    let svg = render_svg_with_context(None, Some(&d), Some(&o)).unwrap();
    let anchor = svg
        .split(r#"<a xlink:href="https://example.test/orders">"#)
        .nth(1)
        .unwrap()
        .split("</a>")
        .next()
        .unwrap();
    assert!(anchor.contains("<use xlink:href="));
    assert!(
        anchor.contains(r#"<rect width="1" height="1" fill="transparent" pointer-events="all"/>"#)
    );
}

#[test]
fn root_frame_is_below_lowest_platform() {
    // 77db96718: a root frame must follow descending nested platforms.
    use d2_rs_isometric::{
        img::{
            geometry::MeshBuilder,
            mesh::{Triangle, Vertex, nv},
        },
        scene::Scene,
    };
    let mut b = MeshBuilder {
        scale: 0.01,
        ..Default::default()
    };
    b.triangles.push(Triangle {
        v: [
            Vertex {
                position: nv(0.0, -0.3, 0.0),
                ..Default::default()
            },
            Vertex {
                position: nv(1.0, -0.3, 0.0),
                ..Default::default()
            },
            Vertex {
                position: nv(0.0, -0.3, 1.0),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    let s = Scene {
        root: Shape {
            stroke_width: 3,
            stroke: "#394a56".into(),
            opacity: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    b.root_decorations(&s, None).unwrap();
    assert_eq!(b.panels.len(), 1);
    let frame = &b.panels[0];
    for t in &b.triangles[frame.first..frame.last] {
        for v in &t.v {
            assert!(v.position.y < -0.3);
        }
    }
}
