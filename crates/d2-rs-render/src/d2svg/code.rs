// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2svg code labels: the `Language != ""` chroma branches of
// d2renderers/d2svg/d2svg.go drawShape (2126-2189) and drawConnection
// (1266-1312), plus d2renderers/d2svg/code.go (svgEscaper, styleToSVG,
// styleAttr; themselves copied from chroma's SVG formatter). Chroma v2.27.0
// lexing (lexers.Get/Fallback, Tokenise, SplitTokensIntoLines) and style
// lookup (styles.Get, Style.Get, svg.StyleEntryToSVG) come from
// d2_rs_text::code.

use std::fmt::Write as _;

use d2_rs_text::code::{self, chroma_style};

use super::element::{ThemableElement, Theme};
use super::geo::{self, Point};
use super::svgtext::format_float;
use crate::target::{TargetConnection, TargetShape};

/// lib/textmeasure.CODE_LINE_HEIGHT.
const CODE_LINE_HEIGHT: f64 = 1.3;
/// d2renderers/d2fonts.FONT_SIZE_M.
const FONT_SIZE_M: i64 = 16;

/// lexers.Get(language), falling back to lexers.Fallback, then
/// lexer.Tokenise(nil, label) and chroma.SplitTokensIntoLines.
fn token_lines(language: &str, label: &str) -> Result<Vec<Vec<code::Token>>, String> {
    let tokens = match code::lexer_name(language) {
        // lexers.Fallback: PlaintextRules (`.+` / `\n` as Text) after the
        // default TokeniseOptions' EnsureLF.
        None => code::plain(&label.replace("\r\n", "\n").replace('\r', "\n")),
        Some(name) => code::tokenize(language, label).ok_or_else(|| {
            format!("unsupported: {language} label (untranslated Chroma lexer {name})")
        })?,
    };
    Ok(code::split_lines(tokens))
}

/// code.go svgEscaper (a strings.NewReplacer of single-byte keys).
fn svg_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            ' ' => out.push_str("&#160;"),
            '\t' => out.push_str("&#160;&#160;&#160;&#160;"),
            c => out.push(c),
        }
    }
    out
}

/// code.go styleAttr.
fn style_attr(styles: &std::collections::BTreeMap<i32, String>, kind: &str) -> String {
    let Some(mut tt) = chroma_style::token_type(kind) else {
        return String::new();
    };
    if !styles.contains_key(&tt) {
        tt = chroma_style::sub_category(tt);
        if !styles.contains_key(&tt) {
            tt = chroma_style::category(tt);
            if !styles.contains_key(&tt) {
                return String::new();
            }
        }
    }
    let mut out = styles[&tt].clone();
    let mut classes = Vec::new();
    if out.contains(r#"font-weight="bold""#) {
        out = out.replacen(r#"font-weight="bold""#, "", 1);
        classes.push("text-mono-bold");
    }
    if out.contains(r#"font-style="italic""#) {
        out = out.replacen(r#"font-style="italic""#, "", 1);
        classes.push("text-mono-italic");
    }
    if !classes.is_empty() {
        out.push_str(&format!(r#"class="{}""#, classes.join(" ")));
    }
    out.trim().to_owned()
}

/// The shared `<text>` line loop of both branches.
fn write_lines(w: &mut String, lines: &[Vec<code::Token>], dark: bool) {
    let styles = chroma_style::svg_styles(dark);
    for (index, tokens) in lines.iter().enumerate() {
        // 1+float64(index)*lineHeight: Go ARM64 fuses this into FMADDD.
        let y = geo::go_fmadd(index as f64, CODE_LINE_HEIGHT, 1.0);
        write!(
            w,
            r#"<text class="text-mono" x="0" y="{}em">"#,
            format_float(y)
        )
        .unwrap();
        for token in tokens {
            let text = svg_escape(&token.value);
            let attr = style_attr(styles, token.kind);
            if attr.is_empty() {
                w.push_str(&text);
            } else {
                write!(w, "<tspan {attr}>{text}</tspan>").unwrap();
            }
        }
        w.push_str("</text>");
    }
}

fn font_size_style(font_size: i64) -> String {
    if font_size != FONT_SIZE_M {
        format!(r#" style="font-size:{font_size}""#)
    } else {
        String::new()
    }
}

/// drawShape's syntax-highlighted code label.
pub(crate) fn shape_code(
    w: &mut String,
    s: &TargetShape,
    b: &geo::Box,
    theme: Option<&Theme>,
) -> Result<(), String> {
    let lines = token_lines(&s.language, &s.label)?;
    for is_light in [true, false] {
        let dark = !is_light;
        let class = if is_light { "light-code" } else { "dark-code" };
        write!(
            w,
            r#"<g transform="translate({} {})" class="{class}"{}>"#,
            format_float(b.top_left.x),
            format_float(b.top_left.y),
            font_size_style(s.font_size)
        )
        .unwrap();
        let mut rect = ThemableElement::new("rect", theme);
        rect.width = s.width as f64;
        rect.height = s.height as f64;
        rect.stroke.clone_from(&s.stroke);
        rect.class_name = "shape".into();
        rect.style = format!(
            "fill:{};stroke-width:{};",
            chroma_style::background(dark),
            s.stroke_width
        );
        w.push_str(&rect.render());
        // Padding = 0.5em
        let padding = s.font_size as f64 / 2.0;
        write!(
            w,
            r#"<g transform="translate({} {})">"#,
            format_float(padding),
            format_float(padding)
        )
        .unwrap();
        write_lines(w, &lines, dark);
        w.push_str("</g></g>");
    }
    Ok(())
}

/// drawConnection's syntax-highlighted code label.
pub(crate) fn connection_label(
    w: &mut String,
    c: &TargetConnection,
    label_tl: Point,
    _theme: Option<&Theme>,
) -> Result<(), String> {
    let lines = token_lines(&c.language, &c.label)?;
    for is_light in [true, false] {
        let class = if is_light { "light-code" } else { "dark-code" };
        write!(
            w,
            r#"<g transform="translate({} {})" class="{class}"{}>"#,
            format_float(label_tl.x),
            format_float(label_tl.y),
            font_size_style(c.font_size)
        )
        .unwrap();
        write_lines(w, &lines, !is_light);
        w.push_str("</g>");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_attr_moves_bold_and_italic_into_classes_like_code_go() {
        let mut styles = std::collections::BTreeMap::new();
        styles.insert(
            1000,
            r##"fill="#cba6f7" font-weight="bold" font-style="italic""##.to_owned(),
        );
        styles.insert(3100, r##"fill="#a6e3a1""##.to_owned());
        // Exact, then SubCategory, then Category lookups.
        assert_eq!(
            style_attr(&styles, "KeywordType"),
            r##"fill="#cba6f7"  class="text-mono-bold text-mono-italic""##
        );
        assert_eq!(
            style_attr(&styles, "LiteralStringDouble"),
            r##"fill="#a6e3a1""##
        );
        assert_eq!(style_attr(&styles, "Punctuation"), "");
        assert_eq!(
            svg_escape("a <b>\t&\"c\""),
            "a&#160;&lt;b&gt;&#160;&#160;&#160;&#160;&amp;&quot;c&quot;"
        );
    }

    #[test]
    fn unknown_languages_use_the_plaintext_fallback() {
        let lines = token_lines("d2", "x: {\r\n  y\n}").unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().flatten().all(|t| t.kind == "Text"));
    }
}
