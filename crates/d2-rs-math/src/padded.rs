// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 tex/commands.go and svg/mpadded.go,mphantom.go.
use super::*;
#[derive(Clone, Debug, Default)]
pub(super) struct Padding {
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
        let dx = p.lspace_width * local.width + p.lspace_em * 1000.0 * scale;
        let dy = p.voffset_em * 1000.0 * scale;
        if dx != 0.0 || dy != 0.0 {
            // Original mpadded puts lspace in a separate, independently rounded g.
            let mut frame = Layout {
                bounds_em: None,
                width_em: None,
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
        if p.width_add != 0.0 {
            local.width = (local.width + p.width_add * 1000.0 * scale).max(0.0);
        }
        if p.height_add != 0.0 {
            local.ascent = (local.ascent + p.height_add * 1000.0 * scale).max(0.0);
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
        if hidden { "mphantom" } else { "mpadded" },
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
                    let body =
                        Atom::Style(Box::new(Atom::Text(match self.padded_raw_argument(name) {
                            Ok(raw) => raw,
                            Err(message) => return Some(Atom::ErrorDisplay(message)),
                        })));
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
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            normal_font: self.normal_font,
            calligraphic_font: self.calligraphic_font,
            multi_letter_font: true,
            cd_minimums: self.cd_minimums,
        };
        parser.sequence(None)
    }
    pub(super) fn padded_parse_string(&self, raw: &str) -> Option<Atom> {
        let mut sub = Parser {
            ams: self.ams.clone(),
            splice_inferred: false,
            chars: raw.chars().collect(),
            index: 0,
            normal_font: self.normal_font,
            calligraphic_font: self.calligraphic_font,
            multi_letter_font: false,
            cd_minimums: self.cd_minimums,
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
