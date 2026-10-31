// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/class.go and table.go, plus the
// d2target class/sql_table helpers (VisibilityToken, ConstraintAbbr).

use std::fmt::Write as _;

use super::element::{ThemableElement, Theme};
use super::geo::{self, Point};
use super::label::{self, Position};
use super::svgtext::{escape_text, format_float, svg_id};
use super::target_methods::get_icon_size;
use crate::target::{TargetShape, TargetSqlColumn};

const PREFIX_PADDING: f64 = 10.0;
const PREFIX_WIDTH: f64 = 20.0;
const NAME_PADDING: f64 = 10.0;
const TYPE_PADDING: f64 = 20.0;
const HEADER_PADDING: f64 = 10.0;

/// fmt `%v` of a float64 (strconv 'g' shortest), for the sizes d2svg prints.
fn go_v(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// ClassField/ClassMethod.VisibilityToken.
pub(crate) fn visibility_token(visibility: &str) -> &'static str {
    match visibility {
        "protected" => "#",
        "private" => "-",
        _ => "+",
    }
}

/// SQLColumn.ConstraintAbbr.
pub(crate) fn constraint_abbr(column: &TargetSqlColumn) -> String {
    column
        .constraint
        .iter()
        .flatten()
        .map(|c| match c.as_str() {
            "primary_key" => "PK",
            "foreign_key" => "FK",
            "unique" => "UNQ",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// table.go clipPathForBorderRadius.
pub(crate) fn clip_path_for_border_radius(diagram_hash: &str, s: &TargetShape) -> String {
    let f = format_float;
    let b = geo::Box::new(s.top_left(), s.width as f64, s.height as f64);
    let r = s.border_radius as f64;
    let (top_x, top_y) = (b.top_left.x + b.width, b.top_left.y);
    let (x, y) = (b.top_left.x, b.top_left.y);
    let mut out = format!(r#"<clipPath id="{diagram_hash}-{}">"#, svg_id(&s.id));
    write!(
        out,
        r#"<path d="M {} {} L {} {} S {} {} {} {} "#,
        f(x),
        f(y + r),
        f(x),
        f(y + r),
        f(x),
        f(y),
        f(x + r),
        f(y)
    )
    .unwrap();
    write!(
        out,
        "L {} {} L {} {} ",
        f(x + b.width - r),
        f(y),
        f(top_x - r),
        f(top_y)
    )
    .unwrap();
    write!(
        out,
        "S {} {} {} {} ",
        f(top_x),
        f(top_y),
        f(top_x),
        f(top_y + r)
    )
    .unwrap();
    write!(out, "L {} {} ", f(top_x), f(top_y + b.height - r)).unwrap();
    if s.columns.as_ref().is_some_and(|c| !c.is_empty()) {
        write!(
            out,
            "L {} {} L {} {}",
            f(top_x),
            f(top_y + b.height),
            f(x),
            f(y + b.height)
        )
        .unwrap();
    } else {
        write!(
            out,
            "S {}  {} {} {} ",
            f(top_x),
            f(top_y + b.height),
            f(top_x - r),
            f(top_y + b.height)
        )
        .unwrap();
        write!(out, "L {} {} ", f(x + r), f(y + b.height)).unwrap();
        write!(
            out,
            "S {} {} {} {}",
            f(x),
            f(y + b.height),
            f(x),
            f(y + b.height - r)
        )
        .unwrap();
        write!(out, "L {} {}", f(x), f(y + r)).unwrap();
    }
    write!(out, r#"Z {} {}" "#, f(x), f(y)).unwrap();
    out + r#"fill="none" /> </clipPath>"#
}

fn header_rect(diagram_hash: &str, s: &TargetShape, b: &geo::Box, theme: Option<&Theme>) -> String {
    let mut rect = ThemableElement::new("rect", theme);
    rect.x = b.top_left.x;
    rect.y = b.top_left.y;
    rect.width = b.width;
    rect.height = b.height;
    rect.fill.clone_from(&s.fill);
    rect.fill_pattern = s.fill_pattern_str().to_owned();
    rect.class_name = "class_header".into();
    if s.border_radius != 0 {
        // borderRadiusClipPathID: the reference uses the clipPath's encoded id.
        rect.clip_path = format!("{diagram_hash}-{}", svg_id(&s.id));
    }
    rect.render()
}

fn icon(w: &mut String, s: &TargetShape, b: &geo::Box) -> Result<(), String> {
    if let Some(url) = &s.icon {
        let size = get_icon_size(b, &s.icon_position);
        let tl = Position::from_string(&s.icon_position).get_point_on_box(
            b,
            label::PADDING,
            size as f64,
            size as f64,
        );
        write!(
            w,
            r#"<image href="{}" x="{}" y="{}" width="{size}" height="{size}" />"#,
            html_escape(&crate::target::url(url)),
            format_float(tl.x),
            format_float(tl.y)
        )
        .unwrap();
    }
    Ok(())
}

/// html.EscapeString.
pub(crate) fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '"' => out.push_str("&#34;"),
            c => out.push(c),
        }
    }
    out
}

/// class.go drawClass.
pub(crate) fn draw_class(
    w: &mut String,
    diagram_hash: &str,
    s: &TargetShape,
    theme: Option<&Theme>,
) -> Result<(), String> {
    let (fill, stroke) = (s.stroke.clone(), s.fill.clone());
    let mut el = ThemableElement::new("rect", theme);
    el.x = s.pos.x as f64;
    el.y = s.pos.y as f64;
    el.width = s.width as f64;
    el.height = s.height as f64;
    el.fill = fill;
    el.stroke = stroke;
    el.fill_pattern = s.fill_pattern_str().to_owned();
    el.style = s.css_style();
    if s.border_radius != 0 {
        el.rx = s.border_radius as f64;
        el.ry = s.border_radius as f64;
    }
    w.push_str(&el.render());
    let b = geo::Box::new(s.top_left(), s.width as f64, s.height as f64);
    let fields = s.fields.as_deref().unwrap_or(&[]);
    let methods = s.methods.as_deref().unwrap_or(&[]);
    let row_height = b.height / (2 + fields.len() + methods.len()) as f64;
    let header = geo::Box::new(
        b.top_left,
        b.width,
        (2.0 * row_height).max(s.label_height as f64 + 2.0 * label::PADDING),
    );
    w.push_str(&header_rect(diagram_hash, s, &header, theme));
    let font_size = s.font_size as f64;
    if !s.label.is_empty() {
        let (text_width, text_height) = (s.label_width as f64, s.label_height as f64);
        let tl =
            Position::InsideMiddleCenter.get_point_on_box(&header, 0.0, text_width, text_height);
        let mut text = ThemableElement::new("text", theme);
        text.x = tl.x + text_width / 2.;
        text.y = tl.y + font_size;
        text.fill = s.get_font_color();
        text.class_name = "text-mono".into();
        text.style = format!("text-anchor:middle;font-size:{}px;", go_v(4. + font_size));
        text.content = super::render::render_text(&s.label, text.x, text_height);
        w.push_str(&text.render());
    }
    let mut row = geo::Box::new(b.top_left, b.width, row_height);
    row.top_left.y += header.height;
    for field in fields {
        w.push_str(&class_row(
            s,
            &row,
            visibility_token(&field.visibility),
            &field.name,
            &field.type_text,
            font_size,
            field.underline,
            theme,
        ));
        row.top_left.y += row_height;
    }
    let mut line = ThemableElement::new("line", theme);
    let r = s.border_radius as f64;
    if s.border_radius != 0 && methods.is_empty() {
        (line.x1, line.y1) = (row.top_left.x + r, row.top_left.y);
        (line.x2, line.y2) = (row.top_left.x + row.width - r, row.top_left.y);
    } else {
        (line.x1, line.y1) = (row.top_left.x, row.top_left.y);
        (line.x2, line.y2) = (row.top_left.x + row.width, row.top_left.y);
    }
    line.stroke.clone_from(&s.fill);
    line.style = "stroke-width:1".into();
    w.push_str(&line.render());
    for method in methods {
        w.push_str(&class_row(
            s,
            &row,
            visibility_token(&method.visibility),
            &method.name,
            &method.return_type,
            font_size,
            method.underline,
            theme,
        ));
        row.top_left.y += row_height;
    }
    icon(w, s, &b)
}

/// class.go classRow.
#[allow(clippy::too_many_arguments)]
fn class_row(
    s: &TargetShape,
    b: &geo::Box,
    prefix: &str,
    name: &str,
    type_text: &str,
    font_size: f64,
    underline: bool,
    theme: Option<&Theme>,
) -> String {
    let prefix_tl =
        Position::InsideMiddleLeft.get_point_on_box(b, PREFIX_PADDING, b.width, font_size);
    let type_tr = Position::InsideMiddleRight.get_point_on_box(b, TYPE_PADDING, 0.0, font_size);
    let mut text = ThemableElement::new("text", theme);
    text.x = prefix_tl.x;
    text.y = prefix_tl.y + font_size * 3. / 4.;
    text.fill.clone_from(&s.primary_accent_color);
    text.class_name = "text-mono".into();
    text.style = format!("text-anchor:start;font-size:{}px", go_v(font_size));
    text.content = prefix.to_owned();
    let mut out = text.render();
    text.x = prefix_tl.x + PREFIX_WIDTH;
    text.fill.clone_from(&s.fill);
    text.class_name = "text-mono".into();
    if underline {
        text.class_name += " text-underline";
    }
    text.content = escape_text(name);
    out += &text.render();
    text.x = type_tr.x;
    text.y = type_tr.y + font_size * 3. / 4.;
    text.fill.clone_from(&s.secondary_accent_color);
    text.class_name = "text-mono".into();
    text.style = format!("text-anchor:end;font-size:{}px", go_v(font_size));
    text.content = escape_text(type_text);
    out += &text.render();
    out
}

/// table.go drawTable.
pub(crate) fn draw_table(
    w: &mut String,
    diagram_hash: &str,
    s: &TargetShape,
    theme: Option<&Theme>,
) -> Result<(), String> {
    let mut rect = ThemableElement::new("rect", theme);
    rect.x = s.pos.x as f64;
    rect.y = s.pos.y as f64;
    rect.width = s.width as f64;
    rect.height = s.height as f64;
    rect.fill.clone_from(&s.stroke);
    rect.stroke.clone_from(&s.fill);
    rect.fill_pattern = s.fill_pattern_str().to_owned();
    rect.class_name = "shape".into();
    rect.style = s.css_style();
    if s.border_radius != 0 {
        rect.rx = s.border_radius as f64;
        rect.ry = s.border_radius as f64;
    }
    w.push_str(&rect.render());
    let b = geo::Box::new(s.top_left(), s.width as f64, s.height as f64);
    let columns = s.columns.as_deref().unwrap_or(&[]);
    let row_height = b.height / (1 + columns.len()) as f64;
    let header = geo::Box::new(b.top_left, b.width, row_height);
    w.push_str(&header_rect(diagram_hash, s, &header, theme));
    let font_size = s.font_size as f64;
    if !s.label.is_empty() {
        let text_height = s.label_height as f64;
        let tl = Position::InsideMiddleLeft.get_point_on_box(
            &header,
            HEADER_PADDING,
            s.width as f64,
            text_height,
        );
        let mut text = ThemableElement::new("text", theme);
        text.x = tl.x;
        text.y = tl.y + text_height * 3. / 4.;
        text.fill = s.get_font_color();
        text.class_name = "text".into();
        text.style = format!("text-anchor:start;font-size:{}px", go_v(4. + font_size));
        text.content = escape_text(&s.label);
        w.push_str(&text.render());
    }
    let longest_name = columns
        .iter()
        .map(|c| c.name.label_width)
        .max()
        .unwrap_or(0)
        .max(0) as f64;
    let mut row = geo::Box::new(Point::new(b.top_left.x, b.top_left.y), b.width, row_height);
    row.top_left.y += header.height;
    for (index, column) in columns.iter().enumerate() {
        w.push_str(&table_row(
            s,
            &row,
            &column.name.label,
            &column.type_text.label,
            &constraint_abbr(column),
            font_size,
            longest_name,
            theme,
        ));
        row.top_left.y += row_height;
        let mut line = ThemableElement::new("line", theme);
        let r = s.border_radius as f64;
        if index == columns.len() - 1 && s.border_radius != 0 {
            (line.x1, line.y1) = (row.top_left.x + r, row.top_left.y);
            (line.x2, line.y2) = (row.top_left.x + row.width - r, row.top_left.y);
        } else {
            (line.x1, line.y1) = (row.top_left.x, row.top_left.y);
            (line.x2, line.y2) = (row.top_left.x + row.width, row.top_left.y);
        }
        line.stroke.clone_from(&s.fill);
        line.style = "stroke-width:2".into();
        w.push_str(&line.render());
    }
    icon(w, s, &b)
}

/// table.go tableRow.
#[allow(clippy::too_many_arguments)]
fn table_row(
    s: &TargetShape,
    b: &geo::Box,
    name: &str,
    type_text: &str,
    constraint: &str,
    font_size: f64,
    longest_name: f64,
    theme: Option<&Theme>,
) -> String {
    let name_tl = Position::InsideMiddleLeft.get_point_on_box(b, NAME_PADDING, 0.0, font_size);
    let mut text = ThemableElement::new("text", theme);
    text.x = name_tl.x;
    text.y = name_tl.y + font_size * 3. / 4.;
    text.fill.clone_from(&s.primary_accent_color);
    text.class_name = "text".into();
    text.style = format!("text-anchor:start;font-size:{}px", go_v(font_size));
    text.content = escape_text(name);
    let mut out = text.render();
    text.x += longest_name + TYPE_PADDING;
    text.fill.clone_from(&s.neutral_accent_color);
    text.content = escape_text(type_text);
    out += &text.render();
    text.x = b.top_left.x + (b.width - NAME_PADDING);
    text.fill.clone_from(&s.secondary_accent_color);
    text.style = format!("text-anchor:end;font-size:{}px", go_v(font_size));
    text.set_text(constraint);
    out += &text.render();
    out
}
