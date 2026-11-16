use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use d2_rs_text::d2fonts::{CustomFonts, FontFamily, FontStyle, font_face};

fn regular_face(font_family: &str, custom_fonts: CustomFonts) -> String {
    let input = serde_json::json!({
        "fontFamily": font_family,
        "shapes": [{
            "id": "a", "type": "text", "pos": {"x": 0, "y": 0},
            "width": 40, "height": 40, "opacity": 1, "strokeWidth": 2,
            "fill": "transparent", "stroke": "N1", "color": "N1",
            "label": "x λ", "language": "markdown",
            "fontSize": 16, "labelWidth": 40, "labelHeight": 40,
            "labelPosition": "INSIDE_MIDDLE_CENTER"
        }]
    });
    let target = TargetDiagram::decode_json(&serde_json::to_vec(&input).unwrap())
        .unwrap()
        .unwrap();
    let options = SvgRenderOptions {
        custom_fonts,
        ..Default::default()
    };
    let svg = String::from_utf8(target.render_flat_svg(&options).unwrap()).unwrap();
    let start = svg.find("-font-regular;\n\tsrc: url(\"").unwrap();
    let end = start + svg[start..].find("\");").unwrap();
    svg[start..end].to_owned()
}

#[test]
fn embedded_regular_face_follows_the_diagram_font_family() {
    // d2svg.embedFonts subsets diagram.FontFamily. d2exporter replaces the
    // CLI's custom family with Source Code Pro under a mono theme, so a
    // custom regular face must not be embedded for that diagram.
    let mut fonts = CustomFonts::default();
    fonts.regular.insert(
        FontStyle::Regular,
        font_face(FontFamily::HandDrawn.font(FontStyle::Regular)).to_vec(),
    );
    fonts.register().unwrap();
    let builtin = regular_face("SourceCodePro", CustomFonts::default());
    assert_eq!(regular_face("SourceCodePro", fonts.clone()), builtin);
    assert_ne!(regular_face("custom", fonts), builtin);
}
