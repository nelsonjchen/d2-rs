// SPDX-License-Identifier: MPL-2.0
//! Complete public Go scene frames; no native backend fallback is permitted.
use super::*;
#[cfg(feature = "oracle-fixtures")]
use sha2::{Digest, Sha256};

#[test]
fn generated_sketch_legend_retains_rough_geometry_and_native_backend() {
    use d2_rs_core::{LayoutOptions, Size, ast::Shared};
    use d2_rs_language::{compiler, parse};
    let parsed=parse("",br#"vars:{d2-legend:{a:A;b:{label:B;shape:class;field:int};c:{label:C;shape:sql_table;id:int;style.font:mono};d:{label:D;shape:c4-person};e:{label:E;style.multiple:true};f:{label:F;style.double-border:true}}}
x"#,None);
    assert!(parsed.error.is_none());
    let (ir, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
    let (graph, labels, styles) =
        compiler::lower_graph_with_measurer_and_labels(&ir, root, &|_: &str| Size {
            width: 8.0,
            height: 20.0,
        })
        .unwrap();
    let layout = d2_rs_tala::Engine
        .layout(&graph, &LayoutOptions::default())
        .unwrap();
    let (svg, paths) = d2_rs_render::render_svg_with_raster_sources_checked(
        &graph,
        &layout,
        &labels,
        &styles,
        RenderOptions::default(),
        true,
    )
    .unwrap();
    let svg = configure_d2_markers_for_native_raster(&strip_embedded_font_faces(
        std::str::from_utf8(&svg).unwrap(),
    ));
    let doc = roxmltree::Document::parse(&svg).unwrap();
    for index in 0..6 {
        let id = format!("d2-rs-native-legend-shape-icon-{index}");
        let icon = doc
            .descendants()
            .find(|n| n.attribute("id") == Some(&id))
            .unwrap();
        assert!(
            icon.descendants()
                .any(|n| n.attribute("data-d2-rs-sketch-set") == Some("rough")),
            "Sketch dispatch lost for {id}"
        );
    }
    let mut ids = BTreeSet::new();
    for id in doc.descendants().filter_map(|n| n.attribute("id")) {
        assert!(ids.insert(id), "Duplicate generated ID: {id}");
    }
    let (tagged, sources) = svg_precision::tag_embedded_svg_images(&svg);
    let tagged_doc = roxmltree::Document::parse(&tagged).unwrap();
    for path in tagged_doc
        .descendants()
        .filter(|n| n.attribute("data-d2-rs-sketch-set") == Some("rough"))
    {
        let id = path
            .attribute("id")
            .expect("Every generated rough path retains an identity");
        assert!(
            sources.raw_path(id).is_some(),
            "Generated rough path lost f64 operations: {id}"
        );
    }
    let mut options = resvg::usvg::Options {
        font_family: "Source Sans Pro".into(),
        ..Default::default()
    };
    load_bundled_fonts_for_primary(&mut options, false, true);
    let tree = resvg::usvg::Tree::from_str(&tagged, &options).unwrap();
    assert!(
        scanline::render_simple_with_sources(&tree, 2, &paths, &sources).is_some(),
        "Generated sketch legend must remain on the native backend"
    );
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_go_complete_fractional_sketch_frames() {
    use d2_rs_core::{LayoutOptions, Node, Size};
    let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/sketch-affine-frames-source-v090.json"
    ))
    .unwrap();
    let mut rows = rows.as_array().unwrap().clone();
    for extra in [
        d2_rs_testdata::oracle_fixture!("tests/fixtures/sketch-c4-affine-source-v090.json"),
        d2_rs_testdata::oracle_fixture!("tests/fixtures/sketch-ellipse-affine-source-v090.json"),
    ] {
        let extra: serde_json::Value = serde_json::from_str(extra).unwrap();
        rows.extend(extra.as_array().unwrap().iter().cloned());
    }
    assert_eq!(rows.len(), 120);
    let capture = std::env::var_os("D2_RS_TEST_SKETCH_OUTPUT").map(std::path::PathBuf::from);
    if let Some(root) = &capture {
        std::fs::create_dir_all(root).unwrap();
    }
    let mut templates = BTreeMap::new();
    for (shape, fill) in ["rectangle", "c4-person", "oval"]
        .into_iter()
        .flat_map(|shape| ["#f7f8fe", "#ffffff", "#0a0f25", "#ffcc00"].map(|fill| (shape, fill)))
    {
        let mut graph = Graph::default();
        let id = graph.add_node(Node::new(
            "probe",
            Size {
                width: 120.0,
                height: 80.0,
            },
        ));
        graph.node_mut(id).unwrap().shape = match shape {
            "c4-person" => ShapeKind::C4Person,
            "oval" => ShapeKind::Oval,
            _ => ShapeKind::Rectangle,
        };
        let mut layout = d2_rs_tala::Engine
            .layout(&graph, &LayoutOptions::default())
            .unwrap();
        layout.boxes.insert(
            id,
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 120.0,
                    height: 80.0,
                },
            },
        );
        let styles = BTreeMap::from([(
            id,
            SourceNodeStyle {
                fill: Some(fill.into()),
                stroke: Some("#000410".into()),
                stroke_width: Some("2".into()),
                ..Default::default()
            },
        )]);
        let (svg, _) = d2_rs_render::render_svg_with_raster_sources_checked(
            &graph,
            &layout,
            &BTreeMap::new(),
            &styles,
            RenderOptions::default(),
            true,
        )
        .unwrap();
        let svg = strip_embedded_font_faces(std::str::from_utf8(&svg).unwrap());
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let geometry = doc
            .descendants()
            .find(|n| n.has_tag_name("g") && n.attribute("class") == Some("shape"))
            .unwrap();
        let body = svg[geometry.range()].to_owned();
        let defs = doc
            .descendants()
            .filter(|n| {
                n.has_tag_name("defs")
                    && !n
                        .ancestors()
                        .skip(1)
                        .any(|a| a.has_tag_name("defs") || a == geometry)
            })
            .map(|n| &svg[n.range()])
            .collect::<String>();
        let css = doc
            .descendants()
            .filter(|n| n.has_tag_name("style"))
            .map(|n| &svg[n.range()])
            .collect::<String>();
        templates.insert((shape, fill), (defs, css, body));
    }
    let mut failures = Vec::new();
    for row in &rows {
        let name = row["name"].as_str().unwrap();
        let (defs, css, body) = &templates[&(
            row["shape"].as_str().unwrap_or("rectangle"),
            row["fill"].as_str().unwrap(),
        )];
        let m = row["matrix"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_f64().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" class="d2-rs-theme d2-rs-sketch" width="192" height="128" viewBox="0 0 192 128" preserveAspectRatio="xMinYMin meet">{defs}{css}<svg id="d2-rs-native-canvas-viewport-sketch-control" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="192" height="128" viewBox="0 0 192 128"><g id="sketch-shape" stroke-linecap="round" stroke-linejoin="round" transform="matrix({m})">{body}</g></svg></svg>"##
        );
        if let Some(root) = &capture {
            std::fs::write(root.join(format!("{name}.svg")), &svg).unwrap();
        }
        let (tagged, sources) = svg_precision::tag_embedded_svg_images(&svg);
        let tree = resvg::usvg::Tree::from_str(&tagged, &resvg::usvg::Options::default()).unwrap();
        let Some((w, h, pixels)) = scanline::render_simple_with_sources(
            &tree,
            row["scale"].as_u64().unwrap() as u32,
            &[],
            &sources,
        ) else {
            failures.push((name.to_owned(), "native backend unavailable".into()));
            continue;
        };
        assert_eq!(
            (w, h),
            (
                row["width"].as_u64().unwrap() as u32,
                row["height"].as_u64().unwrap() as u32
            )
        );
        if let Some(root) = &capture {
            std::fs::write(root.join(format!("{name}.rgba")), &pixels).unwrap();
        }
        let digest = format!("{:x}", Sha256::digest(&pixels));
        if digest != row["rgba_sha256"].as_str().unwrap() {
            failures.push((name.to_owned(), digest));
        }
    }
    assert!(
        failures.is_empty(),
        "Complete Go sketch frames differ: {failures:?}"
    );
}
