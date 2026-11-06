// SPDX-License-Identifier: MPL-2.0
// Complete natural viewport investigation against original LayoutMarkdown.
use d2_rs_render::markdown_measure::MarkdownMeasurer;
use d2_rs_text::measure::{self as fonts, TrueTypeFont};
use std::io::{self, Read};
fn main() {
    let regular = fonts::bundled_source_sans_pro_regular().unwrap();
    let bold = fonts::bundled_source_sans_pro_bold().unwrap();
    let italic = fonts::bundled_source_sans_pro_italic().unwrap();
    let semibold = fonts::bundled_source_sans_pro_semibold().unwrap();
    let mono = fonts::bundled_source_code_pro_regular().unwrap();
    let mono_bold = fonts::bundled_source_code_pro_bold().unwrap();
    let mono_italic = fonts::bundled_source_code_pro_italic().unwrap();
    let mono_semibold = TrueTypeFont::parse(include_bytes!(
        "../../d2-rs-text/assets/SourceCodePro-Semibold.ttf"
    ))
    .unwrap();
    let hand = fonts::bundled_fuzzy_bubbles_regular().unwrap();
    let hand_bold = fonts::bundled_fuzzy_bubbles_bold().unwrap();
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let requests: Vec<serde_json::Value> = serde_json::from_str(&input).unwrap();
    let rows: Vec<_> = requests
        .iter()
        .map(|request| {
            let label = request["Markdown"].as_str().unwrap();
            let labels = std::collections::BTreeMap::from([(d2_rs_core::NodeId(0), label.to_owned())]);
            let styles = std::collections::BTreeMap::from([(d2_rs_core::NodeId(0),
                d2_rs_language::compiler::SourceNodeStyle { markdown: true, ..Default::default() })]);
            if let Err(error) = d2_rs_render::validate_markdown_labels(&labels, &styles) {
                return serde_json::json!({"request":request,"error":error.to_string()});
            }
            let (font, bold_font, italic_font, semibold_font) =
                match request["Family"].as_str().unwrap() {
                    "mono" => (&mono, &mono_bold, &mono_italic, &mono_semibold),
                    "hand" => (&hand, &hand_bold, &hand, &hand_bold),
                    _ => (&regular, &bold, &italic, &semibold),
                };
            let measurer = MarkdownMeasurer {
                font,
                bold_font,
                italic_font,
                semibold_font,
                mono_font: &mono,
                mono_bold_font: &mono_bold,
                mono_italic_font: &mono_italic,
                mono_semibold_font: &mono_semibold,
            };
            let size = measurer.markdown_dimensions(
                request["Markdown"].as_str().unwrap(),
                request["FontSize"].as_u64().unwrap() as u32,
                false,
                false,
                request["Family"] == "mono",
            );
            serde_json::json!({"request":request,"width":size.width,"height":size.height,"rendered_html":d2_rs_text::markdown::html::render_html(request["Markdown"].as_str().unwrap().as_bytes())})
        })
        .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
