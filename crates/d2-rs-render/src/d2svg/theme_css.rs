// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/d2svg.go ThemeCSS/singleThemeRulesets/
// blendMode and d2renderers/d2svg/theme.go (palette rule pruning).

use std::collections::BTreeSet;

use super::color;
use super::element::Theme;
use crate::target::TargetThemeOverrides;

/// d2svg.ThemeCSS.
pub(crate) fn theme_css(
    diagram_hash: &str,
    theme_id: Option<i64>,
    dark_theme_id: Option<i64>,
    overrides: Option<&TargetThemeOverrides>,
    dark_overrides: Option<&TargetThemeOverrides>,
) -> Result<String, String> {
    super::paint_validation::theme(overrides, "theme override")?;
    super::paint_validation::theme(dark_overrides, "dark theme override")?;
    let mut out = single_theme_rulesets(diagram_hash, theme_id.unwrap_or(0), overrides)?;
    if let Some(dark) = dark_theme_id {
        let dark_out = single_theme_rulesets(diagram_hash, dark, dark_overrides)?;
        out += &format!("@media screen and (prefers-color-scheme:dark){{{dark_out}}}");
    }
    Ok(out)
}

fn single_theme_rulesets(
    diagram_hash: &str,
    theme_id: i64,
    overrides: Option<&TargetThemeOverrides>,
) -> Result<String, String> {
    let mut theme = Theme::find(theme_id);
    theme.apply_overrides(overrides);
    let c = &theme.colors;
    let palette: [(&str, &str); 18] = [
        ("N1", &c.n1),
        ("N2", &c.n2),
        ("N3", &c.n3),
        ("N4", &c.n4),
        ("N5", &c.n5),
        ("N6", &c.n6),
        ("N7", &c.n7),
        ("B1", &c.b1),
        ("B2", &c.b2),
        ("B3", &c.b3),
        ("B4", &c.b4),
        ("B5", &c.b5),
        ("B6", &c.b6),
        ("AA2", &c.aa2),
        ("AA4", &c.aa4),
        ("AA5", &c.aa5),
        ("AB4", &c.ab4),
        ("AB5", &c.ab5),
    ];
    let mut out = String::new();
    for property in ["fill", "stroke", "background-color", "color"] {
        for (code, value) in palette {
            out += &format!("\n\t\t.{diagram_hash} .{property}-{code}{{{property}:{value};}}");
        }
    }
    out += &format!(".appendix text.text{{fill:{}}}", c.n1);
    out += &format!(
        ".md{{--color-fg-default:{};--color-fg-muted:{};--color-fg-subtle:{};--color-canvas-default:{};--color-canvas-subtle:{};--color-border-default:{};--color-border-muted:{};--color-neutral-muted:{};--color-accent-fg:{};--color-accent-emphasis:{};--color-attention-subtle:{};--color-danger-fg:{};}}",
        c.n1, c.n2, c.n3, c.n7, c.n6, c.b1, c.b2, c.n6, c.b2, c.b2, c.n2, "red",
    );
    // Sketch overlays: B, AA, AB, then neutrals, in Go's order.
    let sketch_order: [(&str, &str); 18] = [
        ("B1", &c.b1),
        ("B2", &c.b2),
        ("B3", &c.b3),
        ("B4", &c.b4),
        ("B5", &c.b5),
        ("B6", &c.b6),
        ("AA2", &c.aa2),
        ("AA4", &c.aa4),
        ("AA5", &c.aa5),
        ("AB4", &c.ab4),
        ("AB5", &c.ab5),
        ("N1", &c.n1),
        ("N2", &c.n2),
        ("N3", &c.n3),
        ("N4", &c.n4),
        ("N5", &c.n5),
        ("N6", &c.n6),
        ("N7", &c.n7),
    ];
    for (code, value) in sketch_order {
        let lc = color::luminance_category(value)?;
        out += &format!(
            ".sketch-overlay-{code}{{fill:url(#streaks-{lc}-{diagram_hash});mix-blend-mode:{}}}",
            blend_mode(lc)
        );
    }
    if theme.is_dark() {
        out += ".light-code{display: none}.dark-code{display: block}";
    } else {
        out += ".light-code{display: block}.dark-code{display: none}";
    }
    Ok(out)
}

fn blend_mode(lc: &str) -> &'static str {
    match lc {
        "bright" => "darken",
        "normal" => "color-burn",
        "dark" => "overlay",
        "darker" => "lighten",
        _ => panic!("invalid luminance category"),
    }
}

