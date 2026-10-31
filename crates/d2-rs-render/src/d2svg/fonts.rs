// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/font_corpus.go and d2svg.go
// embedFonts/appendOnTrigger.

use std::collections::{BTreeMap, BTreeSet};

use super::xml::{self, Token};

/// fontCorpora: embedded font class → rendered characters.
pub(crate) type FontCorpora = BTreeMap<&'static str, String>;

/// embeddedFontClasses, in the cascade order of embedFonts' rules.
const EMBEDDED_FONT_CLASSES: [&str; 8] = [
    "text",
    "text-semibold",
    "text-bold",
    "text-italic",
    "text-mono",
    "text-mono-semibold",
    "text-mono-bold",
    "text-mono-italic",
];

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// font_corpus.go collectFontCorporaAndClasses. The first value is None when
/// Go falls back to whole-corpus embedding; the second is None when the
/// class scan is not safe to reuse for theme pruning.
pub(crate) fn collect_font_corpora_and_classes(
    source: &str,
) -> (Option<FontCorpora>, Option<BTreeSet<String>>) {
    let wrapped = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">{source}</svg>"#
    );
    let Some(tokens) = xml::tokens(&wrapped) else {
        return (None, None);
    };
    #[derive(Clone, Copy)]
    struct Frame {
        font: i32,
        text: bool,
        ignore: bool,
    }
    let mut stack = vec![Frame {
        font: -1,
        text: false,
        ignore: false,
    }];
    let mut corpora: [String; 8] = Default::default();
    let mut classes = BTreeSet::new();
    let mut classes_safe = true;
    // The wrapper root is the decoder's first start element.
    let mut tokens = tokens.into_iter();
    tokens.next();
    for token in tokens {
        match token {
            Token::Start { name, attrs } => {
                if name.local.eq_ignore_ascii_case("script") {
                    classes_safe = false;
                }
                if !name.space.is_empty() && name.space != SVG_NS {
                    return (None, None);
                }
                let mut current = *stack.last().expect("frame");
                match name.local.as_str() {
                    "style" | "script" | "foreignObject" | "use" | "tref" | "textPath"
                    | "animate" | "animateMotion" | "animateTransform" | "set" => {
                        return (None, None);
                    }
                    "title" | "desc" | "metadata" => current.ignore = true,
                    "text" => current.text = true,
                    "tspan" | "a" => {}
                    _ => {
                        if current.text && !current.ignore {
                            return (None, None);
                        }
                    }
                }
                let mut local_font = -1;
                let mut class_seen = false;
                for attr in &attrs {
                    let lower = attr.name.local.to_lowercase();
                    if lower.starts_with("on")
                        || (lower == "attributename"
                            && attr.value.trim().eq_ignore_ascii_case("class"))
                    {
                        classes_safe = false;
                    }
                    if lower == "href" {
                        let value = attr.value.trim();
                        if (name.local.eq_ignore_ascii_case("use") && !value.starts_with('#'))
                            || value.to_lowercase().starts_with("javascript:")
                        {
                            classes_safe = false;
                        }
                    }
                    if attr.name.local.starts_with("font-variant") {
                        return (None, None);
                    }
                    match attr.name.local.as_str() {
                        "class" => {
                            if !attr.name.space.is_empty() || class_seen {
                                return (None, None);
                            }
                            class_seen = true;
                            if attr
                                .value
                                .chars()
                                .any(|c| (c as u32) >= 128 && c.is_whitespace())
                            {
                                classes_safe = false;
                            }
                            for class in attr
                                .value
                                .split([' ', '\t', '\n', '\r', '\x0c'])
                                .filter(|c| !c.is_empty())
                            {
                                if is_palette_class(class) {
                                    classes.insert(class.to_owned());
                                }
                                for (i, font_class) in EMBEDDED_FONT_CLASSES.iter().enumerate() {
                                    if class == *font_class && i as i32 > local_font {
                                        local_font = i as i32;
                                    }
                                }
                            }
                        }
                        "font-family" | "text-transform" | "direction" | "unicode-bidi" => {
                            return (None, None);
                        }
                        "style" => {
                            if !attr.name.space.is_empty() || !font_corpus_style_safe(&attr.value) {
                                return (None, None);
                            }
                        }
                        local => {
                            if local.starts_with("on") {
                                return (None, None);
                            }
                        }
                    }
                }
                if local_font >= 0 {
                    current.font = local_font;
                }
                stack.push(current);
            }
            Token::End { .. } => {
                if stack.len() == 1 {
                    // The wrapper's own end tag.
                    continue;
                }
                stack.pop();
            }
            Token::CharData(data) => {
                let current = *stack.last().expect("frame");
                if current.text && !current.ignore && !data.is_empty() {
                    if current.font < 0 {
                        return (None, None);
                    }
                    corpora[current.font as usize].push_str(&data);
                }
            }
            Token::Directive | Token::ProcInst => return (None, None),
            Token::Comment => {}
        }
    }
    if stack.len() != 1 {
        return (None, None);
    }
    let mut result = FontCorpora::new();
    for (i, corpus) in corpora.into_iter().enumerate() {
        if !corpus.is_empty() {
            result.insert(EMBEDDED_FONT_CLASSES[i], corpus);
        }
    }
    if result.contains_key("text")
        && !source.contains(r#"class="text""#)
        && !source.contains(r#"class="text "#)
        && !source.contains(r#"class="md""#)
        && !source.contains(r#"class="md "#)
    {
        return (None, None);
    }
    (Some(result), classes_safe.then_some(classes))
}

fn is_palette_class(class: &str) -> bool {
    [
        "fill-",
        "stroke-",
        "background-color-",
        "color-",
        "sketch-overlay-",
    ]
    .iter()
    .find_map(|prefix| class.strip_prefix(prefix))
    .is_some_and(super::color::is_theme_color)
}

/// font_corpus.go fontCorpusStyleSafe.
fn font_corpus_style_safe(style: &str) -> bool {
    if style.contains(['\\', '/', '{', '}']) {
        return false;
    }
    for declaration in style.split(';') {
        if declaration.trim().is_empty() {
            continue;
        }
        let Some((property, _)) = declaration.split_once(':') else {
            return false;
        };
        let property = property.trim().to_lowercase();
        if property.starts_with("font-variant") {
            return false;
        }
        if matches!(
            property.as_str(),
            "font"
                | "font-family"
                | "text-transform"
                | "direction"
                | "unicode-bidi"
                | "all"
                | "content"
        ) {
            return false;
        }
    }
    true
}

/// font_corpus.go restrictFontCorpora.
fn restrict_font_corpora(corpora: Option<&FontCorpora>, corpus: &str) -> Option<FontCorpora> {
    let mut corpora = corpora?.clone();
    let mut allowed = [false; 128];
    for &c in corpus.as_bytes() {
        if c >= 128 || c == 127 || (c < 32 && c != b'\n') {
            return None;
        }
        allowed[c as usize] = true;
    }
    for rendered in corpora.values_mut() {
        if rendered
            .chars()
            .any(|r| (r as u32) >= 128 || !allowed[r as usize])
        {
            *rendered = corpus.to_owned();
        }
    }
    Some(corpora)
}

pub(crate) use d2_rs_text::d2fonts::FontStyle;

/// d2fonts.Font.GetEncodedSubset for a family named as in d2target
/// (`FontFamily` string values).
fn encoded_subset(family: &str, style: FontStyle, corpus: &str) -> Result<String, String> {
    d2_rs_text::d2fonts::registered_encoded_subset(family, style, corpus)
        .ok_or_else(|| format!("font subset failed for {family:?} {style:?}"))
}

fn face_rule(hash: &str, class: &str, face: &str, src: &str) -> String {
    format!(
        "\n.{hash} .{class} {{\n\tfont-family: \"{hash}-font-{face}\";\n}}\n@font-face {{\n\tfont-family: {hash}-font-{face};\n\tsrc: url(\"{src}\");\n}}"
    )
}

/// d2svg.embedFonts.
pub(crate) fn embed_fonts(
    buf: &mut String,
    diagram_hash: &str,
    source: &str,
    font_family: Option<&str>,
    mono_font_family: Option<&str>,
    corpus: &str,
    corpora: Option<&FontCorpora>,
    custom_fonts: &d2_rs_text::d2fonts::CustomFonts,
) -> Result<(), String> {
    let mut corpus = corpus.to_owned();
    for text in native_markdown_texts(source) {
        corpus += &html_unescape(&text)?;
    }
    let corpora = restrict_font_corpora(corpora, &corpus);
    let family = font_family
        .filter(|f| !f.is_empty())
        .unwrap_or("SourceSansPro");
    let mono = mono_font_family
        .filter(|f| !f.is_empty())
        .unwrap_or("SourceCodePro");
    buf.push_str(r#"<style type="text/css"><![CDATA["#);
    let font = |buf: &mut String,
                triggers: &[&str],
                class: &'static str,
                face: &str,
                family: &str,
                style: FontStyle|
     -> Result<(), String> {
        let chosen = match &corpora {
            None => triggers
                .iter()
                .any(|t| source.contains(t))
                .then(|| corpus.clone()),
            Some(corpora) => corpora.get(class).cloned(),
        };
        if let Some(chosen) = chosen {
            buf.push_str(&face_rule(
                diagram_hash,
                class,
                face,
                &match custom_fonts.get_encoded_subset(
                    class.starts_with("text-mono"),
                    style,
                    &chosen,
                )? {
                    Some(subset) => subset,
                    None => encoded_subset(family, style, &chosen)?,
                },
            ));
        }
        Ok(())
    };
    let on_trigger = |buf: &mut String, triggers: &[&str], content: &str| {
        if triggers.iter().any(|t| source.contains(t)) {
            buf.push_str(content);
        }
    };
    font(
        buf,
        &[
            r#"class="text""#,
            r#"class="text "#,
            r#"class="md""#,
            r#"class="md "#,
        ],
        "text",
        "regular",
        family,
        FontStyle::Regular,
    )?;
    font(
        buf,
        &["text-semibold"],
        "text-semibold",
        "semibold",
        family,
        FontStyle::Semibold,
    )?;
    on_trigger(
        buf,
        &["text-underline"],
        "\n.text-underline {\n\ttext-decoration: underline;\n}",
    );
    on_trigger(
        buf,
        &["text-link"],
        "\n.text-link {\n\tfill: blue;\n}\n\n.text-link:visited {\n\tfill: purple;\n}",
    );
    on_trigger(
        buf,
        &["animated-connection"],
        "\n@keyframes dashdraw {\n\tfrom {\n\t\tstroke-dashoffset: 0;\n\t}\n}\n",
    );
    on_trigger(
        buf,
        &["animated-shape"],
        "\n@keyframes shapeappear {\n    0%, 100% { transform: translateY(0); filter: drop-shadow(0px 0px 0px rgba(0,0,0,0)); }\n    50% { transform: translateY(-4px); filter: drop-shadow(0px 12.6px 25.2px rgba(50,50,93,0.25)) drop-shadow(0px 7.56px 15.12px rgba(0,0,0,0.1)); }\n}\n.animated-shape {\n\tanimation: shapeappear 1s linear infinite;\n}\n",
    );
    on_trigger(
        buf,
        &["appendix-icon"],
        "\n.appendix-icon {\n\tfilter: drop-shadow(0px 0px 32px rgba(31, 36, 58, 0.1));\n}",
    );
    font(
        buf,
        &["text-bold", "<b>", "<strong>"],
        "text-bold",
        "bold",
        family,
        FontStyle::Bold,
    )?;
    font(
        buf,
        &["text-italic", "<em>", "<dfn>"],
        "text-italic",
        "italic",
        family,
        FontStyle::Italic,
    )?;
    font(
        buf,
        &["text-mono", "<pre>", "<code>", "<kbd>", "<samp>"],
        "text-mono",
        "mono",
        mono,
        FontStyle::Regular,
    )?;
    font(
        buf,
        &["text-mono-semibold"],
        "text-mono-semibold",
        "mono-semibold",
        mono,
        FontStyle::Semibold,
    )?;
    font(
        buf,
        &["text-mono-bold"],
        "text-mono-bold",
        "mono-bold",
        mono,
        FontStyle::Bold,
    )?;
    font(
        buf,
        &["text-mono-italic"],
        "text-mono-italic",
        "mono-italic",
        mono,
        FontStyle::Italic,
    )?;
    for (name, mode) in [
        ("bright", "darken"),
        ("normal", "color-burn"),
        ("dark", "overlay"),
        ("darker", "lighten"),
    ] {
        on_trigger(
            buf,
            &[&format!("sketch-overlay-{name}")],
            &format!(
                "\n.sketch-overlay-{name} {{\n\tfill: url(#streaks-{name}-{diagram_hash});\n\tmix-blend-mode: {mode};\n}}"
            ),
        );
    }
    buf.push_str("]]></style>");
    Ok(())
}

/// nativeMarkdownTextPattern:
/// `<text\b[^>]*\bclass="[^"]*\bmd-text\b[^"]*"[^>]*>([^<]*)</text>`.
fn native_markdown_texts(source: &str) -> Vec<String> {
    // Every match contains the literal `md-text`; without it, skip compiling
    // the pattern (most boards have no Markdown text).
    if !source.contains("md-text") {
        return Vec::new();
    }
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(r#"<text\b[^>]*\bclass="[^"]*\bmd-text\b[^"]*"[^>]*>([^<]*)</text>"#)
            .expect("pattern")
    });
    pattern
        .captures_iter(source)
        .map(|c| c[1].to_owned())
        .collect()
}

/// html.UnescapeString (golang.org/x/net/html escape.go), via the pinned
/// Go1.27 entity table port shared with the Goldmark projection.
fn html_unescape(text: &str) -> Result<String, String> {
    if !text.contains('&') {
        return Ok(text.to_owned());
    }
    let bytes = d2_rs_text::markdown::html::unescape_html(text.as_bytes());
    String::from_utf8(bytes)
        .map_err(|error| format!("html unescape produced invalid UTF-8: {error}"))
}
