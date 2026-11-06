// SPDX-License-Identifier: MPL-2.0
//! Pinned master d2renderers/d2svg/paint_validation.go and lib/color.ValidColor.
use super::{color, render::RenderOpts};
use crate::target::{TargetConnection, TargetDiagram, TargetShape, TargetThemeOverrides};
use d2_rs_core::quote_go_bytes as quote;

const NAMED_COLORS: &str = concat!(
    "currentcolor transparent aliceblue antiquewhite aqua aquamarine azure beige bisque black ",
    "blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral ",
    "cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgrey ",
    "darkgreen darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon ",
    "darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink ",
    "deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ",
    "ghostwhite gold goldenrod gray grey green greenyellow honeydew hotpink indianred indigo ivory ",
    "khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan ",
    "lightgoldenrodyellow lightgray lightgrey lightgreen lightpink lightsalmon lightseagreen ",
    "lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen ",
    "magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen ",
    "mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream ",
    "mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid ",
    "palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum ",
    "powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown ",
    "seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen ",
    "steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen",
);

fn named_or_hex(value: &str) -> bool {
    let lower = String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap();
    NAMED_COLORS
        .split_ascii_whitespace()
        .any(|name| name == lower)
        || value.strip_prefix('#').is_some_and(|digits| {
            matches!(digits.len(), 3 | 6) && digits.bytes().all(|c| c.is_ascii_hexdigit())
        })
}

fn valid_color(value: &str) -> bool {
    if color::is_gradient(value) {
        color::parse_gradient(value).is_some_and(|g| {
            g.color_stops
                .iter()
                .all(|(c, _)| d2_rs_math::svg_color::parse(c).is_some())
        })
    } else {
        named_or_hex(value)
    }
}
fn paint(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value == "none" || color::is_theme_color(value) || valid_color(value) {
        return Ok(());
    }
    Err(format!(
        "{field} uses unsupported paint {}",
        quote(value.as_bytes())
    ))
}
fn pattern(value: &str, field: &str) -> Result<(), String> {
    let lower = String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap();
    if value.is_empty()
        || matches!(
            lower.as_str(),
            "none" | "dots" | "lines" | "grain" | "paper"
        )
    {
        return Ok(());
    }
    Err(format!(
        "{field} uses unsupported value {}",
        quote(value.as_bytes())
    ))
}
fn shape(s: &TargetShape, object: &str) -> Result<(), String> {
    for (name, value) in [
        ("fill", s.fill.as_str()),
        ("stroke", &s.stroke),
        ("font color", &s.color),
        ("label fill", s.label_fill.as_deref().unwrap_or("")),
        ("primary accent color", &s.primary_accent_color),
        ("secondary accent color", &s.secondary_accent_color),
        ("neutral accent color", &s.neutral_accent_color),
    ] {
        paint(value, &format!("{object} {name}"))?;
    }
    pattern(
        s.fill_pattern.as_deref().unwrap_or(""),
        &format!("{object} fill pattern"),
    )
}
fn connection(c: &TargetConnection, object: &str) -> Result<(), String> {
    for (name, value) in [
        ("stroke", c.stroke.as_str()),
        ("fill", c.fill.as_deref().unwrap_or("")),
        ("font color", &c.color),
    ] {
        paint(value, &format!("{object} {name}"))?;
    }
    if let Some(label) = &c.src_label {
        paint(&label.color, &format!("{object} source label color"))?;
    }
    if let Some(label) = &c.dst_label {
        paint(&label.color, &format!("{object} destination label color"))?;
    }
    Ok(())
}
pub(super) fn theme(overrides: Option<&TargetThemeOverrides>, object: &str) -> Result<(), String> {
    let Some(o) = overrides else {
        return Ok(());
    };
    for (name, value) in [
        ("N1", &o.n1),
        ("N2", &o.n2),
        ("N3", &o.n3),
        ("N4", &o.n4),
        ("N5", &o.n5),
        ("N6", &o.n6),
        ("N7", &o.n7),
        ("B1", &o.b1),
        ("B2", &o.b2),
        ("B3", &o.b3),
        ("B4", &o.b4),
        ("B5", &o.b5),
        ("B6", &o.b6),
        ("AA2", &o.aa2),
        ("AA4", &o.aa4),
        ("AA5", &o.aa5),
        ("AB4", &o.ab4),
        ("AB5", &o.ab5),
    ] {
        if let Some(value) = value {
            if !named_or_hex(value) {
                return Err(format!(
                    "{object} {name} uses unsupported color {}",
                    quote(value.as_bytes())
                ));
            }
        }
    }
    Ok(())
}
pub(super) fn validate(d: &TargetDiagram, opts: &RenderOpts) -> Result<(), String> {
    paint(&d.root.fill, "root fill")?;
    paint(&d.root.stroke, "root stroke")?;
    pattern(
        d.root.fill_pattern.as_deref().unwrap_or(""),
        "root fill pattern",
    )?;
    for s in d.shapes.iter().flatten() {
        shape(s, &format!("shape {}", quote(s.id.as_bytes())))?;
    }
    for c in d.connections.iter().flatten() {
        connection(c, &format!("connection {}", quote(c.id.as_bytes())))?;
    }
    if let Some(l) = &d.legend {
        for s in l.shapes.iter().flatten() {
            shape(s, &format!("legend shape {}", quote(s.id.as_bytes())))?;
        }
        for c in l.connections.iter().flatten() {
            let object = format!("legend connection {}", quote(c.id.as_bytes()));
            paint(&c.stroke, &format!("{object} stroke"))?;
            paint(c.fill.as_deref().unwrap_or(""), &format!("{object} fill"))?;
        }
    }
    theme(opts.theme_overrides.as_ref(), "theme override")?;
    theme(opts.dark_theme_overrides.as_ref(), "dark theme override")
}
pub(super) fn link(value: &str, object: &str) -> Result<(), String> {
    if d2_rs_text::markdown::link_safety::is_dangerous_link(value.as_bytes()) {
        return Err(format!("{object} uses an unsafe link URL scheme"));
    }
    Ok(())
}
pub(super) fn links(d: &TargetDiagram) -> Result<(), String> {
    for s in d.shapes.iter().flatten() {
        link(&s.link, &format!("shape {}", quote(s.id.as_bytes())))?;
    }
    for c in d.connections.iter().flatten() {
        link(&c.link, &format!("connection {}", quote(c.id.as_bytes())))?;
    }
    Ok(())
}
