// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 tex/commands.go and svg/mpadded.go,mphantom.go.
use super::*;
#[derive(Clone, Debug, Default)]
pub(super) struct Padding {
    pub aligned_width: Option<(String, &'static str, f64)>,
    pub mathsize: Option<f64>,
    pub position: Option<[String; 2]>,
    pub voffset: Option<String>,
    pub root: Option<[Option<String>; 2]>,
    pub width_zero: bool,
    pub width_absolute: Option<f64>,
    pub height_zero: bool,
    pub depth_zero: bool,
    pub lspace_width: f64,
    pub width_add: f64,
    pub height_add: f64,
    pub depth_add: f64,
    pub lspace_em: f64,
    pub voffset_em: f64,
}
fn group(
    body: Layout,
    kind: &'static str,
    x: f64,
    baseline: f64,
    scale: f64,
    before_glyph: usize,
) -> LayoutGroup {
    LayoutGroup {
        x_frame: None,
        data_cramped: None,
        own_translate: None,
        accent_offset: None,
        box_border: None,
        reference: None,
        tagged: None,
        percentage: None,
        table_line: None,
        mathsize: None,
        tex_class: 0,
        body: Box::new(body),
        kind,
        x,
        baseline,
        scale,
        before_glyph,
        color: None,
        background: None,
        delimiter: None,
        arrow: None,
        lines: Vec::new(),
    }
}
pub(super) fn layout_node(
    body: &Atom,
    padding: Option<&Padding>,
    hidden: bool,
    x: f64,
    baseline: f64,
    scale: f64,
    out: &mut Layout,
    display: bool,
    prime: bool,
) -> Option<f64> {
    let mut local = Layout {
        bounds_em: None,
        width_em: None,
        intrinsic_width_em: None,
        width: 0.0,
        ascent: 0.0,
        depth: 0.0,
        glyphs: Vec::new(),
        rules: Vec::new(),
        groups: Vec::new(),
    };
    local.width = layout_at_with_style(body, 0.0, 0.0, scale, &mut local, display, prime)?;
    retain_negative_glyph_depth(&mut local);
    if hidden {
        local.glyphs.clear();
        local.rules.clear();
        local.groups.clear();
    }
    if let Some(p) = padding {
        let dimension_scale = scale * p.mathsize.unwrap_or(1.);
        if let Some((raw, align, mathsize)) = &p.aligned_width {
            let unit = 1000. * scale;
            let (h, d) = styles::em_bounds(&local);
            let size = if raw.contains("height") {
                h / scale
            } else if raw.contains("depth") {
                d / scale
            } else {
                local.width / unit
            };
            let mut width = length::dimension(raw).em(size, scale * mathsize);
            if raw.starts_with(['+', '-']) {
                width += size;
            }
            let width = width.max(0.);
            let delta = (width - local.width / unit) * unit;
            // mpadded.paddedToSVG cancels negative alignment displacement.
            let dx = match *align {
                "left" => 0.,
                "right" => delta.max(0.),
                _ => delta.max(0.) / 2.,
            };
            if dx != 0. {
                let mut frame = Layout {
                    bounds_em: None,
                    width_em: None,
                    intrinsic_width_em: None,
                    width: 0.,
                    ascent: 0.,
                    depth: 0.,
                    glyphs: vec![],
                    rules: vec![],
                    groups: vec![],
                };
                frame.width = local.width;
                frame.ascent = local.ascent;
                frame.depth = local.depth;
                frame.groups.push(group(local, "", dx, 0., scale, 0));
                local = frame;
            }
            local.width = width * unit;
            local.width_em = Some(width * scale);
        }
        // mpadded resolves named dimensions against the content box. An
        // explicit rule offset is independent of its height adjustment.
        let dimension = |raw: &str, default_size: f64, adjust: bool| {
            let unit = 1000. * scale;
            let size = if raw.contains("width") {
                local.width / unit
            } else if raw.contains("height") {
                local.ascent / unit
            } else if raw.contains("depth") {
                local.depth / unit
            } else {
                default_size
            };
            let value = length::dimension(raw).em(size, dimension_scale);
            if adjust && raw.starts_with(['+', '-']) {
                value + size
            } else {
                value
            }
        };
        let position = p.voffset.as_ref().map_or_else(
            || {
                p.position
                    .as_ref()
                    .map_or(0., |d| dimension(&d[0], 0., false))
            },
            |raw| dimension(raw, 0., false),
        );
        let adjusted_bounds = p.position.as_ref().map(|dimensions| {
            let unit = 1000. * scale;
            let h = local.ascent / unit;
            let d = local.depth / unit;
            (
                dimension(&dimensions[0], h, true).max(0.) * unit,
                dimension(&dimensions[1], d, true).max(0.) * unit,
            )
        });
        let dx = p.lspace_width * local.width + p.lspace_em * 1000.0 * scale;
        let root_dy = p
            .root
            .as_ref()
            .and_then(|r| r[1].as_ref())
            .map_or(0., |raw| length::dimension(raw).em(0., dimension_scale));
        let dy = (p.voffset_em + position + root_dy) * 1000.0 * scale;
        if dx != 0.0 || dy != 0.0 {
            // Original mpadded puts lspace in a separate, independently rounded g.
            let mut frame = Layout {
                bounds_em: None,
                width_em: None,
                intrinsic_width_em: None,
                width: local.width,
                ascent: local.ascent,
                depth: local.depth,
                glyphs: Vec::new(),
                rules: Vec::new(),
                groups: Vec::new(),
            };
            frame.groups.push(group(local, "", dx, dy, scale, 0));
            local = frame;
        }
        if let Some(root) = &p.root {
            let unit = 1000. * scale;
            if let Some(raw) = &root[0] {
                let w = local.width / unit;
                local.width = (w + length::dimension(raw).em(w, dimension_scale)).max(0.) * unit;
            }
            if let Some(raw) = &root[1] {
                let h = local.ascent / unit;
                local.ascent = (h + length::dimension(raw).em(h, dimension_scale)).max(0.) * unit;
            }
        }
        if p.width_add != 0.0 {
            local.width = (local.width + p.width_add * 1000.0 * scale).max(0.0);
        }
        if p.height_add != 0.0 {
            local.ascent = (local.ascent + p.height_add * 1000.0 * scale).max(0.0);
        }
        if let Some((height, depth)) = adjusted_bounds {
            local.ascent = height;
            local.depth = depth;
            local.bounds_em = None;
        }
        if p.depth_add != 0.0 {
            local.depth = (local.depth + p.depth_add * 1000.0 * scale).max(0.0);
        }
        if p.width_zero {
            local.width = 0.0;
        }
        if let Some(width) = p.width_absolute {
            local.width = width * 1000.0 * scale;
        }
        if p.height_zero {
            local.ascent = 0.0;
        }
        if p.depth_zero {
            local.depth = 0.0;
        }
    }
    let width = local.width;
    out.ascent = out.ascent.max(baseline + local.ascent);
    out.depth = out.depth.max(local.depth - baseline);
    out.groups.push(group(
        local,
        if hidden {
            "mphantom"
        } else if let Some((_, align, _)) = padding.and_then(|p| p.aligned_width.as_ref()) {
            match *align {
                "left" => "#padded-left",
                "right" => "#padded-right",
                _ => "#padded-center",
            }
        } else {
            "mpadded"
        },
        x,
        baseline,
        scale,
        out.glyphs.len(),
    ));
    Some(width)
}
impl Parser {
    pub(super) fn padded_command(&mut self, name: &str) -> Option<Atom> {
        let mut padding = Padding::default();
        let body = match name {
            "phantom" | "hphantom" | "vphantom" => {
                let phantom = Atom::Phantom(Box::new(self.padded_argument(name)?));
                if name == "phantom" {
                    return Some(Atom::Ord(Box::new(phantom)));
                }
                padding.width_zero = name == "vphantom";
                padding.height_zero = name == "hphantom";
                padding.depth_zero = name == "hphantom";
                phantom
            }
            "smash" => {
                let option = match self.padded_brackets() {
                    Ok(s) => s,
                    Err(s) => return Some(Atom::ErrorDisplay(s)),
                };
                padding.height_zero = option.is_empty() || option == "t";
                padding.depth_zero = option.is_empty() || option == "b";
                self.padded_argument(name)?
            }
            _ => {
                padding.width_zero = true;
                padding.lspace_width = if name.contains("llap") {
                    -1.0
                } else if name.contains("clap") {
                    -0.5
                } else {
                    0.0
                };
                if name == "clap" {
                    let raw = match self.padded_raw_argument(name) {
                        Ok(raw) => raw,
                        Err(message) => return Some(Atom::ErrorDisplay(message)),
                    };
                    let body = self.internal_math(&raw, None, true)?;
                    return Some(Atom::Padded {
                        body: Box::new(body),
                        padding,
                    });
                }
                self.padded_argument(name)?
            }
        };
        let mut atom = Atom::Padded {
            body: Box::new(body),
            padding,
        };
        if name.starts_with("math") || name.starts_with("cramped") {
            atom = Atom::CrampedStyle(Box::new(atom), name.starts_with("cramped"));
        }
        Some(Atom::Ord(Box::new(atom)))
    }
    pub(super) fn padded_raw_argument(&mut self, name: &str) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        let Some(c) = self.chars.get(self.index).copied() else {
            return Err(format!("Missing argument for \\{name}"));
        };
        if c == '}' {
            return Err("Extra close brace or missing open brace".into());
        }
        if c == '{' {
            return self
                .braced_text()
                .ok_or_else(|| "Missing close brace".into());
        }
        let start = self.index;
        self.index += 1;
        if c == '\\' {
            if self
                .chars
                .get(self.index)
                .is_some_and(char::is_ascii_alphabetic)
            {
                while self
                    .chars
                    .get(self.index)
                    .is_some_and(char::is_ascii_alphabetic)
                {
                    self.index += 1;
                }
                if self.chars.get(self.index) == Some(&' ') {
                    self.index += 1;
                }
            } else if self.index < self.chars.len() {
                self.index += 1;
            }
        }
        Ok(self.chars[start..self.index].iter().collect())
    }
    pub(super) fn padded_argument(&mut self, name: &str) -> Option<Atom> {
        let raw = match self.padded_raw_argument(name) {
            Ok(raw) => raw,
            Err(message) => return Some(Atom::ErrorDisplay(message)),
        };
        self.padded_parse_string(&raw)
    }
    pub(super) fn bounded_font_argument(&mut self, name: &str) -> Option<Atom> {
        let raw = match self.padded_raw_argument(name) {
            Ok(v) => v,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let mut parser = Parser {
            in_root: self.in_root,
            root_moves: Default::default(),
            physics_after: None,
            physics_auto: None,
            command_position: None,
            pending_cell: None,
            pending_infix: None,
            row_delimiter: None,
            active_color: self.active_color.clone(),
            equation_seen: Default::default(),
            array_cell: None,
            vector_alias: self.vector_alias,
            vector_font: self.vector_font,
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            active_font: Some(declaration_fonts::font_name(name)?),
            normal_font: false,
            calligraphic_font: false,
            multi_letter_font: !declaration_fonts::font_name(name)?.is_empty(),
            operator_letters: self.operator_letters,
            operator_pattern: false,
            no_auto_operator: true,
        };
        let saved_count = self.ams.borrow().paired_macro_count;
        self.ams.borrow_mut().paired_macro_count = 0;
        let result = parser.sequence(None);
        self.ams.borrow_mut().paired_macro_count = saved_count;
        result
    }
    pub(super) fn padded_parse_string(&self, raw: &str) -> Option<Atom> {
        let mut sub = Parser {
            in_root: self.in_root,
            root_moves: Default::default(),
            physics_after: None,
            physics_auto: None,
            command_position: None,
            pending_cell: None,
            pending_infix: None,
            row_delimiter: None,
            active_color: self.active_color.clone(),
            equation_seen: Default::default(),
            array_cell: None,
            vector_alias: self.vector_alias,
            vector_font: self.vector_font,
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            active_font: self.active_font,
            normal_font: self.normal_font,
            calligraphic_font: self.calligraphic_font,
            multi_letter_font: self.vector_alias && self.multi_letter_font,
            operator_letters: self.operator_letters,
            operator_pattern: self.operator_pattern,
            no_auto_operator: self.no_auto_operator,
        };
        sub.sequence(None)
    }
    pub(super) fn padded_brackets(&mut self) -> Result<String, String> {
        while self
            .chars
            .get(self.index)
            .is_some_and(|c| c.is_whitespace())
        {
            self.index += 1;
        }
        if self.chars.get(self.index) != Some(&'[') {
            return Ok(String::new());
        }
        let start = self.index + 1;
        let mut i = start;
        let mut depth = 0;
        while let Some(c) = self.chars.get(i) {
            match c {
                '\\' => {
                    i += 1;
                }
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return Err("Extra close brace while looking for ']'".into());
                    }
                    depth -= 1;
                }
                ']' if depth == 0 => {
                    let s = self.chars[start..i].iter().collect();
                    self.index = i + 1;
                    return Ok(s);
                }
                _ => {}
            }
            i += 1;
        }
        Err("Could not find closing ']' for argument".into())
    }
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_padding_phantom_smash_lap_boxes() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/padded-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 304);
        for row in rows {
            let expression = row["expression"].as_str().unwrap();
            let source = roxmltree::Document::parse(row["svg"].as_str().unwrap()).unwrap();
            let svg = render(expression).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let root = doc.root_element();
            // The root box of the Go render: width, height, viewBox and style.
            for k in ["width", "height", "viewBox", "style"] {
                assert_eq!(
                    root.attribute(k),
                    source.root_element().attribute(k),
                    "{expression} {k}"
                );
            }
            let size = measure(expression).unwrap();
            assert_eq!(
                (size.width, size.height),
                (
                    row["width"].as_f64().unwrap(),
                    row["height"].as_f64().unwrap()
                ),
                "{expression}"
            );
        }
    }
}
