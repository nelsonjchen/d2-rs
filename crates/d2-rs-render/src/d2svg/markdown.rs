// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2svg/markdown.go (markdownRenderer) over
// lib/textmeasure LayoutMarkdown and MarkdownLayout.SVG
// (lib/textmeasure/markdown_svg.go), which live in the submodules below.

use std::fmt::Write as _;

use ruler::FontFamily;

use super::color;
use super::element::{ThemableElement, Theme, resolve_theme_color};
use super::geo::Point;
use super::svgtext::{escape_text, format_float};
use crate::target::{TargetConnection, TargetShape};

use d2_rs_text::markdown::html_tree as dom;
#[path = "markdown/measure.rs"]
mod measure;
#[path = "markdown/paint.rs"]
mod paint;
#[path = "markdown/ruler.rs"]
mod ruler;

pub(crate) use paint::MarkdownLayout;
use paint::{ColorRole, PrimitiveKind};

/// textmeasure.MarkdownSVGPaint.
#[derive(Clone, Debug, Default)]
struct SvgPaint {
    class: String,
    color: String,
}

/// d2fonts family names as the d2target diagram stores them.
fn family_from_name(name: Option<&str>, default: FontFamily) -> Result<FontFamily, String> {
    match name {
        None | Some("") => Ok(default),
        Some("SourceSansPro") => Ok(FontFamily::SourceSansPro),
        Some("SourceCodePro") => Ok(FontFamily::SourceCodePro),
        Some("HandDrawn") => Ok(FontFamily::HandDrawn),
        Some("custom") => Ok(FontFamily::Custom),
        Some("customMono") => Ok(FontFamily::CustomMono),
        Some(other) => Err(format!("unsupported: markdown font family {other:?}")),
    }
}

/// d2svg.markdownRenderer.
pub(crate) struct MarkdownRenderer<'t> {
    font_family: Option<String>,
    mono_font_family: Option<String>,
    inline_theme: Option<&'t Theme>,
    ruler: ruler::Ruler,
    corpus: String,
}

impl<'t> MarkdownRenderer<'t> {
    pub(crate) fn new(
        font_family: Option<&str>,
        mono_font_family: Option<&str>,
        inline_theme: Option<&'t Theme>,
    ) -> Self {
        Self {
            font_family: font_family.map(str::to_owned),
            mono_font_family: mono_font_family.map(str::to_owned),
            inline_theme,
            ruler: ruler::Ruler::new(),
            corpus: String::new(),
        }
    }

    pub(crate) fn set_custom_fonts(&mut self, fonts: &d2_rs_text::d2fonts::CustomFonts) {
        self.ruler.set_custom_fonts(fonts);
        if !fonts.regular.is_empty()
            && self.font_family.as_deref().unwrap_or("SourceSansPro") == "SourceSansPro"
        {
            self.font_family = Some("custom".into());
        }
        if !fonts.mono.is_empty()
            && self.mono_font_family.as_deref().unwrap_or("SourceCodePro") == "SourceCodePro"
        {
            self.mono_font_family = Some("customMono".into());
        }
    }

