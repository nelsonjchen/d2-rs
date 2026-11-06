// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Original D2 v0.9.0 sketch geometry and texture paint order.

use super::{
    PaletteView, escape, gradient_definition, number, patternassets, shapes, style_flag,
    themed_color,
};
use d2_rs_core::{LabelPosition, Point, Rect, ShapeKind};
use d2_rs_language::compiler::SourceNodeStyle;
use std::{collections::BTreeSet, fmt::Write};

/// Ordinary rectangular targets. Effect shapes retain their separate painter
/// until the Source effect geometry is translated.
pub(crate) fn draw_rectangle(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    mask_id: Option<&str>,
    streaks: &mut BTreeSet<&'static str>,
    native_scene: bool,
) -> bool {
    if !matches!(
        shape,
        ShapeKind::Rectangle
            | ShapeKind::Square
            | ShapeKind::RealSquare
            | ShapeKind::Oval
            | ShapeKind::Circle
    ) || style_flag(Some(style), |s| s.three_dee.as_ref())
    {
        return false;
    }
    let multiple = style_flag(Some(style), |s| s.multiple.as_ref());
    let double = style_flag(Some(style), |s| s.double_border.as_ref());
    if !multiple && !double {
        return draw_rectangle_layers(
            svg,
            shape,
            rect,
            style,
            palette,
            mask_id,
            streaks,
            native_scene,
            true,
        );
    }
    // Original buildSketchShapeEffects: the duplicate is sharp, main geometry
    // is rough; double borders append their streak only after both outlines.
    let shadow = style_flag(Some(style), |s| s.shadow.as_ref());
    if shadow {
        svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">");
    }
    let mut base = style.clone();
    base.multiple = None;
    base.double_border = None;
    base.shadow = None;
    if multiple {
        let back = Rect {
            origin: Point {
                x: rect.origin.x + 10.0,
                y: rect.origin.y - 10.0,
            },
            ..rect
        };
        let mut sharp = base.clone();
        if !double || matches!(shape, ShapeKind::Oval | ShapeKind::Circle) {
            sharp.fill_pattern = None;
        }
        svg.push_str("<g stroke-linecap=\"butt\" stroke-linejoin=\"round\">");
        super::draw_shape_with_pattern_overlay(
            svg,
            shape,
            back,
            &sharp,
            palette,
            "",
            native_scene,
            false,
        );
        if double && back.size.width > 10.0 && back.size.height > 10.0 {
            sharp.fill_pattern = None;
            shapes::draw_shape(
                svg,
                shape,
                Rect {
                    origin: Point {
                        x: back.origin.x + 5.0,
                        y: back.origin.y + 5.0,
                    },
                    size: d2_rs_core::Size {
                        width: back.size.width - 10.0,
                        height: back.size.height - 10.0,
                    },
                },
                &sharp,
                palette,
            );
        }
        svg.push_str("</g>");
    }
    draw_rectangle_layers(
        svg,
        shape,
        rect,
        &base,
        palette,
        mask_id,
        streaks,
        native_scene,
        !double,
    );
    if double {
        if rect.size.width > 10.0 && rect.size.height > 10.0 {
            let mut inner = base.clone();
            inner.fill = Some("transparent".into());
            inner.fill_pattern = None;
            draw_rectangle_layers(
                svg,
                shape,
                Rect {
                    origin: Point {
                        x: rect.origin.x + 5.0,
                        y: rect.origin.y + 5.0,
                    },
                    size: d2_rs_core::Size {
                        width: rect.size.width - 10.0,
                        height: rect.size.height - 10.0,
                    },
                },
                &inner,
                palette,
                mask_id,
                streaks,
                native_scene,
                false,
            );
        }
        let fill = themed_color(base.fill.as_deref(), "none", palette);
        let mask = mask_id
            .map(|id| format!(" mask=\"url(#{})\"", escape(id)))
            .unwrap_or_default();
        append_streak(svg, shape, rect, &fill, &mask, streaks, native_scene);
    }
    if shadow {
        svg.push_str("</g>");
    }
    true
}

