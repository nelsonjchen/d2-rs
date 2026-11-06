// SPDX-License-Identifier: MPL-2.0
// Port of pinned master d2themes/{d2themes,element}.go.

use super::color;
use super::markup::{self as svg, Attribute, Fragment, Id};
use super::svgtext::format_float;
use crate::target::TargetThemeOverrides;

/// d2themes.ColorPalette with owned (overridable) colors.
#[derive(Clone, Debug, Default)]
pub struct Colors {
    pub n1: String,
    pub n2: String,
    pub n3: String,
    pub n4: String,
    pub n5: String,
    pub n6: String,
    pub n7: String,
    pub b1: String,
    pub b2: String,
    pub b3: String,
    pub b4: String,
    pub b5: String,
    pub b6: String,
    pub aa2: String,
    pub aa4: String,
    pub aa5: String,
    pub ab4: String,
    pub ab5: String,
}

/// d2themes.Theme. `Find` of an unknown ID yields the zero Theme.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub id: i64,
    pub colors: Colors,
    pub special_rules: d2_rs_core::theme::SpecialRules,
}

impl Theme {
    /// d2themescatalog.Find.
    pub(crate) fn find(id: i64) -> Self {
        let Some(palette) = d2_rs_core::theme::palette(id) else {
            return Self::default();
        };
        let n = palette.neutrals;
        Self {
            id,
            colors: Colors {
                n1: n.n1.into(),
                n2: n.n2.into(),
                n3: n.n3.into(),
                n4: n.n4.into(),
                n5: n.n5.into(),
                n6: n.n6.into(),
                n7: n.n7.into(),
                b1: palette.b1.into(),
                b2: palette.b2.into(),
                b3: palette.b3.into(),
                b4: palette.b4.into(),
                b5: palette.b5.into(),
                b6: palette.b6.into(),
                aa2: palette.aa2.into(),
                aa4: palette.aa4.into(),
                aa5: palette.aa5.into(),
                ab4: palette.ab4.into(),
                ab5: palette.ab5.into(),
            },
            special_rules: palette.special_rules,
        }
    }

    pub(crate) fn is_dark(&self) -> bool {
        (200..300).contains(&self.id)
    }

    pub(crate) fn apply_overrides(&mut self, overrides: Option<&TargetThemeOverrides>) {
        let Some(o) = overrides else { return };
        let c = &mut self.colors;
        for (slot, value) in [
            (&mut c.b1, &o.b1),
            (&mut c.b2, &o.b2),
            (&mut c.b3, &o.b3),
            (&mut c.b4, &o.b4),
            (&mut c.b5, &o.b5),
            (&mut c.b6, &o.b6),
            (&mut c.aa2, &o.aa2),
            (&mut c.aa4, &o.aa4),
            (&mut c.aa5, &o.aa5),
            (&mut c.ab4, &o.ab4),
            (&mut c.ab5, &o.ab5),
            (&mut c.n1, &o.n1),
            (&mut c.n2, &o.n2),
            (&mut c.n3, &o.n3),
            (&mut c.n4, &o.n4),
            (&mut c.n5, &o.n5),
            (&mut c.n6, &o.n6),
            (&mut c.n7, &o.n7),
        ] {
            if let Some(value) = value {
                slot.clone_from(value);
            }
        }
    }
}

/// d2themes.ResolveThemeColor.
pub(crate) fn resolve_theme_color<'a>(theme: &'a Theme, code: &'a str) -> &'a str {
    let c = &theme.colors;
    match code {
        "N1" => &c.n1,
        "N2" => &c.n2,
        "N3" => &c.n3,
        "N4" => &c.n4,
        "N5" => &c.n5,
        "N6" => &c.n6,
        "N7" => &c.n7,
        "B1" => &c.b1,
        "B2" => &c.b2,
        "B3" => &c.b3,
        "B4" => &c.b4,
        "B5" => &c.b5,
        "B6" => &c.b6,
        "AA2" => &c.aa2,
        "AA4" => &c.aa4,
        "AA5" => &c.aa5,
        "AB4" => &c.ab4,
        "AB5" => &c.ab5,
        _ => code,
    }
}

/// Go initializes every numeric attribute to math.MaxFloat64 ("unset").
const UNSET: f64 = f64::MAX;