    /// markdownRenderer.inlineTheme (renderPositionedTooltip paints with it).
    pub(crate) fn inline_theme(&self) -> Option<&'t Theme> {
        self.inline_theme
    }

    /// markdownRenderer.layout.
    pub(crate) fn layout(
        &mut self,
        markdown: &str,
        font_name: &str,
        font_size: i64,
    ) -> Result<MarkdownLayout, String> {
        let mut font_family =
            family_from_name(self.font_family.as_deref(), FontFamily::SourceSansPro)?;
        let mono_font_family =
            family_from_name(self.mono_font_family.as_deref(), FontFamily::SourceCodePro)?;
        if font_name.eq_ignore_ascii_case("mono") {
            font_family = mono_font_family;
        } else if !font_name.is_empty() && !font_name.eq_ignore_ascii_case("default") {
            // fontToFamily(strings.ToLower(fontName)) over D2_FONT_TO_FAMILY.
            match font_name.to_lowercase().as_str() {
                "default" => font_family = FontFamily::SourceSansPro,
                "mono" => font_family = FontFamily::SourceCodePro,
                _ => {}
            }
        }
        let layout = paint::layout_markdown(
            markdown,
            &mut self.ruler,
            font_family,
            mono_font_family,
            font_size,
        )?;
        self.corpus.push_str(&layout.corpus);
        Ok(layout)
    }

    /// markdownRenderer.render.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &self,
        layout: &MarkdownLayout,
        x: f64,
        y: f64,
        width: i64,
        height: i64,
        foreground: &str,
        background: &str,
        disable_links: bool,
        underline: bool,
    ) -> String {
        let mut out = format!(
            r#"<svg x="{}" y="{}" width="{width}" height="{height}" viewBox="0 0 {width} {height}" overflow="hidden">"#,
            format_float(x),
            format_float(y),
        );
        if !background.is_empty() && background != "none" && background != "transparent" {
            let mut el = ThemableElement::new("rect", self.inline_theme);
            el.width = width as f64;
            el.height = height as f64;
            el.fill = background.to_owned();
            out.push_str(&el.render());
        }
        let role_paint = |role: ColorRole| -> SvgPaint {
            match role {
                ColorRole::None => SvgPaint::default(),
                ColorRole::Foreground => self.paint(foreground, "fill"),
                ColorRole::ForegroundStroke => self.paint(foreground, "stroke"),
                ColorRole::Muted => self.paint("N2", "fill"),
                ColorRole::MutedStroke => self.paint("N2", "stroke"),
                ColorRole::Accent => self.paint("B2", "fill"),
                ColorRole::Border => self.paint("B1", "fill"),
                ColorRole::BorderMuted => self.paint("B2", "fill"),
                ColorRole::Canvas => self.paint("N7", "fill"),
                ColorRole::CanvasSubtle | ColorRole::NeutralMuted => self.paint("N6", "fill"),
            }
        };
        out.push_str(&layout_svg(layout, &role_paint, disable_links, underline));
        out.push_str("</svg>");
        out
    }

    fn paint(&self, value: &str, property: &str) -> SvgPaint {
        let value = if value.is_empty() { "N1" } else { value };
        if color::is_theme_color(value) {
            let mut paint = SvgPaint {
                class: format!("{property}-{value}"),
                color: String::new(),
            };
            if let Some(theme) = self.inline_theme {
                paint.color = resolve_theme_color(theme, value).to_owned();
            }
            return paint;
        }
        if color::is_gradient(value) {
            return SvgPaint {
                class: String::new(),
                color: format!("url('#{}')", color::unique_gradient_id(value)),
            };
        }
        SvgPaint {
            class: String::new(),
            color: value.to_owned(),
        }
    }

    /// drawShape's Markdown label: markdown.layout + markdown.render.
    pub(crate) fn render_shape_label(
        &mut self,
        s: &TargetShape,
        label_tl: Point,
    ) -> Result<String, String> {
        let layout = self.layout(&s.label, &s.font_family, s.font_size)?;
        Ok(self.render(
            &layout,
            label_tl.x,
            label_tl.y,
            s.label_width,
            s.label_height,
            &s.get_font_color(),
            &s.fill,
            !s.link.is_empty(),
            s.underline,
        ))
    }

    /// drawConnection's Markdown label.
    pub(crate) fn render_connection_label(
        &mut self,
        c: &TargetConnection,
        label_tl: Point,
    ) -> Result<String, String> {
        let layout = self.layout(&c.label, &c.font_family, c.font_size)?;
        Ok(self.render(
            &layout,
            label_tl.x,
            label_tl.y,
            c.label_width,
            c.label_height,
            &c.get_font_color(),
            c.fill_str(),
            !c.link.is_empty(),
            c.underline,
        ))
    }

    /// markdownRenderer.generatedCorpus.
    pub(crate) fn generated_corpus(&self) -> String {
        self.corpus.clone()
    }
}