fn draw_rectangle_layers(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    mask_id: Option<&str>,
    streak_categories: &mut BTreeSet<&'static str>,
    native_scene: bool,
    streak: bool,
) -> bool {
    if !matches!(
        shape,
        ShapeKind::Rectangle
            | ShapeKind::Square
            | ShapeKind::RealSquare
            | ShapeKind::Oval
            | ShapeKind::Circle
    ) || style_flag(Some(style), |s| s.multiple.as_ref())
        || style_flag(Some(style), |s| s.three_dee.as_ref())
        || style_flag(Some(style), |s| s.double_border.as_ref())
    {
        return false;
    }
    let options = d2_rs_sketch::SketchOptions {
        seed: 1,
        bowing: 2.0,
        ..Default::default()
    };
    let mut paths = if matches!(shape, ShapeKind::Oval | ShapeKind::Circle) {
        d2_rs_sketch::rough_solid_ellipse_paths(
            Point {
                x: rect.size.width / 2.0,
                y: rect.size.height / 2.0,
            },
            rect.size.width,
            rect.size.height,
            options,
        )
    } else {
        d2_rs_sketch::rough_solid_rect_paths(
            Rect {
                origin: Point::default(),
                ..rect
            },
            options,
        )
    };
    if native_scene {
        paths.fill = structured_scene_path(&paths.fill);
        paths.outline = structured_scene_path(&paths.outline);
    } else {
        paths.fill = super::sketch_svg::path_data(&paths.fill);
        paths.outline = super::sketch_svg::path_data(&paths.outline);
    }
    let linecap = if native_scene { "round" } else { "butt" };
    let rendering = if native_scene {
        ""
    } else {
        " shape-rendering=\"geometricPrecision\""
    };
    let shadow = style_flag(Some(style), |s| s.shadow.as_ref());
    if shadow {
        svg.push_str("<g filter=\"url(#d2-rs-shadow-filter)\">\n");
    }
    let mut base = style.clone();
    base.fill_pattern = None;
    let (attrs, _) = shapes::attributes(&base, palette);
    let builtin = style.fill_pattern.as_deref().and_then(patternassets::id);
    let mask = mask_id
        .map(|id| format!(" mask=\"url(#{})\"", escape(id)))
        .unwrap_or_default();
    for path in [&paths.fill, &paths.outline] {
        let _ = writeln!(
            svg,
            "<g{mask}><path data-d2-rs-sketch-set=\"rough\" transform=\"translate({} {})\" d=\"{}\" {attrs} stroke-linecap=\"{linecap}\" stroke-linejoin=\"round\"{rendering}/></g>",
            number(rect.origin.x),
            number(rect.origin.y),
            escape(path)
        );
        if let Some(pattern) = builtin {
            let _ = writeln!(
                svg,
                "<g style=\"mix-blend-mode:multiply\"{mask}><path class=\"{}-overlay\" transform=\"translate({} {})\" d=\"{}\" fill=\"url(#{pattern})\" stroke=\"none\"/></g>",
                escape(style.fill_pattern.as_deref().unwrap()),
                number(rect.origin.x),
                number(rect.origin.y),
                escape(path)
            );
        }
    }
    let default_fill = palette.color("B6", palette.base.b6);
    let fill = themed_color(style.fill.as_deref(), &default_fill, palette);
    if streak {
        append_streak(
            svg,
            shape,
            rect,
            &fill,
            &mask,
            streak_categories,
            native_scene,
        );
    }
    if shadow {
        svg.push_str("</g>\n");
    }
    true
}

