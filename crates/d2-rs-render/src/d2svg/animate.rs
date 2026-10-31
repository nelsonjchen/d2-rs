// SPDX-License-Identifier: MPL-2.0
// Port of D2 v0.9.0 d2renderers/d2animate/d2animate.go.

use super::render::{BASE_STYLESHEET, RenderOpts, VERSION};
use crate::target::TargetDiagram;

const TRANSITION_DURATION_MS: i64 = 1;

/// d2animate.makeKeyframe.
fn make_keyframe(
    delay_ms: i64,
    duration_ms: i64,
    total_ms: i64,
    identifier: usize,
    diagram_hash: &str,
) -> String {
    let before =
        ((0.0f64).max((delay_ms - TRANSITION_DURATION_MS) as f64) / total_ms as f64) * 100.;
    let start = (delay_ms as f64 / total_ms as f64) * 100.;
    let end = ((delay_ms + duration_ms - TRANSITION_DURATION_MS) as f64 / total_ms as f64) * 100.;
    if end.ceil() as i64 == 100 {
        return format!(
            "@keyframes d2Transition-{diagram_hash}-{identifier} {{\n\t\t0%, {before:.6}% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{start:.6}%, {:.6}% {{\n\t\t\t\topacity: 1;\n\t\t}}\n}}",
            end.ceil()
        );
    }
    let after = ((delay_ms + duration_ms) as f64 / total_ms as f64) * 100.;
    format!(
        "@keyframes d2Transition-{diagram_hash}-{identifier} {{\n\t\t0%, {before:.6}% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{start:.6}%, {end:.6}% {{\n\t\t\t\topacity: 1;\n\t\t}}\n\t\t{after:.6}%, 100% {{\n\t\t\t\topacity: 0;\n\t\t}}\n}}"
    )
}

/// d2animate.Wrap.
pub(crate) fn wrap(
    root: &TargetDiagram,
    svgs: &[Vec<u8>],
    opts: &RenderOpts,
    interval_ms: i64,
) -> Result<Vec<u8>, String> {
    super::target_validation::validate(root)?;
    let ((tlx, tly), (brx, bry)) = root.nested_go_bounding_box()?;
    let pad = opts.pad.unwrap_or(super::render::DEFAULT_PADDING);
    let (left, top) = (tlx - pad, tly - pad);
    let (width, height) = (brx - tlx + pad * 2, bry - tly + pad * 2);
    let dimensions = opts
        .scale
        .map(|scale| {
            format!(
                r#" width="{}" height="{}""#,
                (scale * width as f64).ceil() as i64,
                (scale * height as f64).ceil() as i64
            )
        })
        .unwrap_or_default();
    let mut buf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" d2Version="{VERSION}" preserveAspectRatio="xMinYMin meet" viewBox="0 0 {width} {height}"{dimensions}>"#
    );
    buf += &format!(
        r#"<svg class="d2-svg" width="{width}" height="{height}" viewBox="{left} {top} {width} {height}">"#
    );
    let mut svgs_str = String::new();
    for svg in svgs {
        svgs_str += &String::from_utf8_lossy(svg);
        svgs_str.push(' ');
    }
    let diagram_hash = root.hash_id(opts.salt.as_deref())?;
    super::fonts::embed_fonts(
        &mut buf,
        &diagram_hash,
        &svgs_str,
        root.font_family.as_deref(),
        root.mono_font_family.as_deref(),
        &root.get_nested_corpus()?,
        None,
        &opts.custom_fonts,
    )?;
    let theme_stylesheet = super::theme_css::theme_css(
        &diagram_hash,
        opts.theme_id,
        opts.dark_theme_id,
        opts.theme_overrides.as_ref(),
        opts.dark_theme_overrides.as_ref(),
    )?;
    buf += &format!(
        r#"<style type="text/css"><![CDATA[{BASE_STYLESHEET}{theme_stylesheet}]]></style>"#
    );
    if opts.sketch == Some(true) {
        super::sketch::define_fill_patterns(&mut buf, &diagram_hash);
    }
    buf += r#"<style type="text/css"><![CDATA["#;
    let total = svgs.len() as i64 * interval_ms;
    for i in 0..svgs.len() {
        buf += &make_keyframe(i as i64 * interval_ms, interval_ms, total, i, &diagram_hash);
    }
    buf += "]]></style>";
    for (i, svg) in svgs.iter().enumerate() {
        let s = String::from_utf8_lossy(svg);
        buf += &s.replacen(
            "<g",
            &format!(r#"<g style="animation: d2Transition-{diagram_hash}-{i} {total}ms infinite""#),
            1,
        );
    }
    buf += "</svg></svg>";
    Ok(buf.into_bytes())
}
