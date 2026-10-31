// SPDX-License-Identifier: MPL-2.0
// d2scenebuild/code_shape.go and d2svg.drawShape language branch, D2 v0.9.0.
use super::*;
use d2_rs_text::{
    code,
    measure::{bundled_source_code_pro_bold, bundled_source_code_pro_italic},
};

fn font(bold: bool, italic: bool) -> &'static TrueTypeFont {
    static REGULAR: OnceLock<TrueTypeFont> = OnceLock::new();
    static BOLD: OnceLock<TrueTypeFont> = OnceLock::new();
    static ITALIC: OnceLock<TrueTypeFont> = OnceLock::new();
    if italic {
        ITALIC.get_or_init(|| bundled_source_code_pro_italic().expect("bundled mono face"))
    } else if bold {
        BOLD.get_or_init(|| bundled_source_code_pro_bold().expect("bundled mono face"))
    } else {
        REGULAR.get_or_init(|| bundled_source_code_pro_regular().expect("bundled mono face"))
    }
}

pub(super) struct Runs {
    lines: Vec<Vec<code::Token>>,
    advances: Vec<Vec<f64>>,
}
fn token_text(value: &str) -> String {
    let value = value.strip_suffix('\n').unwrap_or(value);
    value
        .strip_suffix('\r')
        .unwrap_or(value)
        .replace('\t', "    ")
}
pub(super) fn prepare(
    object: &str,
    tokens: Vec<code::Token>,
    font_size: u32,
    dark: bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Runs, RenderError> {
    let lines = code::split_lines(tokens);
    let mut advances = Vec::with_capacity(lines.len());
    for (line_index, line) in lines.iter().enumerate() {
        code_budget::context(cancel)?;
        let mut row = Vec::with_capacity(line.len());
        for (token_index, token) in line.iter().enumerate() {
            code_budget::context(cancel)?;
            let value = token_text(&token.value);
            if value.is_empty() {
                row.push(0.0);
                continue;
            }
            let entry = code::style(token.kind, dark);
            let italic = entry.italic == Some(true);
            let bold = entry.bold == Some(true) && !italic;
            let advance = font(bold, italic)
                .measure_code_advance_checked(&value, f64::from(font_size), cancel)
                .map_err(|error| RenderError::CodeGlyph {
                    object: object.to_owned(),
                    line: line_index,
                    token: token_index,
                    error,
                })?;
            row.push(advance);
        }
        advances.push(row);
    }
    Ok(Runs { lines, advances })
}

pub(super) fn draw_cached(
    svg: &mut String,
    label: &str,
    language: &str,
    rect: Rect,
    font_size: u32,
    source_style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    dark: bool,
    preserve_precision: bool,
    cached: Option<&Option<Runs>>,
    identity_prefix: &str,
) -> bool {
    if label.is_empty() {
        return true;
    }
    let background = code::style("Background", dark)
        .background
        .expect("pinned code background");
    let stroke = themed_color(
        source_style.stroke.as_deref(),
        &palette.color("N1", palette.base.neutrals.n1),
        palette,
    );
    // No token background/border, shape dash, radius, fill pattern, multiple
    // or 3D modifier belongs to the Source code background primitive.
    let _ = writeln!(
        svg,
        "<g class=\"{}\" data-d2-language=\"{}\"><rect class=\"shape\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
        if dark { "dark-code" } else { "light-code" },
        escape(language),
        number(rect.origin.x),
        number(rect.origin.y),
        number(rect.size.width),
        number(rect.size.height),
        background,
        escape(&stroke),
        escape(source_style.stroke_width.as_deref().unwrap_or("2"))
    );
    let rendered = draw_runs_cached(
        svg,
        label,
        language,
        Point {
            x: rect.origin.x + f64::from(font_size) / 2.0,
            y: rect.origin.y + f64::from(font_size) / 2.0,
        },
        font_size,
        dark,
        preserve_precision,
        cached,
        identity_prefix,
    );
    svg.push_str("</g>\n");
    rendered
}

/// Source connection code runs have no background and begin at label top-left.
#[cfg(test)]
pub(super) fn draw(
    svg: &mut String,
    label: &str,
    language: &str,
    rect: Rect,
    font_size: u32,
    source_style: &SourceNodeStyle,
    palette: PaletteView<'_>,
    dark: bool,
    preserve_precision: bool,
    identity_prefix: &str,
) -> bool {
    draw_cached(
        svg,
        label,
        language,
        rect,
        font_size,
        source_style,
        palette,
        dark,
        preserve_precision,
        None,
        identity_prefix,
    )
}
#[cfg(test)]
pub(super) fn draw_runs(
    svg: &mut String,
    label: &str,
    language: &str,
    origin: Point,
    font_size: u32,
    dark: bool,
    preserve_precision: bool,
    identity_prefix: &str,
) -> bool {
    draw_runs_cached(
        svg,
        label,
        language,
        origin,
        font_size,
        dark,
        preserve_precision,
        None,
        identity_prefix,
    )
}
pub(super) fn draw_runs_cached(
    svg: &mut String,
    label: &str,
    language: &str,
    origin: Point,
    font_size: u32,
    dark: bool,
    preserve_precision: bool,
    cached: Option<&Option<Runs>>,
    identity_prefix: &str,
) -> bool {
    let fallback;
    let runs = match cached {
        Some(Some(runs)) => runs,
        Some(None) => return false,
        None => {
            let Some(tokens) = code::tokenize(language, label) else {
                return false;
            };
            let lines = code::split_lines(tokens);
            let advances = lines
                .iter()
                .map(|line| {
                    line.iter()
                        .map(|token| {
                            let entry = code::style(token.kind, dark);
                            let italic = entry.italic == Some(true);
                            let bold = entry.bold == Some(true) && !italic;
                            font(bold, italic).measure_code_advance(
                                &token_text(&token.value),
                                f64::from(font_size),
                            )
                        })
                        .collect()
                })
                .collect();
            fallback = Runs { lines, advances };
            &fallback
        }
    };
    let size = f64::from(font_size);
    let line_advance = size * 1.3;
    for (line_index, tokens) in runs.lines.iter().enumerate() {
        // Original buildCodeTextRuns forms lineTop first, then adds font size.
        // Its pinned ARM64 build fuses the line-index multiply with origin.Y.
        let line_top = (line_index as f64).mul_add(line_advance, origin.y);
        let baseline = line_top + size;
        let mut cursor = origin.x;
        for (token_index, token) in tokens.iter().enumerate() {
            let value = token_text(&token.value);
            if value.is_empty() {
                continue;
            }
            let entry = code::style(token.kind, dark);
            let italic = entry.italic == Some(true);
            let bold = entry.bold == Some(true) && !italic;
            let advance = runs.advances[line_index][token_index];
            if value.chars().any(|character| !character.is_whitespace())
                || entry.underline == Some(true)
            {
                let class = text_font_class(TextStyle {
                    mono: true,
                    bold,
                    italic,
                    underline: false,
                });
                let face = if italic {
                    " font-style=\"italic\""
                } else if bold {
                    " font-weight=\"bold\""
                } else {
                    ""
                };
                let underline = if entry.underline == Some(true) {
                    " text-decoration=\"underline\""
                } else {
                    ""
                };
                // Native raster runs retain the source f64 cursor and ordinary
                // scene paint reference, just like other native text runs.
                // Public SVG does not carry this private precision transport.
                let precision = if preserve_precision {
                    format!(
                        " id=\"d2-rs-native-code-text-{}-line-{line_index}-token-{token_index}\" data-d2-rs-text-x-bits=\"{:016x}\" data-d2-rs-text-y-bits=\"{:016x}\"",
                        escape(identity_prefix),
                        cursor.to_bits(),
                        baseline.to_bits()
                    )
                } else {
                    String::new()
                };
                let _ = writeln!(
                    svg,
                    "<text class=\"{class}\" x=\"{}\" y=\"{}\" text-anchor=\"start\" font-size=\"{font_size}px\" fill=\"{}\" xml:space=\"preserve\"{face}{underline}{precision}>{}</text>",
                    native_markdown_text_number(cursor, preserve_precision),
                    native_markdown_text_number(baseline, preserve_precision),
                    entry.color.as_deref().unwrap_or("#000000"),
                    escape(&value)
                );
            }
            cursor += advance;
        }
    }
    true
}

/// Public d2svg code branch: two switchable themes and token spans. Shapes
/// use the exported box and half-em padding; connections have no background.
pub(super) fn draw_public(
    svg: &mut String,
    label: &str,
    language: &str,
    origin: Point,
    background: Option<(Size, &SourceNodeStyle, PaletteView<'_>)>,
    font_size: u32,
) -> bool {
    let tokens = code::tokenize(language, label).or_else(|| {
        (!code::registered(language))
            .then(|| code::plain(&label.replace("\r\n", "\n").replace('\r', "\n")))
    });
    let Some(tokens) = tokens else {
        return false;
    };
    let lines = code::split_lines(tokens);
    for dark in [false, true] {
        let font_style = if font_size == 16 {
            String::new()
        } else {
            format!(" style=\"font-size:{font_size}\"")
        };
        let _ = write!(
            svg,
            "<g transform=\"translate({} {})\" class=\"{}\"{font_style}>",
            number(origin.x),
            number(origin.y),
            if dark { "dark-code" } else { "light-code" }
        );
        if let Some((size, style, palette)) = background {
            let stroke = themed_color(
                style.stroke.as_deref(),
                &palette.color("N1", palette.base.neutrals.n1),
                palette,
            );
            let _ = write!(
                svg,
                "<rect class=\"shape\" width=\"{}\" height=\"{}\" stroke=\"{}\" style=\"fill:{};stroke-width:{};\"/><g transform=\"translate({} {})\">",
                number(size.width),
                number(size.height),
                escape(&stroke),
                code::style("Background", dark).background.unwrap(),
                escape(style.stroke_width.as_deref().unwrap_or("2")),
                number(f64::from(font_size) / 2.0),
                number(f64::from(font_size) / 2.0)
            );
        }
        for (index, line) in lines.iter().enumerate() {
            let _ = write!(
                svg,
                "<text class=\"text-mono\" x=\"0\" y=\"{}em\">",
                number(1.0 + index as f64 * 1.3)
            );
            for token in line {
                let entry = code::style(token.kind, dark);
                let mut attributes = String::new();
                if let Some(color) = entry.color {
                    let _ = write!(attributes, " fill=\"{}\"", escape(&color));
                }
                if entry.underline == Some(true) {
                    attributes.push_str(" text-decoration=\"underline\"");
                }
                let mut classes = Vec::new();
                if entry.bold == Some(true) {
                    classes.push("text-mono-bold");
                }
                if entry.italic == Some(true) {
                    classes.push("text-mono-italic");
                }
                if !classes.is_empty() {
                    let _ = write!(attributes, " class=\"{}\"", classes.join(" "));
                }
                // Source svgEscaper retains newlines and uses NBSP for spaces
                // and four NBSPs for each tab, independent of token position.
                let value = escape(&token.value)
                    .replace(' ', "&#160;")
                    .replace('\t', "&#160;&#160;&#160;&#160;");
                if attributes.is_empty() {
                    svg.push_str(&value);
                } else {
                    let _ = write!(svg, "<tspan{attributes}>{value}</tspan>");
                }
            }
            svg.push_str("</text>");
        }
        if background.is_some() {
            svg.push_str("</g>");
        }
        svg.push_str("</g>\n");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_unknown_fallback_keeps_plain_tokens_and_known_lexer_gap_distinct() {
        let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
        assert!(draw_public(
            &mut svg,
            "a\r\n\t<&>",
            "totally-unregistered",
            Point::default(),
            None,
            16
        ));
        svg.push_str("</svg>");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let groups: Vec<_> = doc
            .descendants()
            .filter(|n| matches!(n.attribute("class"), Some("light-code" | "dark-code")))
            .collect();
        assert_eq!(groups.len(), 2);
        for group in groups {
            let text = group
                .descendants()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect::<String>();
            assert_eq!(text, "a\n\u{a0}\u{a0}\u{a0}\u{a0}<&>");
        }
        let mut gap = String::new();
        assert!(!draw_public(
            &mut gap,
            "fn f() {}",
            "file.rs",
            Point::default(),
            None,
            16
        ));
        assert!(gap.is_empty());
        assert_eq!(code::lexer_name("file.rs"), Some("Rust"));
    }

    #[test]
    fn public_code_preserves_source_theme_groups_tokens_and_spacing() {
        let overrides = BTreeMap::new();
        let palette = PaletteView::new(active_palette(None), &overrides);
        let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
        assert!(draw_public(
            &mut svg,
            "func f() {\n\t// <&>\n}",
            "go",
            Point { x: -8.25, y: 7.5 },
            Some((
                Size {
                    width: 200.0,
                    height: 80.0
                },
                &SourceNodeStyle::default(),
                palette
            )),
            23
        ));
        svg.push_str("</svg>");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        for class in ["light-code", "dark-code"] {
            let group = doc
                .descendants()
                .find(|n| n.attribute("class") == Some(class))
                .unwrap();
            assert_eq!(group.attribute("transform"), Some("translate(-8.25 7.5)"));
            assert_eq!(group.attribute("style"), Some("font-size:23"));
            assert_eq!(
                group.children().filter(|n| n.has_tag_name("rect")).count(),
                1
            );
            let lines: Vec<_> = group
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect();
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[1].attribute("y"), Some("2.3em"));
            let comment = lines[1]
                .descendants()
                .find(|n| n.has_tag_name("tspan") && n.text().is_some_and(|v| v.starts_with("//")))
                .unwrap();
            assert_eq!(comment.text(), Some("//\u{a0}<&>"));
            if class == "dark-code" {
                assert_eq!(comment.attribute("class"), Some("text-mono-italic"));
            }
        }
        assert!(!svg.contains("data-d2-rs-text"));
        let mut connection = String::new();
        assert!(draw_public(
            &mut connection,
            "x = 1",
            "python",
            Point::default(),
            None,
            16
        ));
        assert!(!connection.contains("<rect"));
        assert!(connection.contains("<tspan"));
    }

    #[test]
    fn code_precision_transport_is_private_and_preserves_each_run_origin() {
        let overrides = BTreeMap::new();
        let palette = PaletteView::new(active_palette(None), &overrides);
        for private in [false, true] {
            let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
            for (owner, x) in [10.125, -8.25].into_iter().enumerate() {
                assert!(draw(
                    &mut svg,
                    "func f() {\n\t// comment\n}",
                    "go",
                    Rect {
                        origin: Point { x, y: 7.25 },
                        size: Size {
                            width: 200.0,
                            height: 80.0
                        }
                    },
                    23,
                    &SourceNodeStyle::default(),
                    palette,
                    false,
                    private,
                    &format!("origin-{owner}")
                ));
            }
            svg.push_str("</svg>");
            let document = roxmltree::Document::parse(&svg).unwrap();
            let mut ids = std::collections::HashSet::new();
            for text in document
                .descendants()
                .filter(|node| node.has_tag_name("text"))
            {
                assert_eq!(text.attribute("id").is_some(), private);
                for coordinate in ["x", "y"] {
                    let attribute = format!("data-d2-rs-text-{coordinate}-bits");
                    assert_eq!(text.attribute(attribute.as_str()).is_some(), private);
                    if private {
                        let original: f64 = text.attribute(coordinate).unwrap().parse().unwrap();
                        let bits =
                            u64::from_str_radix(text.attribute(attribute.as_str()).unwrap(), 16)
                                .unwrap();
                        assert_eq!(original.to_bits(), bits);
                    }
                }
                if private {
                    assert!(ids.insert(text.attribute("id").unwrap()));
                }
            }
            if private {
                assert!(ids.len() > 6);
            }
        }
    }

    #[test]
    fn untranslated_lexer_keeps_background_group_complete_for_caller_label() {
        let overrides = BTreeMap::new();
        let palette = PaletteView::new(active_palette(None), &overrides);
        let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\"><g>");
        assert!(!draw(
            &mut svg,
            "fn f() {}",
            "rust",
            Rect {
                origin: Point::default(),
                size: Size {
                    width: 140.0,
                    height: 80.0
                }
            },
            16,
            &SourceNodeStyle::default(),
            palette,
            false,
            true,
            "untranslated-code"
        ));
        svg.push_str("<text>caller label</text></g></svg>");
        let document = roxmltree::Document::parse(&svg).expect("complete emitted background group");
        let text = document
            .descendants()
            .find(|node| node.has_tag_name("text"))
            .unwrap();
        assert!(text.parent().unwrap().attribute("class").is_none());
        let background = document
            .descendants()
            .find(|node| node.has_tag_name("rect"))
            .unwrap();
        assert_eq!(
            background.parent().unwrap().attribute("class"),
            Some("light-code")
        );
    }

    #[test]
    fn code_runs_use_source_padding_baselines_faces_and_stroke_contract() {
        let overrides = BTreeMap::new();
        let palette = PaletteView::new(active_palette(None), &overrides);
        let style = SourceNodeStyle {
            stroke: Some("#112233".into()),
            stroke_width: Some("3".into()),
            stroke_dash: Some("5".into()),
            ..Default::default()
        };
        let rect = Rect {
            origin: Point { x: 10.0, y: 20.0 },
            size: Size {
                width: 100.0,
                height: 70.0,
            },
        };
        let mut svg = String::new();
        assert!(draw(
            &mut svg,
            "func f() {\n\t// comment\n}",
            "go",
            rect,
            10,
            &style,
            palette,
            true,
            true,
            "styled-code"
        ));
        assert!(svg.contains("fill=\"#1e1e2e\" stroke=\"#112233\" stroke-width=\"3\""));
        assert!(!svg.contains("dash"));
        assert!(svg.contains("x=\"15\" y=\"35\" text-anchor=\"start\""));
        assert!(svg.contains("class=\"text-mono-italic\" x=\"39\" y=\"48\""));
        assert!(svg.contains("fill=\"#f38ba8\""));
        assert!(!svg.contains("text-mono-bold\" x=\"15\""));
    }
}

#[cfg(test)]
mod original_line_origin_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn code_line_origins_match_original_scene_text_runs() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/code-line-origins-source-v090.json"))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 36);
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let size = row["size"].as_u64().unwrap() as u32;
            let mut svg = String::from("<svg>");
            assert!(draw_runs(
                &mut svg,
                row["label"].as_str().unwrap(),
                "python",
                Point {
                    x: f64::from(size) / 2.,
                    y: row["pos_y"].as_f64().unwrap() + f64::from(size) / 2.
                },
                size,
                false,
                true,
                "origin-control"
            ));
            svg.push_str("</svg>");
            let document = roxmltree::Document::parse(&svg).unwrap();
            let actual = document
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .collect::<Vec<_>>();
            let expected = row["tokens"].as_array().unwrap();
            assert_eq!(actual.len(), expected.len(), "{row}");
            for (i, (node, token)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    node.text().unwrap_or(""),
                    token["text"].as_str().unwrap(),
                    "{row}"
                );
                let bits = |attribute| {
                    u64::from_str_radix(node.attribute(attribute).unwrap(), 16).unwrap()
                };
                let actual = (
                    bits("data-d2-rs-text-x-bits"),
                    bits("data-d2-rs-text-y-bits"),
                );
                let expected = (
                    token["x_bits"].as_u64().unwrap(),
                    token["y_bits"].as_u64().unwrap(),
                );
                if actual != expected {
                    differences.push(format!(
                        "size {size} y {} token {i}: {:?} vs {:?}",
                        row["pos_y"], actual, expected
                    ));
                }
            }
        }
        assert!(
            differences.is_empty(),
            "{} original scene origin differences: {:?}",
            differences.len(),
            differences.iter().take(8).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod independent_code_identity_contract {
    use super::*;
    #[test]
    fn separately_rendered_node_and_edge_code_runs_have_unique_scene_identities() {
        let mut combined = String::from("<svg>");
        for owner in ["node-0", "node-1", "edge-0"] {
            let mut fragment = String::new();
            assert!(draw_runs_cached(
                &mut fragment,
                "print(\"x\")",
                "python",
                Point { x: 0., y: 0. },
                16,
                false,
                true,
                None,
                owner
            ));
            combined.push_str(&fragment);
        }
        combined.push_str("</svg>");
        let document = roxmltree::Document::parse(&combined).unwrap();
        let mut identities = std::collections::BTreeSet::new();
        let mut counts = std::collections::BTreeMap::new();
        for node in document.descendants().filter(|n| n.has_tag_name("text")) {
            let identity = node.attribute("id").expect("native scene identity");
            assert!(
                identities.insert(identity),
                "duplicate code scene identity {identity}"
            );
            let owner = ["node-0", "node-1", "edge-0"]
                .into_iter()
                .find(|owner| identity.contains(owner))
                .unwrap();
            *counts.entry(owner).or_insert(0) += 1;
            assert!(node.attribute("data-d2-rs-text-x-bits").is_some());
        }
        assert_eq!(counts.len(), 3);
        assert!(counts.values().all(|&n| n > 0 && n == counts["node-0"]));
    }
}