// d2sketch/scene.go::finiteRoughData truncates each generated operand before
// retaining a typed scene command. Keep ordinary SVG rough helpers unchanged.
fn structured_scene_path(path: &str) -> String {
    let n = |v: f64| (v * 1_000_000.0).trunc() / 1_000_000.0;
    let mut out = String::new();
    for segment in svgtypes::PathParser::from(path) {
        match segment.expect("native rough generator emits valid paths") {
            svgtypes::PathSegment::MoveTo { abs: true, x, y } => {
                let _ = write!(out, "M{} {} ", n(x), n(y));
            }
            svgtypes::PathSegment::LineTo { abs: true, x, y } => {
                let _ = write!(out, "L{} {} ", n(x), n(y));
            }
            svgtypes::PathSegment::CurveTo {
                abs: true,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let _ = write!(
                    out,
                    "C{} {} {} {} {} {} ",
                    n(x1),
                    n(y1),
                    n(x2),
                    n(y2),
                    n(x),
                    n(y)
                );
            }
            _ => unreachable!("native rough rectangles and lines emit only absolute M/L/C"),
        }
    }
    out.trim_end().to_owned()
}

/// Source buildSketchStructuredShape applies shared paint to both rough sets.
pub(crate) fn draw_structured_rect(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    streak: bool,
    raster: bool,
    streak_categories: &mut BTreeSet<&'static str>,
) {
    // d2sketch's SVG keeps the default butt cap; d2scenebuild strokes sketch
    // outlines with CapRound/JoinRound (sketch.go).
    let cap = if raster { "round" } else { "butt" };
    let paths = d2_rs_sketch::rough_solid_rect_paths(
        Rect {
            origin: Point::default(),
            ..rect
        },
        d2_rs_sketch::SketchOptions {
            seed: 1,
            bowing: 2.0,
            ..Default::default()
        },
    );
    let mut base = style.clone();
    base.fill_pattern = None;
    let (attrs, _) = shapes::attributes(&base, palette);
    for path in [&paths.fill, &paths.outline] {
        let _ = writeln!(
            svg,
            "<path data-d2-rs-sketch-set=\"rough\" transform=\"translate({} {})\" d=\"{}\" {attrs} stroke-linecap=\"{cap}\" stroke-linejoin=\"round\"/>",
            number(rect.origin.x),
            number(rect.origin.y),
            escape(&structured_scene_path(path))
        );
        if let Some(pattern) = style.fill_pattern.as_deref().and_then(patternassets::id) {
            let _ = writeln!(
                svg,
                "<g style=\"mix-blend-mode:multiply\"><path data-d2-rs-sketch-set=\"rough\" transform=\"translate({} {})\" d=\"{}\" fill=\"url(#{pattern})\" stroke=\"none\"/></g>",
                number(rect.origin.x),
                number(rect.origin.y),
                escape(&structured_scene_path(path))
            );
        }
    }
    if streak {
        draw_streak_rect(svg, rect, style, palette, streak_categories);
    }
}

/// Source buildSketchStreakOverlays appends SQL texture after all member text.
pub(crate) fn draw_streak_rect(
    svg: &mut String,
    rect: Rect,
    style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    streaks: &mut BTreeSet<&'static str>,
) {
    let fill = themed_color(style.fill.as_deref(), "none", palette);
    append_streak(svg, ShapeKind::Rectangle, rect, &fill, "", streaks, true);
}
fn append_streak(
    svg: &mut String,
    shape: ShapeKind,
    rect: Rect,
    fill: &str,
    mask: &str,
    streaks: &mut BTreeSet<&'static str>,
    native_scene: bool,
) {
    let Some(streak) = streak_style(fill) else {
        return;
    };
    if streaks.insert(streak.category) {
        svg.push_str(&streak_definition_for_mode(streak, native_scene));
    }
    let transform = if native_scene {
        String::new()
    } else {
        format!(
            " transform=\"translate({} {})\"",
            number(rect.origin.x),
            number(rect.origin.y)
        )
    };
    let rect = if native_scene {
        rect
    } else {
        Rect {
            origin: Point::default(),
            ..rect
        }
    };
    let primitive = if matches!(shape, ShapeKind::Oval | ShapeKind::Circle) {
        format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"url(#d2-rs-sketch-streak-{})\" stroke=\"none\"/>",
            number(rect.origin.x + rect.size.width / 2.0),
            number(rect.origin.y + rect.size.height / 2.0),
            number(rect.size.width / 2.0),
            number(rect.size.height / 2.0),
            streak.category
        )
    } else {
        format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"url(#d2-rs-sketch-streak-{})\" stroke=\"none\"/>",
            number(rect.origin.x),
            number(rect.origin.y),
            number(rect.size.width),
            number(rect.size.height),
            streak.category
        )
    };
    let primitive = if native_scene {
        primitive
    } else {
        format!("<g{transform}>{primitive}</g>")
    };
    let _ = writeln!(
        svg,
        "<g class=\"sketch-streak-overlay sketch-streak-{}\" style=\"mix-blend-mode:{}\"{mask}>{primitive}</g>",
        streak.category, streak.blend
    );
}