/// d2themes.ThemableElement.
#[derive(Clone, Debug)]
pub struct ThemableElement<'t> {
    tag: String,
    pub x: f64,
    pub x1: f64,
    pub x2: f64,
    pub y: f64,
    pub y1: f64,
    pub y2: f64,
    pub width: f64,
    pub height: f64,
    pub r: f64,
    pub rx: f64,
    pub ry: f64,
    pub cx: f64,
    pub cy: f64,
    pub d: String,
    pub mask: String,
    pub points: String,
    pub transform: String,
    pub href: String,
    pub xmlns: String,
    pub fill: String,
    pub stroke: String,
    pub stroke_dash_array: String,
    pub background_color: String,
    pub color: String,
    pub class_name: String,
    pub style: String,
    pub attributes: String,
    pub content: String,
    pub clip_path: String,
    pub fill_pattern: String,
    inline_theme: Option<&'t Theme>,
    extra_attributes: Vec<Attribute>,
    inner: Option<Fragment>,
    mask_id: String,
    clip_path_id: String,
}

impl<'t> ThemableElement<'t> {
    pub fn new(tag: &str, inline_theme: Option<&'t Theme>) -> Self {
        Self {
            tag: tag.into(),
            x: UNSET,
            x1: UNSET,
            x2: UNSET,
            y: UNSET,
            y1: UNSET,
            y2: UNSET,
            width: UNSET,
            height: UNSET,
            r: UNSET,
            rx: UNSET,
            ry: UNSET,
            cx: UNSET,
            cy: UNSET,
            d: String::new(),
            mask: String::new(),
            points: String::new(),
            transform: String::new(),
            href: String::new(),
            xmlns: if tag == "div" {
                "http://www.w3.org/1999/xhtml".into()
            } else {
                String::new()
            },
            fill: String::new(),
            stroke: String::new(),
            stroke_dash_array: String::new(),
            background_color: String::new(),
            color: String::new(),
            class_name: String::new(),
            style: String::new(),
            attributes: String::new(),
            content: String::new(),
            clip_path: String::new(),
            fill_pattern: String::new(),
            inline_theme,
            extra_attributes: Vec::new(),
            inner: None,
            mask_id: String::new(),
            clip_path_id: String::new(),
        }
    }

    pub fn set_translate(&mut self, x: f64, y: f64) {
        self.transform = format!("translate({} {})", format_float(x), format_float(y));
    }

    pub fn set_mask_url(&mut self, url: &str) {
        self.set_mask_id(&svg::literal_id(url));
    }

    pub fn set_mask_id(&mut self, id: &Id) {
        self.mask_id = svg::local_iri(id);
    }

    pub fn set_clip_path_id(&mut self, id: &Id) {
        self.clip_path_id = id.as_str().into();
    }

    pub fn add_attributes(&mut self, attributes: impl IntoIterator<Item = Attribute>) {
        self.extra_attributes.extend(attributes);
    }

    pub fn set_text(&mut self, text: &str) {
        self.inner = Some(svg::text(text));
    }

    pub fn set_inner_svg(&mut self, content: Fragment) {
        self.inner = Some(content);
    }

    /// ThemableElement.Render. Go mutates Fill/Stroke for gradients while
    /// rendering; the pattern overlay copy is taken after that mutation.
    pub fn render(&self) -> String {
        let mut el = self.clone();
        el.render_mut()
    }

