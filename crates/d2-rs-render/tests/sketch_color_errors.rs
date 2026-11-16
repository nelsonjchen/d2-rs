use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};

#[test]
fn sketch_double_oval_propagates_css_color_parser_errors() {
    // Pinned d2sketch.DoubleOval passes the authored fill to
    // renderSketchOverlay -> lib/color.Luminance -> csscolorparser.Parse.
    // Unlike Oval, it does not pass the rendered gradient URL instead.
    for (fill, error) in [
        ("linear-gradient(red, blue)", "Invalid format"),
        ("radial-gradient(#00ff00, #ff00ff)", "Invalid format"),
        (
            "linear-gradient(red, green, blue)",
            "Invalid color format, linear-gradient(red, green, blue)",
        ),
    ] {
        let input = serde_json::json!({
            "shapes": [{
                "id": "a", "type": "oval", "pos": {"x": 0, "y": 0},
                "width": 100, "height": 80, "opacity": 1,
                "strokeWidth": 2, "fill": fill, "stroke": "red",
                "double-border": true
            }]
        });
        let target = TargetDiagram::decode_json(&serde_json::to_vec(&input).unwrap())
            .unwrap()
            .unwrap();
        let options = SvgRenderOptions {
            sketch: Some(true),
            ..Default::default()
        };
        assert_eq!(
            target.render_flat_svg(&options).unwrap_err(),
            error,
            "{fill}"
        );
        // The same gradient remains admitted by the ordinary SVG painter.
        assert!(target.render_flat_svg(&SvgRenderOptions::default()).is_ok());
    }
}
