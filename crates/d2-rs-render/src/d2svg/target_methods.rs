// SPDX-License-Identifier: MPL-2.0
// Port of the d2target/d2target.go methods d2svg renders with: BoundingBox,
// GetCorpus, label/icon positions, CSSStyle, GetFontColor, Arrowhead
// Dimensions, GetIconSize and the exported constants.

use super::geo::{self, Point};
use super::label::{self, Position};
use super::shape;
use super::svgpath::get_stroke_dash_attributes;
use crate::target::{TargetConnection, TargetDiagram, TargetShape};

pub(crate) const DEFAULT_ICON_SIZE: i64 = 32;
pub(crate) const MAX_ICON_SIZE: i64 = 64;
pub(crate) const SHADOW_SIZE_X: i64 = 3;
pub(crate) const SHADOW_SIZE_Y: i64 = 5;
pub(crate) const THREE_DEE_OFFSET: i64 = 15;
pub(crate) const MULTIPLE_OFFSET: i64 = 10;
pub(crate) const INNER_BORDER_OFFSET: i64 = 5;
pub(crate) const BG_COLOR: &str = "N7";
pub(crate) const FG_COLOR: &str = "N1";
pub(crate) const MIN_ARROWHEAD_STROKE_WIDTH: f64 = 2.0;
pub(crate) const ARROWHEAD_PADDING: f64 = 2.0;
pub(crate) const CONNECTION_ICON_LABEL_GAP: i64 = 8;

/// d2target.Point arithmetic is int; Go `int(math.Ceil(float64(n)/2.))`.
fn half_ceil(n: i64) -> i64 {
    (n as f64 / 2.0).ceil() as i64
}

impl TargetShape {
    /// Shape.CSSStyle.
    pub(crate) fn css_style(&self) -> String {
        let mut out = format!("stroke-width:{};", self.stroke_width);
        if self.stroke_dash.0 != 0.0 {
            let (dash, gap) =
                get_stroke_dash_attributes(self.stroke_width as f64, self.stroke_dash.0);
            out += &format!("stroke-dasharray:{dash:.6},{gap:.6};");
        }
        out
    }

    /// Shape.GetFontColor.
    pub(crate) fn get_font_color(&self) -> String {
        if self.shape_type == "class" || self.shape_type == "sql_table" {
            if !super::color::is_theme_color(&self.color) {
                return self.color.clone();
            }
            return self.stroke.clone();
        }
        if !self.color.is_empty() {
            return self.color.clone();
        }
        "N1".to_owned()
    }

    pub(crate) fn label_position_str(&self) -> &str {
        self.label_position.as_deref().unwrap_or("")
    }

    pub(crate) fn tooltip_position_str(&self) -> &str {
        self.tooltip_position.as_deref().unwrap_or("")
    }

    pub(crate) fn fill_pattern_str(&self) -> &str {
        self.fill_pattern.as_deref().unwrap_or("")
    }

    pub(crate) fn pretty_link_str(&self) -> &str {
        self.pretty_link.as_deref().unwrap_or("")
    }

    pub(crate) fn top_left(&self) -> Point {
        Point::new(self.pos.x as f64, self.pos.y as f64)
    }
}

impl TargetConnection {
    /// Connection.CSSStyle.
    pub(crate) fn css_style(&self) -> String {
        let mut out = format!("stroke-width:{};", self.stroke_width);
        let mut stroke_dash = self.stroke_dash.0;
        if stroke_dash == 0.0 && self.animated {
            stroke_dash = 5.0;
        }
        if stroke_dash != 0.0 {
            let (dash, gap) = get_stroke_dash_attributes(self.stroke_width as f64, stroke_dash);
            out += &format!("stroke-dasharray:{dash:.6},{gap:.6};");
            if self.animated {
                let mut dash_offset = -10.0;
                if self.src_arrow != "none" && self.dst_arrow == "none" {
                    dash_offset = 10.0;
                }
                out += &format!("stroke-dashoffset:{:.6};", dash_offset * (dash + gap));
                out += &format!("animation: dashdraw {:.6}s linear infinite;", gap * 0.5);
            }
        }
        out
    }