    fn render_mut(&mut self) -> String {
        let mut attributes = Vec::with_capacity(24 + self.extra_attributes.len());
        if !self.href.is_empty() {
            attributes.push(svg::attr("href", &self.href));
        }
        for (name, value) in [
            ("x", self.x),
            ("x1", self.x1),
            ("x2", self.x2),
            ("y", self.y),
            ("y1", self.y1),
            ("y2", self.y2),
            ("width", self.width),
            ("height", self.height),
            ("r", self.r),
        ] {
            if value != UNSET {
                attributes.push(svg::float_attr(name, value));
            }
        }
        if self.rx != UNSET {
            attributes.push(svg::float_attr(
                "rx",
                calculate_axis_radius(self.rx, self.width, self.height),
            ));
        }
        if self.ry != UNSET {
            attributes.push(svg::float_attr(
                "ry",
                calculate_axis_radius(self.ry, self.width, self.height),
            ));
        }
        if self.cx != UNSET {
            attributes.push(svg::float_attr("cx", self.cx));
        }
        if self.cy != UNSET {
            attributes.push(svg::float_attr("cy", self.cy));
        }
        if !self.stroke_dash_array.is_empty() {
            attributes.push(svg::attr("stroke-dasharray", &self.stroke_dash_array));
        }
        if !self.d.is_empty() {
            attributes.push(svg::attr("d", &self.d));
        }
        let mask = if !self.mask_id.is_empty() {
            self.mask_id.clone()
        } else if !self.mask.is_empty() {
            svg::local_iri(
                &svg::parse_local_iri(&self.mask).unwrap_or_else(|error| panic!("{error}")),
            )
        } else {
            String::new()
        };
        if !mask.is_empty() {
            attributes.push(svg::attr("mask", mask));
        }
        for (name, value) in [
            ("points", &self.points),
            ("transform", &self.transform),
            ("xmlns", &self.xmlns),
        ] {
            if !value.is_empty() {
                attributes.push(svg::attr(name, value));
            }
        }

        let mut class = self.class_name.clone();
        let style = self.style.clone();
        let theme = self.inline_theme;
        if color::is_theme_color(&self.stroke) {
            class += &format!(" stroke-{}", self.stroke);
            if let Some(theme) = theme {
                attributes.push(svg::attr(
                    "stroke",
                    resolve_theme_color(theme, &self.stroke),
                ));
            }
        } else if !self.stroke.is_empty() {
            if color::is_gradient(&self.stroke) {
                self.stroke = format!("url('#{}')", color::unique_gradient_id(&self.stroke));
            }
            attributes.push(svg::attr("stroke", &self.stroke));
        }
        if color::is_theme_color(&self.fill) {
            class += &format!(" fill-{}", self.fill);
            if let Some(theme) = theme {
                attributes.push(svg::attr("fill", resolve_theme_color(theme, &self.fill)));
            }
        } else if !self.fill.is_empty() {
            if color::is_gradient(&self.fill) {
                self.fill = format!("url('#{}')", color::unique_gradient_id(&self.fill));
            }
            attributes.push(svg::attr("fill", &self.fill));
        }
        if color::is_theme_color(&self.background_color) {
            class += &format!(" background-color-{}", self.background_color);
            if let Some(theme) = theme {
                attributes.push(svg::attr(
                    "background-color",
                    resolve_theme_color(theme, &self.background_color),
                ));
            }
        } else if !self.background_color.is_empty() {
            attributes.push(svg::attr("background-color", &self.background_color));
        }
        if color::is_theme_color(&self.color) {
            class += &format!(" color-{}", self.color);
            if let Some(theme) = theme {
                attributes.push(svg::attr("color", resolve_theme_color(theme, &self.color)));
            }
        } else if !self.color.is_empty() {
            attributes.push(svg::attr("color", &self.color));
        }

        if !class.is_empty() {
            attributes.push(svg::attr("class", class));
        }
        if !style.is_empty() {
            attributes.push(svg::attr("style", style));
        }
        attributes.extend(self.extra_attributes.clone());
        if !self.attributes.is_empty() {
            attributes.extend(
                svg::parse_attributes(&self.attributes).unwrap_or_else(|error| panic!("{error}")),
            );
        }
        let clip_path = if !self.clip_path_id.is_empty() {
            self.clip_path_id.clone()
        } else if !self.clip_path.is_empty() {
            svg::literal_id(&self.clip_path).as_str().into()
        } else {
            String::new()
        };
        if !clip_path.is_empty() {
            attributes.push(svg::attr(
                "clip-path",
                svg::local_iri(&svg::literal_id(&clip_path)),
            ));
        }
        let mut out = svg::open_element(&self.tag, &attributes);
        let inner = self
            .inner
            .clone()
            .or_else(|| (!self.content.is_empty()).then(|| svg::trusted_fragment(&self.content)));
        if let Some(inner) = inner.filter(|inner| !inner.as_str().is_empty()) {
            out.push_str(inner.as_str());
            out.push_str(&svg::close_element(&self.tag));
            return out;
        }
        out.pop();
        out += " />";
        let fill_pattern = self.fill_pattern.to_lowercase();
        if !fill_pattern.is_empty() && fill_pattern != "none" {
            let mut pattern = self.clone();
            pattern.fill.clear();
            pattern.stroke.clear();
            pattern.background_color.clear();
            pattern.color.clear();
            pattern.class_name = format!("{fill_pattern}-overlay");
            pattern.fill_pattern.clear();
            out += &pattern.render_mut();
        }
        out
    }
}

/// element.go calculateAxisRadius.
fn calculate_axis_radius(border_radius: f64, width: f64, height: f64) -> f64 {
    if border_radius.is_nan() || width.is_nan() || height.is_nan() {
        return f64::NAN;
    }
    let minimum_side_size = width.min(height);
    border_radius.min(minimum_side_size / 2.0)
}