/// textmeasure.mdFloat.
fn md_float(v: f64) -> String {
    let v = if v.abs() < 0.0005 { 0.0 } else { v };
    format!("{v:.3}")
}

/// textmeasure.SafeMarkdownLink.
fn safe_markdown_link(link: &str) -> &str {
    let link = link.trim();
    if link.is_empty() {
        return "";
    }
    let normalized: String = link
        .chars()
        .filter(|&r| !r.is_control() && !r.is_whitespace())
        .collect();
    if is_dangerous_url(normalized.as_bytes()) {
        return "";
    }
    link
}

/// goldmark renderer/html.IsDangerousURL.
fn is_dangerous_url(url: &[u8]) -> bool {
    let has_prefix = |s: &[u8], prefix: &[u8]| {
        s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    };
    if has_prefix(url, b"data:image/") && url.len() >= 11 {
        let v = &url[11..];
        return !(has_prefix(v, b"png;")
            || has_prefix(v, b"gif;")
            || has_prefix(v, b"jpeg;")
            || has_prefix(v, b"webp;"));
    }
    has_prefix(url, b"javascript:")
        || has_prefix(url, b"vbscript:")
        || has_prefix(url, b"file:")
        || has_prefix(url, b"data:")
}

/// MarkdownLayout.SVG with d2svg's role paints and the default font classes.
fn layout_svg(
    layout: &MarkdownLayout,
    role_paint: &dyn Fn(ColorRole) -> SvgPaint,
    disable_links: bool,
    underline: bool,
) -> String {
    let mut out = String::from(r#"<g class="md md-native">"#);
    for p in &layout.primitives {
        let mut classes: Vec<&str> = Vec::new();
        let fill_paint = role_paint(p.fill_role);
        let stroke_paint = role_paint(p.stroke_role);
        if p.kind == PrimitiveKind::Text {
            classes.push("md-text");
            classes.push(p.font.class());
        }
        if !fill_paint.class.is_empty() {
            classes.push(&fill_paint.class);
        }
        if !stroke_paint.class.is_empty() {
            classes.push(&stroke_paint.class);
        }
        let class_attr = if classes.is_empty() {
            String::new()
        } else {
            format!(r#" class="{}""#, escape_text(&classes.join(" ")))
        };
        let mut fill = fill_paint.color.clone();
        let mut stroke = stroke_paint.color.clone();
        let link = safe_markdown_link(&p.link);
        let linked = !link.is_empty() && !disable_links;
        if linked {
            let escaped = escape_text(link);
            write!(out, r#"<a href="{escaped}" xlink:href="{escaped}">"#).unwrap();
            if !p.link_title.is_empty() {
                write!(out, "<title>{}</title>", escape_text(&p.link_title)).unwrap();
            }
        }
        match p.kind {
            PrimitiveKind::Rect => {
                write!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"{class_attr}"#,
                    md_float(p.x),
                    md_float(p.y),
                    md_float(p.width),
                    md_float(p.height),
                    md_float(p.radius)
                )
                .unwrap();
                if fill.is_empty() {
                    fill = "none".into();
                }
                write!(out, r#" fill="{}""#, escape_text(&fill)).unwrap();
                if !stroke.is_empty() && p.stroke_width > 0.0 {
                    write!(
                        out,
                        r#" stroke="{}" stroke-width="{}""#,
                        escape_text(&stroke),
                        md_float(p.stroke_width)
                    )
                    .unwrap();
                }
                out.push_str("/>");
            }
            PrimitiveKind::Line => {
                if stroke.is_empty() {
                    stroke = "currentColor".into();
                }
                write!(
                    out,
                    r#"<line x1="{}" y1="{}" x2="{}" y2="{}"{class_attr} stroke="{}" stroke-width="{}"/>"#,
                    md_float(p.x),
                    md_float(p.y),
                    md_float(p.x2),
                    md_float(p.y2),
                    escape_text(&stroke),
                    md_float(p.stroke_width)
                )
                .unwrap();
            }
            PrimitiveKind::Text => {
                if fill.is_empty() {
                    fill = "currentColor".into();
                }
                write!(
                    out,
                    r#"<text x="{}" y="{}"{class_attr} fill="{}" font-size="{}" xml:space="preserve""#,
                    md_float(p.x),
                    md_float(p.y),
                    escape_text(&fill),
                    md_float(p.font_size)
                )
                .unwrap();
                let decoration = match (underline, p.line_through) {
                    (false, false) => "",
                    (true, false) => "underline",
                    (false, true) => "line-through",
                    (true, true) => "underline line-through",
                };
                if !decoration.is_empty() {
                    write!(out, r#" text-decoration="{decoration}""#).unwrap();
                }
                if p.synthetic_bold {
                    out.push_str(r#" font-weight="bold""#);
                }
                if p.synthetic_italic {
                    out.push_str(r#" font-style="italic""#);
                }
                if p.text_length {
                    write!(
                        out,
                        r#" textLength="{}" lengthAdjust="spacingAndGlyphs""#,
                        md_float(p.width)
                    )
                    .unwrap();
                }
                write!(out, ">{}</text>", escape_text(&p.text)).unwrap();
            }
        }
        if linked {
            out.push_str("</a>");
        }
    }
    out.push_str("</g>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(label: &str, font_size: i64, width: i64, height: i64) -> (MarkdownLayout, String) {
        let theme = Theme::find(0);
        let mut renderer = MarkdownRenderer::new(None, None, Some(&theme));
        let layout = renderer
            .layout(label, "DEFAULT", font_size)
            .expect("layout");
        let svg = renderer.render(
            &layout,
            0.0,
            166.0,
            width,
            height,
            "N1",
            "transparent",
            false,
            false,
        );
        assert_eq!(renderer.generated_corpus(), layout.corpus);
        (layout, svg)
    }

    // Fragments of the pinned e2etests/testdata/stable/*/dagre/sketch.exp.svg
    // goldens; the layout dimensions are the board.exp.json label sizes that
    // MeasureMarkdown produced for graph layout.
    #[test]
    fn inline_code_matches_pinned_golden() {
        let (layout, svg) = render("`code`", 16, 46, 24);
        assert_eq!((layout.width, layout.height), (46, 24));
        assert_eq!(
            svg,
            r##"<svg x="0" y="166" width="46" height="24" viewBox="0 0 46 24" overflow="hidden"><g class="md md-native"><rect x="0.000" y="2.280" width="43.499" height="22.440" rx="6.000" class="fill-N6" fill="#EEF1F8"/><text x="5.440" y="18.000" class="md-text text-mono fill-N1" fill="#0A0F25" font-size="13.600" xml:space="preserve">code</text></g></svg>"##
        );
    }

    #[test]
    fn fenced_code_block_matches_pinned_golden() {
        let (layout, svg) = render(
            "```\n{\n\tfenced: \"block\",\n\tof: \"json\",\n}\n```",
            16,
            196,
            111,
        );
        assert_eq!((layout.width, layout.height), (196, 111));
        assert_eq!(
            svg,
            r##"<svg x="0" y="166" width="196" height="111" viewBox="0 0 196 111" overflow="hidden"><g class="md md-native"><rect x="0.000" y="0.000" width="196.000" height="110.880" rx="6.000" class="fill-N6" fill="#EEF1F8"/><text x="16.000" y="30.360" class="md-text text-mono fill-N1" fill="#0A0F25" font-size="13.600" xml:space="preserve">{</text><text x="16.000" y="50.080" class="md-text text-mono fill-N1" fill="#0A0F25" font-size="13.600" xml:space="preserve">    fenced: &#34;block&#34;,</text><text x="16.000" y="69.800" class="md-text text-mono fill-N1" fill="#0A0F25" font-size="13.600" xml:space="preserve">    of: &#34;json&#34;,</text><text x="16.000" y="89.520" class="md-text text-mono fill-N1" fill="#0A0F25" font-size="13.600" xml:space="preserve">}</text></g></svg>"##
        );
    }

    #[test]
    fn nested_list_links_match_pinned_golden() {
        let label = "\n- [Overview](#overview) ok _this is all measured_\n\t- [Philosophy](#philosophy)\n\t- [Inline HTML](#html)\n";
        let (layout, svg) = render(label, 16, 245, 76);
        assert_eq!((layout.width, layout.height), (245, 76));
        assert_eq!(
            svg,
            r##"<svg x="0" y="166" width="245" height="76" viewBox="0 0 245 76" overflow="hidden"><g class="md md-native"><rect x="15.000" y="11.000" width="5.000" height="5.000" rx="2.500" class="fill-N1" fill="#0A0F25"/><a href="#overview" xlink:href="#overview"><text x="32.000" y="18.000" class="md-text text fill-B2" fill="#0D32B2" font-size="16.000" xml:space="preserve">Overview</text></a><text x="94.406" y="18.000" class="md-text text fill-N1" fill="#0A0F25" font-size="16.000" xml:space="preserve"> ok </text><text x="117.406" y="18.000" class="md-text text-italic fill-N1" fill="#0A0F25" font-size="16.000" xml:space="preserve">this is all measured</text><rect x="47.000" y="35.000" width="5.000" height="5.000" rx="2.500" class="stroke-N1" fill="none" stroke="#0A0F25" stroke-width="1.000"/><a href="#philosophy" xlink:href="#philosophy"><text x="64.000" y="42.000" class="md-text text fill-B2" fill="#0D32B2" font-size="16.000" xml:space="preserve">Philosophy</text></a><rect x="47.000" y="63.000" width="5.000" height="5.000" rx="2.500" class="stroke-N1" fill="none" stroke="#0A0F25" stroke-width="1.000"/><a href="#html" xlink:href="#html"><text x="64.000" y="70.000" class="md-text text fill-B2" fill="#0D32B2" font-size="16.000" xml:space="preserve">Inline HTML</text></a></g></svg>"##
        );
    }

    #[test]
    fn unsupported_html_element_is_an_error_like_go() {
        let theme = Theme::find(0);
        let mut renderer = MarkdownRenderer::new(None, None, Some(&theme));
        let error = renderer.layout("<div>x</div>", "", 16).unwrap_err();
        assert_eq!(
            error,
            "native Markdown SVG does not support HTML element <div>"
        );
    }

    #[test]
    fn dangerous_links_are_not_emitted() {
        assert_eq!(safe_markdown_link(" java\tscript:alert(1)"), "");
        assert_eq!(
            safe_markdown_link("data:image/png;base64,AA"),
            "data:image/png;base64,AA"
        );
        assert_eq!(safe_markdown_link("DATA:text/html,x"), "");
        assert_eq!(
            safe_markdown_link(" https://d2lang.com "),
            "https://d2lang.com"
        );
    }

    #[test]
    fn html_tree_follows_x_net_html_for_goldmark_output() {
        let (dom, body) =
            dom::parse_document("<p>a &amp; b &copy</p>\n<ul>\n<li>x<li>y</ul>\n<pre>\nz</pre>");
        let kids: Vec<_> = dom
            .children(body)
            .map(|id| dom.node(id).data.clone())
            .collect();
        assert_eq!(kids, ["p", "\n", "ul", "\n", "pre"]);
        let p = dom.node(body).first_child.unwrap();
        assert_eq!(
            dom.node(dom.node(p).first_child.unwrap()).data,
            "a & b \u{a9}"
        );
        let ul = dom.children(body).nth(2).unwrap();
        assert_eq!(
            dom.children(ul)
                .filter(|&li| dom.is_element(li, "li"))
                .count(),
            2
        );
        let pre = dom.node(body).last_child.unwrap();
        assert_eq!(dom.markdown_node_text(pre), "z");
    }
}
