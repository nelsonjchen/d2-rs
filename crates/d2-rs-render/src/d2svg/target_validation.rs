// SPDX-License-Identifier: MPL-2.0
//! Port of d2target/render_validation.go at master 6a7b48378. Validate the
//! entire owned target tree before recursive hashing or geometry evaluation.
use super::{label::Position, svgpath};
use crate::target::{TargetConnection, TargetDiagram, TargetShape, TargetText};
use d2_rs_core::quote_go_bytes as quote;

fn invalid(path: &str, field: &str, value: impl std::fmt::Display, requirement: &str) -> String {
    format!("render target {path}.{field} has invalid value {value}: {requirement}")
}

fn number(value: f64) -> String {
    // fmt's default %v float verb is %g, unlike SVG's fixed decimal spelling.
    super::geo::go_format_v(value)
}

fn font(family: Option<&str>, path: &str) -> Result<(), String> {
    if let Some(family) = family.filter(|f| !f.is_empty()) {
        if !d2_rs_text::d2fonts::is_registered_family(family) {
            return Err(format!(
                "render target {path} {} is not a registered font family",
                quote(family.as_bytes())
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate(root: &TargetDiagram) -> Result<(), String> {
    let mut stack = vec![(root, "root".to_owned(), 0usize)];
    let mut board_count = 1;
    // TargetDiagram owns its children by value: cycles and reused allocations
    // cannot be constructed. JSON null children must still be rejected.
    while let Some((diagram, path, depth)) = stack.pop() {
        font(
            diagram.font_family.as_deref(),
            &format!("{path}.fontFamily"),
        )?;
        font(
            diagram.mono_font_family.as_deref(),
            &format!("{path}.monoFontFamily"),
        )?;
        shape_numbers(&diagram.root, &format!("{path}.root"))?;
        for (i, shape) in diagram.shapes_slice().iter().enumerate() {
            validate_shape(shape, &format!("{path}.shapes[{i}]"))?;
        }
        for (i, connection) in diagram.connections_slice().iter().enumerate() {
            validate_connection(connection, &format!("{path}.connections[{i}]"))?;
        }
        if let Some(legend) = &diagram.legend {
            for (i, shape) in legend.shapes.iter().flatten().enumerate() {
                let mut projected = shape.clone();
                projected.pos = Default::default();
                projected.width = 1;
                projected.height = 1;
                projected.label.clear();
                projected.font_size = 0;
                projected.label_width = 0;
                projected.label_height = 0;
                validate_shape(&projected, &format!("{path}.legend.shapes[{i}]"))?;
            }
            for (i, c) in legend.connections.iter().flatten().enumerate() {
                let path = format!("{path}.legend.connections[{i}]");
                // Only these fields survive the source legend projection.
                let projected = TargetConnection {
                    stroke_width: c.stroke_width,
                    stroke_dash: c.stroke_dash,
                    border_radius: c.border_radius,
                    opacity: c.opacity,
                    ..Default::default()
                };
                connection_numbers(&projected, &path)?;
                arrow(&c.src_arrow, &format!("{path}.srcArrow"))?;
                arrow(&c.dst_arrow, &format!("{path}.dstArrow"))?;
            }
        }
        for (name, children) in [
            ("steps", &diagram.steps),
            ("scenarios", &diagram.scenarios),
            ("layers", &diagram.layers),
        ] {
            if let Some(children) = children {
                for (i, child) in children.iter().enumerate().rev() {
                    let path = format!("{path}.{name}[{i}]");
                    let child = child
                        .as_ref()
                        .ok_or_else(|| format!("render target board {path} is nil"))?;
                    if depth + 1 > 1024 {
                        return Err(format!(
                            "render target board tree exceeds depth 1024 at {path}"
                        ));
                    }
                    board_count += 1;
                    if board_count > 4096 {
                        return Err("render target board tree exceeds 4096 total boards".into());
                    }
                    stack.push((child, path, depth + 1));
                }
            }
        }
    }
    Ok(())
}

fn text_numbers(
    path: &str,
    prefix: &str,
    font_size: i64,
    width: i64,
    height: i64,
) -> Result<(), String> {
    for (field, value) in [
        ("fontSize", font_size),
        ("labelWidth", width),
        ("labelHeight", height),
    ] {
        if value < 0 {
            return Err(invalid(
                path,
                &format!("{prefix}{field}"),
                value,
                "must be non-negative",
            ));
        }
    }
    Ok(())
}

fn opacity(path: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(invalid(
            path,
            "opacity",
            number(value),
            "must be finite and within [0,1]",
        ));
    }
    Ok(())
}

fn dash(path: &str, value: f64, width: i64) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(invalid(
            path,
            "strokeDash",
            number(value),
            "must be finite and non-negative",
        ));
    }
    if value > 0.0 && width > 0 {
        let (dash, gap) = svgpath::get_stroke_dash_attributes(width as f64, value);
        if !dash.is_finite() || !gap.is_finite() || dash < 0.0 || gap < 0.0 {
            return Err(invalid(
                path,
                "strokeDash",
                number(value),
                "must produce finite non-negative dash lengths with strokeWidth",
            ));
        }
    }
    Ok(())
}

fn shape_numbers(s: &TargetShape, path: &str) -> Result<(), String> {
    for (field, value) in [
        ("width", s.width),
        ("height", s.height),
        ("strokeWidth", s.stroke_width),
        ("borderRadius", s.border_radius),
        ("iconBorderRadius", s.icon_border_radius),
    ] {
        if value < 0 {
            return Err(invalid(path, field, value, "must be non-negative"));
        }
    }
    opacity(path, s.opacity.0)?;
    dash(path, s.stroke_dash.0, s.stroke_width)?;
    if let Some(ratio) = s.content_aspect_ratio {
        if !ratio.0.is_finite() || ratio.0 <= 0.0 {
            return Err(invalid(
                path,
                "contentAspectRatio",
                number(ratio.0),
                "must be finite and greater than zero",
            ));
        }
    }
    text_numbers(path, "", s.font_size, s.label_width, s.label_height)?;
    let half_stroke = s.stroke_width / 2 + s.stroke_width % 2;
    for (axis, position, dimension) in [("pos.x", s.pos.x, s.width), ("pos.y", s.pos.y, s.height)] {
        if position.checked_sub(half_stroke).is_none() {
            return Err(invalid(
                path,
                axis,
                position,
                "must fit bounds arithmetic with strokeWidth",
            ));
        }
        let end = position.checked_add(dimension).ok_or_else(|| {
            invalid(
                path,
                axis,
                position,
                "must fit bounds arithmetic with its dimension",
            )
        })?;
        if end.checked_add(half_stroke).is_none() {
            return Err(invalid(
                path,
                axis,
                position,
                "must fit bounds arithmetic with strokeWidth",
            ));
        }
    }
    Ok(())
}

fn validate_shape(s: &TargetShape, path: &str) -> Result<(), String> {
    shape_numbers(s, path)?;
    if s.shape_type == "image" && s.icon.is_none() {
        return Err(format!("render target {path} image is missing an icon"));
    }
    Ok(())
}

fn arrow(value: &str, path: &str) -> Result<(), String> {
    if !matches!(
        value,
        "none"
            | "arrow"
            | "unfilled-triangle"
            | "triangle"
            | "line"
            | "filled-diamond"
            | "diamond"
            | "filled-circle"
            | "circle"
            | "cross"
            | "filled-box"
            | "box"
            | "cf-one"
            | "cf-many"
            | "cf-one-required"
            | "cf-many-required"
    ) {
        return Err(format!(
            "render target {path} uses unsupported arrowhead {}",
            quote(value.as_bytes())
        ));
    }
    Ok(())
}

fn endpoint_text(t: &TargetText, path: &str, prefix: &str) -> Result<(), String> {
    text_numbers(path, prefix, t.font_size, t.label_width, t.label_height)
}

fn connection_numbers(c: &TargetConnection, path: &str) -> Result<(), String> {
    if c.stroke_width < 0 {
        return Err(invalid(
            path,
            "strokeWidth",
            c.stroke_width,
            "must be non-negative",
        ));
    }
    opacity(path, c.opacity.0)?;
    dash(path, c.stroke_dash.0, c.stroke_width)?;
    for (field, value) in [
        ("borderRadius", c.border_radius.0),
        ("iconBorderRadius", c.icon_border_radius.0),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(invalid(
                path,
                field,
                number(value),
                "must be finite and non-negative",
            ));
        }
    }
    if !c.label_percentage.0.is_finite() || !(0.0..=1.0).contains(&c.label_percentage.0) {
        return Err(invalid(
            path,
            "labelPercentage",
            number(c.label_percentage.0),
            "must be finite and within [0,1]",
        ));
    }
    text_numbers(path, "", c.font_size, c.label_width, c.label_height)?;
    if let Some(t) = &c.src_label {
        endpoint_text(t, path, "srcLabel.")?;
    }
    if let Some(t) = &c.dst_label {
        endpoint_text(t, path, "dstLabel.")?;
    }
    Ok(())
}

fn validate_connection(c: &TargetConnection, path: &str) -> Result<(), String> {
    connection_numbers(c, path)?;
    arrow(&c.src_arrow, &format!("{path}.srcArrow"))?;
    arrow(&c.dst_arrow, &format!("{path}.dstArrow"))?;
    let route = c.route.as_deref().unwrap_or(&[]);
    if route.len() < 2 {
        return Err(format!(
            "render target {path}.route must contain at least two points"
        ));
    }
    if c.is_curve && (route.len() - 1) % 3 != 0 {
        return Err(format!(
            "render target {path}.route must contain 1+3n points for a cubic connection"
        ));
    }
    let mut previous = None;
    for (i, point) in route.iter().enumerate() {
        let point = point
            .as_ref()
            .ok_or_else(|| format!("render target {path}.route[{i}] is nil"))?;
        if !point.x.0.is_finite() || !point.y.0.is_finite() {
            return Err(format!(
                "render target {path}.route[{i}] must contain finite coordinates"
            ));
        }
        let half_stroke = (c.stroke_width as f64 / 2.0).ceil();
        for (field, value) in [("x", point.x.0), ("y", point.y.0)] {
            if value.floor() - half_stroke <= i64::MIN as f64
                || value.ceil() + half_stroke >= i64::MAX as f64
            {
                return Err(invalid(
                    &format!("{path}.route[{i}]"),
                    field,
                    number(value),
                    "must fit integer bounds arithmetic",
                ));
            }
        }
        if let Some((x, y)) = previous {
            let dx: f64 = point.x.0 - x;
            let dy: f64 = point.y.0 - y;
            // Pinned ARM64 render_validation.go:199 rounds dx*dx, then
            // FMADDD accumulates dy*dy (run68-asm/validation.log).
            let length_squared = dy.mul_add(dy, dx * dx);
            if !length_squared.is_finite() || length_squared == 0.0 {
                return Err(format!(
                    "render target {path}.route[{}:{i}] must have a finite, non-zero segment length",
                    i - 1
                ));
            }
        }
        previous = Some((point.x.0, point.y.0));
    }
    if !c.label.is_empty() {
        if !Position::from_string(&c.label_position).is_edge_position() {
            return Err(format!(
                "render target {path}.labelPosition {} is not a connection label position",
                quote(c.label_position.as_bytes())
            ));
        }
        let p = c.get_label_top_left();
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err(format!(
                "render target {path}.labelPosition does not resolve to a finite point on the route"
            ));
        }
    }
    for (name, text, is_dst) in [
        ("srcLabel", &c.src_label, false),
        ("dstLabel", &c.dst_label, true),
    ] {
        if text.as_ref().is_some_and(|t| !t.label.is_empty()) {
            let p = c.get_arrowhead_label_position(is_dst);
            if !p.x.is_finite() || !p.y.is_finite() {
                return Err(format!(
                    "render target {path}.{name} does not resolve to a finite point on the route"
                ));
            }
        }
    }
    Ok(())
}