    /// Connection.GetFontColor.
    pub(crate) fn get_font_color(&self) -> String {
        if !self.color.is_empty() {
            return self.color.clone();
        }
        "N1".to_owned()
    }

    pub(crate) fn route_points(&self) -> Vec<Point> {
        self.route
            .iter()
            .flatten()
            .map(|point| match point {
                Some(point) => Point::new(point.x.0, point.y.0),
                // Go dereferences nil route points, panicking; renderers
                // reject such targets before painting.
                None => Point::new(0.0, 0.0),
            })
            .collect()
    }

    pub(crate) fn fill_str(&self) -> &str {
        self.fill.as_deref().unwrap_or("")
    }

    /// Connection.GetLabelTopLeft.
    pub(crate) fn get_label_top_left(&self) -> Point {
        Position::from_string(&self.label_position)
            .get_point_on_route(
                &self.route_points(),
                self.stroke_width as f64,
                self.label_percentage.0,
                self.label_width as f64,
                self.label_height as f64,
            )
            .0
    }

    /// Connection.GetIconPosition.
    pub(crate) fn get_icon_position(&self) -> Option<Point> {
        self.icon.as_ref()?;
        if !self.label.is_empty() {
            let tl = self.get_label_top_left();
            return Some(Point::new(
                tl.x - CONNECTION_ICON_LABEL_GAP as f64 - DEFAULT_ICON_SIZE as f64,
                tl.y + self.label_height as f64 / 2.0 - (DEFAULT_ICON_SIZE / 2) as f64,
            ));
        }
        Some(
            Position::from_string(self.icon_position.as_deref().unwrap_or(""))
                .get_point_on_route(
                    &self.route_points(),
                    self.stroke_width as f64,
                    -1.0,
                    DEFAULT_ICON_SIZE as f64,
                    DEFAULT_ICON_SIZE as f64,
                )
                .0,
        )
    }

    /// Connection.GetArrowheadLabelPosition.
    pub(crate) fn get_arrowhead_label_position(&self, is_dst: bool) -> Point {
        let text = if is_dst {
            &self.dst_label
        } else {
            &self.src_label
        };
        let text = text.as_ref().expect("arrowhead label");
        let (width, height) = (text.label_width as f64, text.label_height as f64);
        let route = self.route_points();
        let index = if is_dst { route.len() - 2 } else { 0 };
        let (start, end) = (route[index], route[index + 1]);
        let (normal_x, normal_y) = geo::get_unit_normal_vector(end.x, end.y, start.x, start.y);
        // Go ARM64 fuses the second product into the sum.
        let shift = normal_y.abs().mul_add(
            width / 2.0 + label::PADDING,
            normal_x.abs() * (height / 2.0 + label::PADDING),
        );
        let length = geo::route_length(&route);
        let position = if is_dst {
            if length > 0.0 {
                1.0 - shift / length
            } else {
                1.0
            }
        } else if length > 0.0 {
            shift / length
        } else {
            0.0
        };
        let stroke_width = self.stroke_width as f64;
        let (mut label_tl, _) =
            Position::UnlockedTop.get_point_on_route(&route, stroke_width, position, width, height);
        let mut arrow_size = 0.0;
        if is_dst && self.dst_arrow != "none" {
            arrow_size = arrowhead_dimensions(&self.dst_arrow, stroke_width).1;
        } else if self.src_arrow != "none" {
            arrow_size = arrowhead_dimensions(&self.src_arrow, stroke_width).1;
        }
        if arrow_size > 0.0 {
            let offset =
                (arrow_size / 2.0 + ARROWHEAD_PADDING) - stroke_width / 2.0 - label::PADDING;
            if offset > 0.0 {
                label_tl.x = normal_x.mul_add(offset, label_tl.x);
                label_tl.y = normal_y.mul_add(offset, label_tl.y);
            }
        }
        label_tl
    }
}

