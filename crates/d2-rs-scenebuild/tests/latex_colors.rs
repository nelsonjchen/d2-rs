// SPDX-License-Identifier: MPL-2.0
//! Contracts from upstream 614eb8968 latex_test.go and 1a3c6b71b
//! e2etests-cli/raster_test.go, exercised through the shared export scene.
use d2_rs_scene::{Asset, Document, Node, Nrgba, Paint, Primitive};
use d2_rs_scenebuild::{AssetOptions, Options, SvgImportBudget, SvgImportLimits, build};
use d2_rs_target::Diagram;
use serde_json::{Value, json};

fn options() -> Options {
    Options {
        pad: Some(2),
        assets: Some(AssetOptions {
            svg_import_limits: SvgImportLimits {
                max_bytes: 1_000_000,
                max_depth: 100,
                max_elements: 10_000,
                max_attributes: 100_000,
                max_attribute_bytes: 1_000_000,
                max_path_commands: 1_000_000,
                max_transform_functions: 10_000,
                max_use_depth: 100,
                max_resources: 10_000,
                max_decoded_raster_bytes: 0,
            },
            svg_import_budget: SvgImportBudget {
                max_source_bytes: 4_000_000,
                max_elements: 40_000,
                max_attributes: 400_000,
                max_attribute_bytes: 4_000_000,
                max_path_commands: 4_000_000,
                max_transform_functions: 40_000,
                max_declared_resources: 40_000,
                max_expanded_use_instances: 40_000,
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn shape(id: &str, x: i64, formula: &str, color: &str) -> Value {
    let size = d2_rs_math::measure(if formula.is_empty() { "x" } else { formula }).unwrap();
    json!({
        "id": id, "type": "text", "pos": {"x": x, "y": 0},
        "width": size.width.ceil() as i64, "height": size.height.ceil() as i64,
        "opacity": 1, "fill": "none", "stroke": "black", "strokeWidth": 2,
        "label": formula, "language": "latex", "color": color,
        "labelWidth": size.width.ceil() as i64, "labelHeight": size.height.ceil() as i64,
        "labelPosition": "INSIDE_MIDDLE_CENTER"
    })
}

fn document(shapes: Vec<Value>, connections: Vec<Value>) -> Document {
    let diagram: Diagram = serde_json::from_value(json!({
        "root": {}, "shapes": shapes, "connections": connections
    }))
    .unwrap();
    build(&diagram, &options()).unwrap()
}

fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.id == id {
        return Some(node);
    }
    node.children().iter().find_map(|child| find(child, id))
}

fn label_asset<'a>(document: &'a Document, id: &str) -> (&'a str, &'a Node) {
    let label = find(document.root.as_ref().unwrap(), &format!("{id}:label:0")).unwrap();
    let Some(Primitive::Image(image)) = &label.primitive else {
        panic!("LaTeX image")
    };
    let Asset::VectorAsset {
        root: Some(root), ..
    } = &document.assets[&image.asset]
    else {
        panic!("LaTeX vector asset")
    };
    (&image.asset, root)
}

fn colors(root: &Node) -> Vec<Nrgba> {
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Some(Primitive::Path(path)) = &node.primitive {
            if let Some(Paint::SolidPaint { color }) = &path.fill {
                result.push(*color);
            }
        }
        stack.extend(node.children());
    }
    result
}

fn frame(document: &Document) -> d2_rs_raster::NrgbaImage {
    d2_rs_raster::render(
        document,
        &d2_rs_raster::FrameOptions {
            scale: 4.0,
            max_width: 2_000,
            max_height: 1_000,
            max_pixels: 2_000_000,
            max_nodes: 10_000,
            max_depth: 100,
            max_path_commands: 1_000_000,
            max_animation_tracks: 10_000,
            max_animation_keyframes: 100_000,
            max_assets: 100,
            max_asset_bytes: 64 * 1024 * 1024,
            max_decoded_asset_bytes: 64 * 1024 * 1024,
            max_import_depth: 100,
            max_offscreen_bytes: 64 * 1024 * 1024,
            max_even_odd_clip_work: 1_000_000_000,
            max_scanline_work: 1_000_000_000,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn shape_and_connection_share_font_color_assets() {
    let formula = "a + b = c";
    let mut a = shape("a", 0, formula, "#000000");
    a["stroke"] = "#00ff00".into();
    a["labelFill"] = "#ff0000".into();
    a["underline"] = true.into();
    a["bold"] = true.into();
    let connection = json!({
        "id": "edge", "src": "a", "dst": "b", "opacity": 1,
        "srcArrow": "none", "dstArrow": "none",
        "stroke": "black", "strokeWidth": 2, "fill": "#00ff00",
        "label": formula, "language": "latex", "color": "#000000",
        "labelWidth": a["labelWidth"], "labelHeight": a["labelHeight"], "fontSize": 16,
        "labelFill": "#ff0000", "underline": true,
        "labelPosition": "INSIDE_MIDDLE_CENTER", "labelPercentage": 0.5,
        "route": [{"x": 100, "y": 30}, {"x": 200, "y": 30}]
    });
    let document = document(vec![a, shape("b", 200, "", "black")], vec![connection]);
    let (shape_id, _) = label_asset(&document, "a");
    let (edge_id, _) = label_asset(&document, "edge");
    assert_eq!(shape_id, edge_id);
    let root = document.root.as_ref().unwrap();
    assert!(find(root, "a:label-fill").is_none());
    assert!(find(root, "edge:label-fill").is_none());
}

#[test]
fn current_color_preserves_explicit_tex_colors() {
    let formula = r"\textcolor{red}{y} = x";
    let document = document(
        vec![
            shape("first", 0, formula, "#112233"),
            shape("second", 150, formula, "#445566"),
        ],
        vec![],
    );
    assert_eq!(document.assets.len(), 2);
    assert_ne!(
        label_asset(&document, "first").0,
        label_asset(&document, "second").0
    );
    for (id, inherited) in [
        ("first", Nrgba::new(0x11, 0x22, 0x33, 255)),
        ("second", Nrgba::new(0x44, 0x55, 0x66, 255)),
    ] {
        let paints = colors(label_asset(&document, id).1);
        assert!(paints.contains(&inherited));
        assert!(paints.contains(&Nrgba::new(255, 0, 0, 255)));
    }
}

#[test]
fn font_color_renders_blue_pixels() {
    let document = document(vec![shape("formula", 0, "x^2 + y^2", "#0055cc")], vec![]);
    let frame = frame(&document);
    assert!(frame.pix.chunks_exact(4).any(|p| p[3] > 0));
    assert!(
        frame
            .pix
            .chunks_exact(4)
            .any(|p| p[3] > 0 && p[2] > p[1] && p[1] > p[0])
    );
}

#[test]
fn gradient_font_colors_render_identically_to_first_stops() {
    let make_document = |shape_color: &str, edge_color: &str| {
        let mut a = shape("a", 0, r"x + \textcolor{red}{y}", shape_color);
        a["type"] = "rectangle".into();
        document(
            vec![a, shape("b", 200, "", "black")],
            vec![json!({
            "id": "edge", "src": "a", "dst": "b", "opacity": 1,
            "srcArrow": "none", "dstArrow": "none",
                "stroke": "black", "strokeWidth": 2,
                "label": "z", "language": "latex", "color": edge_color,
                "labelWidth": 9, "labelHeight": 8, "fontSize": 16,
                "labelPosition": "INSIDE_MIDDLE_CENTER", "labelPercentage": 0.5,
                "route": [{"x": 100, "y": 30}, {"x": 200, "y": 30}]
            })],
        )
    };
    let gradient = make_document(
        "linear-gradient(to right, #008855 25%, blue)",
        "radial-gradient(circle, #993366, white)",
    );
    let solid = make_document("#008855", "#993366");
    for id in ["a", "edge"] {
        assert_eq!(label_asset(&gradient, id), label_asset(&solid, id));
    }
    let got = frame(&gradient);
    let want = frame(&solid);
    assert_eq!(got.rect, want.rect);
    assert_eq!(got.pix, want.pix);
    assert!(got.pix.chunks_exact(4).any(|p| p[3] > 0));
}