/// theme.go pruneThemeCSS.
pub(crate) fn prune_theme_css(stylesheet: &str, sources: &[&str]) -> String {
    prune_theme_css_with_classes(stylesheet, None, sources)
}

/// theme.go pruneThemeCSSWithClasses.
pub(crate) fn prune_theme_css_with_classes(
    stylesheet: &str,
    used: Option<BTreeSet<String>>,
    sources: &[&str],
) -> String {
    if stylesheet.is_empty() {
        return String::new();
    }
    let mut used = used.unwrap_or_default();
    used.insert("stroke-B2".into());
    for source in sources {
        match scan_classes(source) {
            Some(classes) => {
                used.extend(classes.into_iter().filter(|class| is_palette_class(class)))
            }
            None => return stylesheet.to_owned(),
        }
    }
    prune_theme_rules(stylesheet, &used)
}

/// Collects every `class` attribute value on start elements, the way the
/// encoding/xml token loop in pruneThemeCSSWithClasses sees them. Returns
/// None where Go keeps the full stylesheet: a decode error, a <script>
/// element, event handlers, SMIL class animation, external <use> or
/// javascript: hrefs.
pub(crate) fn scan_classes(source: &str) -> Option<Vec<String>> {
    let mut classes = Vec::new();
    for token in super::xml::tokens(source)? {
        let super::xml::Token::Start { name, attrs } = token else {
            continue;
        };
        if name.local.eq_ignore_ascii_case("script") {
            return None;
        }
        for attr in &attrs {
            let lower = attr.name.local.to_lowercase();
            if lower.starts_with("on")
                || (lower == "attributename" && attr.value.trim().eq_ignore_ascii_case("class"))
            {
                return None;
            }
            if lower == "href" {
                let value = attr.value.trim();
                if (name.local.eq_ignore_ascii_case("use") && !value.starts_with('#'))
                    || value.to_lowercase().starts_with("javascript:")
                {
                    return None;
                }
            }
            if attr.name.local == "class" {
                classes.extend(attr.value.split_whitespace().map(str::to_owned));
            }
        }
    }
    Some(classes)
}

/// theme.go pruneThemeRules.
fn prune_theme_rules(stylesheet: &str, used: &BTreeSet<String>) -> String {
    let bytes = stylesheet.as_bytes();
    let mut out = String::new();
    let mut copied = 0usize;
    let mut scan = 0usize;
    while scan < bytes.len() {
        let Some(rel) = stylesheet[scan..].find('{') else {
            break;
        };
        let open = scan + rel;
        scan = open + 1;
        let Some((mut start, class)) = theme_rule_selector(stylesheet, open) else {
            continue;
        };
        let Some(end_rel) = stylesheet[scan..].find(['{', '}']) else {
            break;
        };
        let end = scan + end_rel;
        if bytes[end] != b'}' {
            continue;
        }
        scan = end + 1;
        if !is_palette_class(class) || used.contains(class) {
            continue;
        }
        while start > copied && theme_rule_whitespace(bytes[start - 1]) {
            start -= 1;
        }
        out.push_str(&stylesheet[copied..start]);
        copied = scan;
    }
    if copied == 0 {
        return stylesheet.to_owned();
    }
    out.push_str(&stylesheet[copied..]);
    out
}

fn theme_rule_selector(stylesheet: &str, open: usize) -> Option<(usize, &str)> {
    let bytes = stylesheet.as_bytes();
    let mut start = open;
    while start > 0 && theme_rule_word(bytes[start - 1]) {
        start -= 1;
    }
    if start == 0 || bytes[start - 1] != b'.' {
        return None;
    }
    let class = &stylesheet[start..open];
    let value;
    if let Some(rest) = class.strip_prefix("sketch-overlay-") {
        value = rest;
        start -= 1;
    } else {
        value = ["fill-", "stroke-", "background-color-", "color-"]
            .iter()
            .find_map(|prefix| class.strip_prefix(prefix))
            .unwrap_or("");
        if start < 3 || bytes[start - 2] != b' ' {
            return None;
        }
        let end = start - 2;
        start = end;
        while start > 0 && theme_rule_word(bytes[start - 1]) {
            start -= 1;
        }
        if start == end || start == 0 || bytes[start - 1] != b'.' {
            return None;
        }
        start -= 1;
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    Some((start, class))
}

fn theme_rule_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn theme_rule_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
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
    .is_some_and(color::is_theme_color)
}
