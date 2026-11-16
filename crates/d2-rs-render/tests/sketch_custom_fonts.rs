use d2_rs_core::{Graph, LayoutReport, LayoutResult, Score};
use d2_rs_render::{RenderOptions, SvgRenderOptions, target::TargetDiagram};
use d2_rs_text::d2fonts::{CustomFonts, FontFamily, FontStyle, font_face};
use std::collections::BTreeMap;

#[test]
fn sketch_markdown_uses_custom_family_fallback_metrics() {
    // Pinned d2cli.loadFonts -> d2fonts.AddFontFamily fills unsupplied
    // styles with SourceSansPro, and d2lib.applyDefaults keeps that family
    // under --sketch. The empty-heading rules and baseline below match
    // Go for `a: |md\n#\n\n##\n\ntext\n|` with a bold/italic override.
    let mut input = serde_json::json!({
        "shapes": [{
            "id": "a", "type": "text", "pos": {"x": 0, "y": 0},
            "width": 27, "height": 134, "opacity": 1, "strokeWidth": 2,
            "fill": "transparent", "stroke": "N1", "color": "N1",
            "label": "#\n\n##\n\ntext", "language": "markdown",
            "fontSize": 16, "labelWidth": 27, "labelHeight": 134,
            "labelPosition": "INSIDE_MIDDLE_CENTER"
        }]
    });
    let result = LayoutResult {
        removed_edges: Default::default(),
        source_edge_storage: Default::default(),
        synthetic_root: None,
        route_curves: Default::default(),
        sequence_lifelines: Vec::new(),
        boxes: Default::default(),
        routes: Default::default(),
        edge_labels: Default::default(),
        edge_label_percentage_commits: Default::default(),
        node_labels: Default::default(),
        report: LayoutReport {
            selected_seed: 1,
            selected_pass: 0,
            score: Score::default(),
            improvement_passes: 0,
            candidates: Vec::new(),
            warnings: Vec::new(),
        },
    };
    for role in [FontStyle::Bold, FontStyle::Italic] {
        let mut fonts = CustomFonts::default();
        fonts.regular.insert(
            role,
            font_face(FontFamily::HandDrawn.font(FontStyle::Regular)).to_vec(),
        );
        fonts.mono.insert(
            FontStyle::Regular,
            font_face(FontFamily::SourceCodePro.font(FontStyle::Regular)).to_vec(),
        );
        let exported: serde_json::Value = serde_json::from_slice(
            &d2_rs_render::target_diagram_json(
                &Graph::default(),
                &result,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &RenderOptions {
                    custom_fonts: fonts.clone(),
                    ..Default::default()
                },
                true,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(exported["fontFamily"], "custom");
        assert_eq!(exported["monoFontFamily"], "customMono");
        assert!(d2_rs_text::d2fonts::is_registered_family("custom"));
        assert!(d2_rs_text::d2fonts::is_registered_family("customMono"));
        input["fontFamily"] = exported["fontFamily"].clone();
        let target = TargetDiagram::decode_json(&serde_json::to_vec(&input).unwrap())
            .unwrap()
            .unwrap();
        let options = SvgRenderOptions {
            sketch: Some(true),
            custom_fonts: fonts,
            ..Default::default()
        };
        let svg = String::from_utf8(target.render_flat_svg(&options).unwrap()).unwrap();
        assert!(
            svg.contains("width=\"27.000\" height=\"1.000\""),
            "{role:?}"
        );
        assert!(svg.contains("y=\"76.800\""), "{role:?}");
        assert!(
            !svg.contains("width=\"34.000\" height=\"1.000\""),
            "{role:?}"
        );
    }
}
