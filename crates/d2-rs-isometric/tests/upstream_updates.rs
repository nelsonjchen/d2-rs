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

#[test]
fn selected_theme_resolves_endpoint_ink_without_changing_metadata() {
    use d2_rs_isometric::{
        scene::{RenderOpts, build_scene},
        target::Connection,
    };
    let mut d = caption_diagram();
    d.connections = Some(vec![Connection {
        id: "paint".into(),
        src: "orders".into(),
        dst: "orders".into(),
        opacity: 1.0,
        src_label: Some(Text {
            label: "one".into(),
            ..Default::default()
        }),
        dst_label: Some(Text {
            label: "many".into(),
            color: "B1".into(),
            ..Default::default()
        }),
        ..Default::default()
    }]);
    let s = build_scene(
        &d,
        Some(&RenderOpts {
            theme_id: Some(200),
            ..Default::default()
        }),
    )
    .unwrap();
    let e = &s.edges[0];
    assert!(e.font_explicit);
    assert_eq!(e.font_color.to_uppercase(), "#CDD6F4");
    assert_eq!(
        e.source_label.as_ref().unwrap().color.to_uppercase(),
        "#CDD6F4"
    );
    assert_eq!(
        e.target_label.as_ref().unwrap().color.to_uppercase(),
        "#CBA6F7"
    );
    assert_eq!(e.metadata.original.src_label.as_ref().unwrap().color, "");
    assert_eq!(e.metadata.original.dst_label.as_ref().unwrap().color, "B1");
    assert!(s.nodes[0].fill_explicit && s.nodes[0].stroke_explicit && s.nodes[0].font_explicit);
}

#[test]
fn captions_are_readable_in_projection() {
    use d2_rs_isometric::{
        img::{
            framing::native_camera_axes,
            mesh::{ndot, nv},
            route_caption_style::native_connection_caption_surface,
            route_labels::RouteCaptionPlacer,
        },
        target::Connection,
    };
    let camera = native_camera_axes();
    for end in [
        nv(0.0, 0.0, 12.0),
        nv(0.0, 0.0, -12.0),
        nv(1.0, 0.0, 12.0),
        nv(-1.0, 0.0, -12.0),
        nv(12.0, 0.0, 3.0),
        nv(-12.0, 0.0, -3.0),
    ] {
        let points = [nv(0.0, 0.0, 0.0), end];
        let c = Connection {
            label_position: "INSIDE_MIDDLE_CENTER".into(),
            ..Default::default()
        };
        let fixed = native_connection_caption_surface(&points, &c, 1.0, 0.3, 0.01).unwrap();
        let auto = RouteCaptionPlacer::new().place(&points, 0.5, 1.0, 0.3);
        for s in [fixed, auto] {
            let baseline = nv(s.angle.cos(), 0.0, s.angle.sin());
            let (x, y) = (ndot(baseline, camera.right), ndot(baseline, camera.up));
            assert!(x > 0.0 && y.abs() <= x);
        }
    }
}

#[test]
fn short_wire_clears_final_upright_caption() {
    use d2_rs_isometric::{
        img::{mesh::*, route_caption_style::native_connection_caption_surface},
        target::Connection,
    };
    let end = nv(0.8, 0.0, 0.3);
    let axis = nunit(end);
    let c = Connection {
        label_position: "OUTSIDE_BOTTOM_CENTER".into(),
        stroke_width: 2,
        ..Default::default()
    };
    let s = native_connection_caption_surface(&[Vec::default(), end], &c, 2.0, 0.3, 0.01).unwrap();
    assert_eq!(s.angle, 0.0);
    let offset = ndot(nsub(s.center, nmul(end, 0.5)), nv(-axis.z, 0.0, axis.x));
    let clearance = offset - (axis.z.abs() * 2.0 + axis.x.abs() * 0.3) / 2.0;
    assert!((clearance - 0.06).abs() < 1e-8);
}

#[test]
fn headings_find_top_strip_between_visible_wires() {
    use d2_rs_isometric::{
        img::{
            geometry::LabelSurface, hierarchy_headers::place_hierarchy_header, mesh::nv,
            route_labels::RouteCaptionPlacer,
        },
        scene::{Board, Node, NodeMetadata},
    };
    let board = Board {
        size: nv(10.0, 0.14, 4.0),
        ..Default::default()
    };
    let owner = Node {
        metadata: NodeMetadata {
            original: Shape {
                label_position: "INSIDE_TOP_CENTER".into(),
                ..Default::default()
            },
        },
        ..Default::default()
    };
    let header = LabelSurface {
        width: 1.0,
        depth: 0.4,
        center: nv(0.0, 0.03, 0.0),
        ..Default::default()
    };
    let mut reservations = RouteCaptionPlacer::new();
    for x in [-4.5, 0.0, 4.5] {
        reservations.avoid_route(&[nv(x, 0.03, -2.0), nv(x, 0.03, 2.0)], 0.06);
    }
    let (got, clear) = place_hierarchy_header(
        header,
        &board,
        &owner,
        &[],
        0.01,
        None,
        Some(&mut reservations),
    );
    assert!(
        clear && got.center.x.abs() >= 0.5 && got.center.z <= -1.7,
        "{got:?}, {clear}"
    );
}

#[test]
fn adjacent_fill_fragments_cancel_shared_edges() {
    use d2_rs_isometric::img::{
        svg::{svg_fill_path, svg_polygon_path},
        svg_visibility::SvgPoint,
    };
    let mut polygons = vec![];
    for x in 0..8 {
        polygons.push(
            vec![(x, 0), (x + 1, 0), (x + 1, 1), (x, 1)]
                .into_iter()
                .map(|(x, y)| SvgPoint {
                    x: x as f64,
                    y: y as f64,
                    z: 0.0,
                })
                .collect(),
        );
    }
    let fill = svg_fill_path(&polygons);
    assert_eq!(fill.matches('M').count(), 1);
    assert!(fill.len() < svg_polygon_path(&polygons).len());
}

#[test]
fn split_paint_groups_have_one_keyboard_stop_per_object() {
    use d2_rs_isometric::img::{mesh::*, svg::{NativeScene,NativeLink,native_scene_svg}};
    let mut geometry = d2_rs_isometric::img::geometry_core::MeshGeometry::default();
    for x in [0.0,2.0] {
        let mat = material(Material { color: d2_rs_scene::Nrgba::new(100,150,200,255), unlit: true, ..Default::default() });
        let v = |x,z| Vertex { position:nv(x,0.0,z),normal:nv(0.0,1.0,0.0),..Default::default() };
        geometry.triangle(v(x,0.0),v(x+1.0,0.0),v(x,1.0),&mat,false);
    }
    let camera = raster_fit(&geometry.triangles,native_view_direction(),400,250,1.08);
    let mut scene = NativeScene { triangles:geometry.triangles, camera,width:400,height:250,
        links:vec![NativeLink { mesh_start:0, mesh_end:2, url:"https://example.test/object".into(), ..Default::default() }],..Default::default() };
    let svg = native_scene_svg(&mut scene,&|_,_| Ok(String::new())).unwrap();
    assert_eq!(svg.matches("<a xlink:href=").count(),2);
    assert_eq!(svg.matches(r#"tabindex="-1""#).count(),1);
}