/// Source structured separators paint the rough line's fill with no stroke.
pub(crate) fn draw_structured_line(
    svg: &mut String,
    start: Point,
    end: Point,
    style: &SourceNodeStyle,
    fill: &str,
) {
    let path = d2_rs_sketch::rough_line_path(
        start,
        end,
        d2_rs_sketch::SketchOptions {
            seed: 1,
            bowing: 2.0,
            ..Default::default()
        },
    );
    let _ = writeln!(
        svg,
        "<path data-d2-rs-sketch-set=\"rough\" d=\"{}\" fill=\"{}\" stroke=\"none\" stroke-width=\"1\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
        escape(&structured_scene_path(&path)),
        escape(fill)
    );
    if let Some(pattern) = style.fill_pattern.as_deref().and_then(patternassets::id) {
        let _ = writeln!(
            svg,
            "<g style=\"mix-blend-mode:multiply\"><path data-d2-rs-sketch-set=\"rough\" d=\"{}\" fill=\"url(#{pattern})\" stroke=\"none\"/></g>",
            escape(&structured_scene_path(&path))
        );
    }
}

#[derive(Clone, Copy)]
struct StreakStyle {
    category: &'static str,
    blend: &'static str,
    ink: &'static str,
    alpha: u8,
}

fn streak_style(fill: &str) -> Option<StreakStyle> {
    let luminance = if gradient_definition(fill).is_some() || fill.starts_with("url(") {
        0.7
    } else {
        let color = fill.parse::<svgtypes::Color>().ok()?;
        // Original lib/color.Luminance uses the encoded sRGB components.
        0.299 * (f64::from(color.red) / 255.0)
            + 0.587 * (f64::from(color.green) / 255.0)
            + 0.114 * (f64::from(color.blue) / 255.0)
    };
    let (category, blend, ink, alpha) = if luminance >= 0.88 {
        ("bright", "darken", "#000000", 26)
    } else if luminance >= 0.55 {
        ("normal", "color-burn", "#000000", 41)
    } else if luminance >= 0.30 {
        ("dark", "overlay", "#000000", 82)
    } else {
        ("darker", "lighten", "#ffffff", 61)
    };
    Some(StreakStyle {
        category,
        blend,
        ink,
        alpha,
    })
}

fn streak_definition_for_mode(style: StreakStyle, native_scene: bool) -> String {
    if native_scene {
        return streak_definition(style);
    }
    let alpha = match style.category {
        "bright" => 0.1,
        "normal" => 0.16,
        "dark" => 0.32,
        _ => 0.24,
    };
    format!(
        "<defs><pattern id=\"d2-rs-sketch-streak-{}\" width=\"100\" height=\"100\" patternUnits=\"userSpaceOnUse\"><path fill=\"{}\" fill-opacity=\"{}\" fill-rule=\"evenodd\" clip-rule=\"evenodd\" d=\"{}\"/></pattern></defs>",
        style.category,
        style.ink,
        alpha,
        patternassets::streak_path()
    )
}