/// Arrowhead.Dimensions (the base + width*multiplier sums fuse on ARM64).
pub(crate) fn arrowhead_dimensions(arrowhead: &str, stroke_width: f64) -> (f64, f64) {
    let (base_width, base_height, width_multiplier, height_multiplier) = match arrowhead {
        "arrow" => (4.0, 4.0, 4.0, 4.0),
        "triangle" => (4.0, 4.0, 3.0, 4.0),
        "unfilled-triangle" => (7.0, 7.0, 3.0, 4.0),
        "line" => (0.0, 0.0, 5.0, 8.0),
        "filled-diamond" => (11.0, 7.0, 5.5, 3.5),
        "diamond" => (11.0, 9.0, 5.5, 4.5),
        "cross" => (7.0, 7.0, 5.0, 5.0),
        "filled-circle" | "circle" => (8.0, 8.0, 5.0, 5.0),
        "filled-box" | "box" => (6.0, 6.0, 5.0, 5.0),
        "cf-one" | "cf-many" | "cf-one-required" | "cf-many-required" => (9.0, 9.0, 4.5, 4.5),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    let clipped = MIN_ARROWHEAD_STROKE_WIDTH.max(stroke_width);
    (
        clipped.mul_add(width_multiplier, base_width),
        clipped.mul_add(height_multiplier, base_height),
    )
}

/// d2target.GetIconSize.
pub(crate) fn get_icon_size(b: &geo::Box, position: &str) -> i64 {
    let icon_position = Position::from_string(position);
    let min_dimension = b.width.min(b.height) as i64;
    let half_min_dimension = (0.5 * min_dimension as f64).ceil() as i64;
    let mut size = if icon_position == Position::InsideMiddleCenter {
        half_min_dimension
    } else {
        min_dimension.min(DEFAULT_ICON_SIZE.max(half_min_dimension))
    };
    size = size.min(MAX_ICON_SIZE);
    if !icon_position.is_outside() {
        let padding = label::PADDING as i64;
        size = size.min(
            ((b.width as i64) - 2 * padding)
                .max(0)
                .min(((b.height as i64) - 2 * padding).max(0)),
        );
    }
    size
}

impl TargetDiagram {
    pub(crate) fn shapes_slice(&self) -> &[TargetShape] {
        self.shapes.as_deref().unwrap_or(&[])
    }

    pub(crate) fn connections_slice(&self) -> &[TargetConnection] {
        self.connections.as_deref().unwrap_or(&[])
    }

    /// Diagram.HashID.
    pub(crate) fn hash_id(&self, salt: Option<&str>) -> Result<String, String> {
        let bytes = self.hash_input().map_err(|error| error.to_string())?;
        Ok(crate::target_hash_id_from_input(&bytes, salt))
    }

    /// Diagram.BoundingBox.
    pub(crate) fn go_bounding_box(&self) -> Result<((i64, i64), (i64, i64)), String> {
        let shapes = self.shapes_slice();
        if shapes.is_empty() {
            return Ok(((0, 0), (0, 0)));
        }
        let (mut x1, mut y1) = (i64::from(i32::MAX), i64::from(i32::MAX));
        let (mut x2, mut y2) = (i64::from(i32::MIN), i64::from(i32::MIN));
        for s in shapes {
            let half = half_ceil(s.stroke_width);
            x1 = x1.min(s.pos.x - half);
            y1 = y1.min(s.pos.y - half);
            x2 = x2.max(s.pos.x + s.width + half);
            y2 = y2.max(s.pos.y + s.height + half);
            if s.shape_type == "c4-person" {
                let head_radius = (s.width as f64 * 0.22) as i64;
                let head_center_y = (s.height as f64 * 0.18) as i64;
                let head_top = s.pos.y + head_center_y - head_radius;
                y1 = y1.min(head_top - s.stroke_width);
            }
            if !s.tooltip.is_empty() || !s.link.is_empty() {
                if !s.tooltip_position_str().is_empty() {
                    let (min_x, min_y, max_x, max_y) = super::tooltip::calculate_bounds(
                        s,
                        self.font_family.as_deref(),
                        self.mono_font_family.as_deref(),
                    );
                    x1 = x1.min(min_x);
                    y1 = y1.min(min_y);
                    x2 = x2.max(max_x);
                    y2 = y2.max(max_y);
                } else {
                    y1 = y1.min(s.pos.y - s.stroke_width - 16);
                    x2 = x2.max(s.pos.x + s.stroke_width + s.width + 16);
                }
            }
            if s.shadow {
                y2 = y2.max(s.pos.y + s.height + half + SHADOW_SIZE_Y);
                x2 = x2.max(s.pos.x + s.width + half + SHADOW_SIZE_X);
            }
            if s.three_dee {
                let offset_y = if s.shape_type == "hexagon" {
                    THREE_DEE_OFFSET / 2
                } else {
                    THREE_DEE_OFFSET
                };
                y1 = y1.min(s.pos.y - offset_y - s.stroke_width);
                x2 = x2.max(s.pos.x + THREE_DEE_OFFSET + s.width + s.stroke_width);
            }
            if s.multiple {
                y1 = y1.min(s.pos.y - MULTIPLE_OFFSET - s.stroke_width);
                x2 = x2.max(s.pos.x + MULTIPLE_OFFSET + s.width + s.stroke_width);
            }
            if s.icon.is_some() && Position::from_string(&s.icon_position).is_outside() {
                let content_box =
                    geo::Box::new(Point::new(0.0, 0.0), s.width as f64, s.height as f64);
                // BoundingBox passes the DSL type, not DSL_SHAPE_TO_SHAPE_TYPE.
                let shape = shape::new_shape(&s.shape_type, content_box);
                let size = get_icon_size(&shape.get_inner_box(), &s.icon_position);
                let padding = label::PADDING as i64;
                if s.icon_position.starts_with("OUTSIDE_TOP") {
                    y1 = y1.min(s.pos.y - padding - size);
                } else if s.icon_position.starts_with("OUTSIDE_BOTTOM") {
                    y2 = y2.max(s.pos.y + s.height + padding + size);
                } else if s.icon_position.starts_with("OUTSIDE_LEFT") {
                    x1 = x1.min(s.pos.x - padding - size);
                } else if s.icon_position.starts_with("OUTSIDE_RIGHT") {
                    x2 = x2.max(s.pos.x + s.width + padding + size);
                }
            }
            if !s.label.is_empty() {
                let position = Position::from_string(s.label_position_str());
                let shape = shape::new_shape(
                    shape::dsl_shape_to_shape_type(&s.shape_type),
                    geo::Box::new(s.top_left(), s.width as f64, s.height as f64),
                );
                let mut tl = position.get_point_on_box(
                    &shape.get_box(),
                    label::PADDING,
                    s.label_width as f64,
                    s.label_height as f64,
                );
                if s.three_dee {
                    let offset = if s.shape_type == "hexagon" {
                        THREE_DEE_OFFSET / 2
                    } else {
                        THREE_DEE_OFFSET
                    };
                    if s.label_position_str().starts_with("OUTSIDE_RIGHT") {
                        tl.x += offset as f64;
                    }
                    if s.label_position_str().starts_with("OUTSIDE_TOP") {
                        tl.y -= offset as f64;
                    }
                }
                x1 = x1.min(tl.x as i64);
                y1 = y1.min(tl.y as i64);
                x2 = x2.max(tl.x as i64 + s.label_width);
                y2 = y2.max(tl.y as i64 + s.label_height);
            }
        }
        for c in self.connections_slice() {
            let half = half_ceil(c.stroke_width);
            for point in c.route_points() {
                x1 = x1.min(point.x.floor() as i64 - half);
                y1 = y1.min(point.y.floor() as i64 - half);
                x2 = x2.max(point.x.ceil() as i64 + half);
                y2 = y2.max(point.y.ceil() as i64 + half);
            }
            if !c.label.is_empty() {
                let tl = c.get_label_top_left();
                x1 = x1.min(tl.x as i64);
                y1 = y1.min(tl.y as i64);
                x2 = x2.max(tl.x as i64 + c.label_width);
                y2 = y2.max(tl.y as i64 + c.label_height);
            }
            for (is_dst, text) in [(false, &c.src_label), (true, &c.dst_label)] {
                if let Some(text) = text.as_ref().filter(|text| !text.label.is_empty()) {
                    let tl = c.get_arrowhead_label_position(is_dst);
                    x1 = x1.min(tl.x as i64);
                    y1 = y1.min(tl.y as i64);
                    x2 = x2.max(tl.x as i64 + text.label_width);
                    y2 = y2.max(tl.y as i64 + text.label_height);
                }
            }
        }
        Ok(((x1, y1), (x2, y2)))
    }

    fn child_boards(&self) -> impl Iterator<Item = &TargetDiagram> {
        self.layers
            .iter()
            .flatten()
            .chain(self.scenarios.iter().flatten())
            .chain(self.steps.iter().flatten())
            .flatten()
    }

    /// Diagram.NestedBoundingBox.
    pub(crate) fn nested_go_bounding_box(&self) -> Result<((i64, i64), (i64, i64)), String> {
        let ((mut x1, mut y1), (mut x2, mut y2)) = self.go_bounding_box()?;
        for child in self.child_boards() {
            let ((cx1, cy1), (cx2, cy2)) = child.nested_go_bounding_box()?;
            x1 = x1.min(cx1);
            y1 = y1.min(cy1);
            x2 = x2.max(cx2);
            y2 = y2.max(cy2);
        }
        Ok(((x1, y1), (x2, y2)))
    }

    /// Diagram.GetNestedCorpus.
    pub(crate) fn get_nested_corpus(&self) -> Result<String, String> {
        let mut corpus = self.get_corpus()?;
        for child in self.child_boards() {
            corpus += &child.get_nested_corpus()?;
        }
        Ok(corpus)
    }

    /// Diagram.GetCorpus.
    pub(crate) fn get_corpus(&self) -> Result<String, String> {
        let mut corpus = String::new();
        let mut appendix_count = 0;
        for s in self.shapes_slice() {
            corpus += &s.label;
            if !s.tooltip.is_empty() {
                corpus += &s.tooltip;
                appendix_count += 1;
                corpus += &appendix_count.to_string();
            }
            if !s.link.is_empty() {
                corpus += &s.link;
                appendix_count += 1;
                corpus += &appendix_count.to_string();
            }
            corpus += s.pretty_link_str();
            if s.shape_type == "class" {
                for field in s.fields.iter().flatten() {
                    corpus += &field.name;
                    corpus += &field.type_text;
                    corpus += super::classtable::visibility_token(&field.visibility);
                }
                for method in s.methods.iter().flatten() {
                    corpus += &method.name;
                    corpus += &method.return_type;
                    corpus += super::classtable::visibility_token(&method.visibility);
                }
            }
            if s.shape_type == "sql_table" {
                for column in s.columns.iter().flatten() {
                    let abbr = super::classtable::constraint_abbr(column);
                    corpus += &column.name.label;
                    corpus += &column.type_text.label;
                    corpus += &abbr;
                    corpus += &abbr;
                }
            }
        }
        for c in self.connections_slice() {
            corpus += &c.label;
            if let Some(text) = &c.src_label {
                corpus += &text.label;
            }
            if let Some(text) = &c.dst_label {
                corpus += &text.label;
            }
        }
        if let Some(legend) = &self.legend {
            corpus += if legend.label.is_empty() {
                "Legend"
            } else {
                &legend.label
            };
            for s in legend.shapes.iter().flatten() {
                corpus += &s.label;
            }
            for c in legend.connections.iter().flatten() {
                corpus += &c.label;
            }
        }
        Ok(corpus)
    }
}
