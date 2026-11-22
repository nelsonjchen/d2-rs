use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use serde_json::json;

fn render(shape_color: &str, connection_color: &str) -> String {
    let diagram: TargetDiagram = serde_json::from_value(json!({
        "shapes": [
            {"id":"x", "type":"rectangle", "width":100, "height":60, "opacity":1,
             "strokeWidth":2, "stroke":"black", "fill":"white", "color":shape_color,
             "label":"x + \\textcolor{red}{y}", "language":"latex", "labelWidth":60, "labelHeight":20},
            {"id":"y", "type":"rectangle", "pos":{"x":200,"y":0}, "width":100, "height":60,
             "opacity":1, "strokeWidth":2, "stroke":"black", "fill":"white"}
        ],
        "connections":[{"id":"(x -> y)[0]", "src":"x", "dst":"y", "srcArrow":"none", "dstArrow":"triangle", "opacity":1,
            "stroke":"black", "strokeWidth":2, "color":connection_color,
            "route":[{"x":100,"y":30},{"x":200,"y":30}],
            "label":"z", "language":"latex", "labelWidth":20, "labelHeight":20,
            "labelPosition":"OUTSIDE_TOP_CENTER", "labelPercentage":0.5}]
    })).unwrap();
    String::from_utf8(
        diagram
            .render_flat_svg(&SvgRenderOptions::default())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn latex_labels_use_font_colors_independently_of_strokes() {
    // Upstream 614eb8968: changing style.font-color changes the LaTeX group,
    // while explicit TeX textcolor remains in charge of its own glyphs.
    let svg = render("#008855", "#993366");
    assert!(svg.contains(r##" color="#008855""##));
    assert!(svg.contains(r##" color="#993366""##));
    assert!(svg.contains(r#" fill="red""#));
    assert!(svg.contains(r#"stroke="black""#));
}

#[test]
fn latex_gradient_font_colors_use_first_stop_for_shapes_and_connections() {
    // SVG half of upstream 1a3c6b71b TestLatexGradientFontColor.
    let svg = render(
        "linear-gradient(to right, #008855 25%, blue)",
        "radial-gradient(circle, #993366, white)",
    );
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let colors: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("g"))
        .filter_map(|n| n.attribute("color"))
        .collect();
    assert!(colors.contains(&"#008855"));
    assert!(colors.contains(&"#993366"));
    assert!(!colors.iter().any(|c| c.contains("gradient")));
    assert!(svg.contains(r#" fill="red""#));
}

#[test]
fn latex_default_font_color_resolves_through_theme() {
    let svg = render("", "N1");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let labels: Vec<_> = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name("g")
                && n.attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "color-N1"))
        })
        .collect();
    assert_eq!(labels.len(), 2);
    // Upstream d2themes/d2themes.go:129: default neutral N1 is #0A0F25.
    assert!(
        labels
            .iter()
            .all(|n| n.attribute("color") == Some("#0A0F25"))
    );
}