fn streak_definition(style: StreakStyle) -> String {
    let mut output = format!(
        "<defs><pattern id=\"d2-rs-sketch-streak-{}\" width=\"100\" height=\"100\" patternUnits=\"userSpaceOnUse\">",
        style.category
    );
    let subpaths: Vec<_> = patternassets::streak_path()
        .trim()
        .split_inclusive('Z')
        .collect();
    // Source sketchStreakRoot preserves the sole nested compound (16,17).
    // Keeping the other shapes local also bounds raster point-in-edge work.
    assert_eq!(subpaths.len(), 85, "pinned streak asset topology");
    let mut index = 0;
    while index < subpaths.len() {
        let path = if index == 16 {
            index += 1;
            format!("{}{}", subpaths[16], subpaths[17])
        } else {
            subpaths[index].to_owned()
        };
        let _ = write!(
            output,
            "<path fill-rule=\"evenodd\" fill=\"{}\" fill-opacity=\"{}\" stroke=\"none\" d=\"{}\"/>",
            style.ink,
            f64::from(style.alpha) / 255.0,
            path
        );
        index += 1;
    }
    output.push_str("</pattern></defs>\n");
    output
}

pub(crate) fn is_border(position: LabelPosition) -> bool {
    matches!(
        position,
        LabelPosition::BorderTopLeft
            | LabelPosition::BorderTopCenter
            | LabelPosition::BorderTopRight
            | LabelPosition::BorderLeftTop
            | LabelPosition::BorderLeftMiddle
            | LabelPosition::BorderLeftBottom
            | LabelPosition::BorderRightTop
            | LabelPosition::BorderRightMiddle
            | LabelPosition::BorderRightBottom
            | LabelPosition::BorderBottomLeft
            | LabelPosition::BorderBottomCenter
            | LabelPosition::BorderBottomRight
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_core::{Size, theme};
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn rectangle_painter_matches_original_go_svg_geometry_and_order() {
        let source: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-rectangle-source-v090.json"
        ))
        .unwrap();
        let svg_source: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/sketch-svg-rectangle-source-v090.json"
        ))
        .unwrap();
        let expected_text = format!("<svg>{}</svg>", svg_source[0]["svg"].as_str().unwrap());
        let expected_doc = roxmltree::Document::parse(&expected_text).unwrap();
        let expected_paths = expected_doc
            .descendants()
            .filter(|n| {
                n.has_tag_name("path")
                    && n.attribute("class")
                        .unwrap_or("")
                        .split_whitespace()
                        .any(|c| c == "shape")
            })
            .collect::<Vec<_>>();
        assert_eq!(expected_paths.len(), 2);
        let mut svg = String::new();
        assert!(draw_rectangle(
            &mut svg,
            ShapeKind::Rectangle,
            Rect {
                origin: Point { x: 10.0, y: 20.0 },
                size: Size {
                    width: 140.0,
                    height: 80.0
                }
            },
            &SourceNodeStyle {
                fill: Some("#ffffff".into()),
                stroke: Some("#112233".into()),
                stroke_width: Some("2".into()),
                fill_pattern: Some("dots".into()),
                ..Default::default()
            },
            PaletteView::new(theme::palette(0).unwrap(), &BTreeMap::new()),
            None,
            &mut BTreeSet::new(),
            false,
        ));
        let svg = format!("<svg>{svg}</svg>");
        let document = roxmltree::Document::parse(&svg).unwrap();
        let paints: Vec<_> = document
            .root_element()
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() != "defs")
            .collect();
        assert_eq!(
            paints.len(),
            source["paint_order"].as_array().unwrap().len()
        );
        let mut actual_order = Vec::new();
        let mut rough_index = 0;
        for node in &paints {
            let primitive = node.children().find(|n| n.is_element()).unwrap();
            if primitive.attribute("data-d2-rs-sketch-set").is_some() {
                actual_order.push("rough");
                let expected = expected_paths[rough_index].attribute("d").unwrap();
                let actual_ops: Vec<_> =
                    svgtypes::PathParser::from(primitive.attribute("d").unwrap())
                        .collect::<Result<_, _>>()
                        .unwrap();
                let source_ops: Vec<_> = svgtypes::PathParser::from(expected)
                    .collect::<Result<_, _>>()
                    .unwrap();
                assert_eq!(actual_ops, source_ops);
                assert_eq!(primitive.attribute("transform"), Some("translate(10 20)"));
                assert_eq!(primitive.attribute("fill"), Some("#ffffff"));
                assert_eq!(primitive.attribute("stroke"), Some("#112233"));
                assert_eq!(primitive.attribute("stroke-linecap"), Some("butt"));
                rough_index += 1;
            } else if primitive.attribute("class") == Some("dots-overlay") {
                actual_order.push("dots-overlay");
            } else {
                actual_order.push("sketch-streak-overlay");
                assert_eq!(node.attribute("style"), Some("mix-blend-mode:darken"));
                let texture_rect = node.descendants().find(|n| n.has_tag_name("rect")).unwrap();
                assert_eq!(texture_rect.attribute("x"), Some("0"));
                assert_eq!(
                    texture_rect
                        .parent_element()
                        .unwrap()
                        .attribute("transform"),
                    Some("translate(10 20)")
                );
            }
        }
        assert_eq!(serde_json::json!(actual_order), source["paint_order"]);
        let texture = document
            .descendants()
            .find(|n| n.tag_name().name() == "pattern")
            .unwrap();
        let paths: Vec<_> = texture.children().filter(|n| n.is_element()).collect();
        let expected_pattern = expected_doc
            .descendants()
            .find(|n| n.has_tag_name("pattern"))
            .unwrap();
        let expected_texture = expected_pattern
            .children()
            .filter(|n| n.is_element())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), expected_texture.len());
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].attribute("d"), expected_texture[0].attribute("d"));
        let source_alpha = expected_texture[0]
            .attribute("fill")
            .unwrap()
            .rsplit(',')
            .next()
            .unwrap()
            .trim()
            .trim_end_matches(')')
            .parse::<f64>()
            .unwrap();
        assert_eq!(
            paths[0]
                .attribute("fill-opacity")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
            source_alpha
        );
        assert_eq!(paths[0].attribute("fill-rule"), Some("evenodd"));
    }
    #[test]
    fn rectangle_streaks_keep_original_categories_masks_and_empty_fill_rules() {
        for (fill, category, blend, ink, alpha) in [
            ("#ffffff", "bright", "darken", "#000000", 26),
            ("#aaaaaa", "normal", "color-burn", "#000000", 41),
            ("#666666", "dark", "overlay", "#000000", 82),
            ("#111111", "darker", "lighten", "#ffffff", 61),
        ] {
            let style = streak_style(fill).unwrap();
            assert_eq!(
                (style.category, style.blend, style.ink, style.alpha),
                (category, blend, ink, alpha)
            );
        }
        let rect = Rect {
            origin: Point::default(),
            size: Size {
                width: 140.0,
                height: 80.0,
            },
        };
        let mut categories = BTreeSet::new();
        for fill in ["#ffffff", "#eeeeee", "none"] {
            let mut svg = String::new();
            draw_rectangle(
                &mut svg,
                ShapeKind::Rectangle,
                rect,
                &SourceNodeStyle {
                    fill: Some(fill.into()),
                    ..Default::default()
                },
                PaletteView::new(theme::palette(0).unwrap(), &BTreeMap::new()),
                Some("label-mask"),
                &mut categories,
                false,
            );
            let source = format!("<svg>{svg}</svg>");
            let document = roxmltree::Document::parse(&source).unwrap();
            let paints: Vec<_> = document
                .root_element()
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() != "defs")
                .collect();
            assert_eq!(paints.len(), if fill == "none" { 2 } else { 3 });
            assert!(
                paints
                    .iter()
                    .all(|n| n.attribute("mask") == Some("url(#label-mask)"))
            );
            assert_eq!(svg.contains("<pattern"), fill == "#ffffff");
        }
    }
    use std::collections::BTreeMap;
}

#[path = "sketch_typed.rs"]
mod typed;
pub(crate) use typed::draw_typed;
