// SPDX-License-Identifier: MPL-2.0
//! Upstream 8558856dc d2svgimport/text.go: positioned SVG text, bundled
//! face selection, bounded shaping and shared glyph outlines.
use super::{
    svg_geometry::Geometry, svg_path::Command, svg_scene::Work, svg_style::Style,
    svg_xml_document::Document,
};
use d2_rs_fontface::{ParsedFace, ShapeFace, ShapeLimits, ShapingWorkspace};
#[derive(Default)]
pub struct Fonts {
    faces: std::collections::BTreeMap<&'static str, ParsedFace>,
    shaping: ShapingWorkspace,
}
impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgTextFonts")
            .field("faces", &self.faces.len())
            .finish()
    }
}
pub fn frozen(document: &Document, mathjax: bool, index: usize) -> bool {
    mathjax
        || document.elements[index]
            .attributes
            .metadata
            .get("data-variant")
            .is_some_and(|v| !v.is_empty())
}
pub fn validate(source: &str, document: &Document, index: usize) -> Result<(), String> {
    let el = &document.elements[index];
    if el.name == "tspan"
        && (el
            .parent
            .is_none_or(|p| document.elements[p].name != "text")
            || ["x", "y"]
                .iter()
                .any(|k| el.attributes.values.get(*k).is_none_or(String::is_empty)))
    {
        return Err(format!(
            "d2svgimport: {source}: a <tspan> must be an explicitly positioned child of <text>"
        ));
    }
    for &child in &el.children {
        if super::svg_compile::non_rendering(&document.elements[child].name) {
            continue;
        }
        if el.name != "text"
            || document.elements[child].name != "tspan"
            || !String::from_utf8_lossy(&el.text).trim().is_empty()
        {
            return Err(format!(
                "d2svgimport: {source}: mixed or flowing SVG text is unsupported"
            ));
        }
    }
    Ok(())
}
fn family(families: &str) -> &'static str {
    for part in families.split(',') {
        match part
            .trim()
            .trim_matches(['\'', '"'])
            .to_lowercase()
            .as_str()
        {
            "sourcesanspro" | "source sans pro" | "sans-serif" => return "SourceSansPro",
            "sourcecodepro" | "source code pro" | "monospace" => return "SourceCodePro",
            "handdrawn" | "hand drawn" | "cursive" => return "HandDrawn",
            _ => {}
        }
    }
    "SourceSansPro"
}
pub fn geometry(
    source: &str,
    document: &Document,
    index: usize,
    style: &Style,
    work: &mut Work,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Geometry, String> {
    let el = &document.elements[index];
    if !style.paint.visible || style.font_size == 0. || el.text.is_empty() {
        return Ok(Geometry::default());
    }
    let mut preserve = false;
    let mut parent = Some(index);
    while let Some(i) = parent {
        if let Some(value) = document.elements[i].attributes.metadata.get("xml:space") {
            preserve = value == "preserve";
            break;
        }
        parent = document.elements[i].parent;
    }
    let raw = String::from_utf8_lossy(&el.text);
    let raw = if preserve {
        raw.replace(['\n', '\r', '\t'], " ")
    } else {
        raw.split([' ', '\t', '\r', '\n'])
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if raw.is_empty() {
        return Ok(Geometry::default());
    }
    let mut position = [0.; 2];
    for (n, key) in ["x", "y"].iter().enumerate() {
        if let Some(raw) = el.attributes.values.get(*key) {
            position[n] = super::svg_length::length(raw, true).map_err(|e| {
                format!(
                    "d2svgimport: {source}: element <{}> has invalid {key}: {e}",
                    el.name
                )
            })?;
        }
    }
    let family = family(&style.font_family);
    if !work.text_fonts.faces.contains_key(family) {
        use d2_rs_text::d2fonts::{FontStyle, registered_font_face, registered_font_face_digest};
        let data = registered_font_face(family, FontStyle::Regular)
            .ok_or("bundled SVG text face is unavailable")?;
        if let Some((size, digest)) = registered_font_face_digest(family, FontStyle::Regular) {
            d2_rs_fontface::register_bundled_face_size(size, digest).map_err(|e| e.to_string())?;
        }
        let registered = d2_rs_fontface::registered_bundled_face(&data, 0)
            .map_err(|e| e.to_string())?
            .ok_or("bundled SVG text face is unavailable")?;
        work.text_fonts.faces.insert(
            family,
            registered.clone_read_only().map_err(|e| e.to_string())?,
        );
    }
    let face = &work.text_fonts.faces[family];
    let ppem = (style.font_size * 64.).round() as i32;
    if ppem == 0 {
        return Ok(Geometry::default());
    }
    let limit = 4096.min(work.budget.max_commands.saturating_sub(work.shaped_runes)) as i64;
    if limit <= 0 {
        return Err(format!(
            "d2svgimport: {source}: text shaping exceeds import budget"
        ));
    }
    let ctx = || match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err(d2_rs_fontface::Error::Canceled(
            "context deadline exceeded".into(),
        )),
        Some(_) => Err(d2_rs_fontface::Error::Canceled("context canceled".into())),
        None => Ok(()),
    };
    let shaped = work
        .text_fonts
        .shaping
        .shape_text_transient(
            &ctx,
            &raw,
            ppem,
            &[ShapeFace {
                id: "SVG text",
                face,
            }],
            ShapeLimits {
                runes: limit,
                faces: 1,
                runs: limit,
                glyphs: limit,
                coverage_checks: limit * 2,
            },
        )
        .map_err(|e| format!("d2svgimport: {source}: SVG text shaping: {e}"))?;
    work.shaped_runes += shaped.runes as usize;
    let mut commands = Vec::new();
    for glyph in shaped.glyphs {
        d2_rs_fontface::Context::err(&ctx).map_err(|e| e.to_string())?;
        if glyph.empty || !glyph.has_ink {
            continue;
        }
        let path = d2_rs_fontface::glyph_path(
            face,
            glyph.id,
            ppem,
            [
                position[0] + glyph.position_x,
                position[1] + glyph.position_y,
            ],
        )?;
        if path.len()
            > work
                .budget
                .max_commands
                .saturating_sub(work.budget.commands + commands.len())
        {
            return Err(format!(
                "d2svgimport: {source}: glyph outlines exceed import path command limit"
            ));
        }
        commands.extend(path.into_iter().map(|c| Command {
            kind: c.kind,
            points: c.points,
            ..Default::default()
        }));
    }
    Ok(Geometry {
        kind: 3,
        commands,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn import(
        body: &str,
        max_commands: usize,
    ) -> Result<super::super::svg_import::Imported, String> {
        let svg = format!("<svg width=\"200\" height=\"40\">{body}</svg>");
        let outcome = super::super::svg_import::import_labeled(
            "icon.svg",
            svg.as_bytes(),
            [
                100_000,
                100,
                10_000,
                10_000,
                100_000,
                max_commands,
                1000,
                100,
                1000,
            ],
            None,
            None,
        )?;
        match outcome {
            super::super::svg_import::Outcome::Imported(i) => Ok(i),
            _ => panic!("image"),
        }
    }
    fn paths(body: &str) -> Vec<Vec<Command>> {
        import(body, 100_000)
            .unwrap()
            .scene
            .nodes
            .into_iter()
            .filter_map(|n| n.primitive.map(|p| p.geometry.commands))
            .collect()
    }
    #[test]
    fn positioned_text_whitespace_font_aliases_and_inheritance() {
        let ordinary = paths(r#"<text x="2" y="30">A B</text>"#);
        assert!(!ordinary[0].is_empty());
        assert_eq!(ordinary, paths("<text x=\"2\" y=\"30\">  A\t B\n </text>"));
        assert_eq!(
            ordinary,
            paths("<g xml:space=\"preserve\"><text x=\"2\" y=\"30\">A\tB</text></g>")
        );
        assert_eq!(
            ordinary,
            paths(
                r#"<g font-family="unknown, 'Source Sans Pro'" font-size="16"><text x="2" y="30">A B</text></g>"#
            )
        );
        assert_ne!(
            ordinary,
            paths(r#"<text x="2" y="30" font-family="monospace">A B</text>"#)
        );
        assert_ne!(
            ordinary,
            paths(r#"<text x="2" y="30" xml:space="preserve"> A  B </text>"#)
        );
    }
    #[test]
    fn reject_flowing_text_and_bound_glyph_outlines() {
        for (body, want) in [
            (
                r#"<text>left<tspan x="1" y="1">right</tspan></text>"#,
                "mixed or flowing",
            ),
            (
                r#"<text><tspan y="1">right</tspan></text>"#,
                "explicitly positioned",
            ),
            (
                r#"<tspan x="1" y="1">right</tspan>"#,
                "explicitly positioned",
            ),
        ] {
            assert!(import(body, 100_000).unwrap_err().contains(want));
        }
        assert!(
            import(r#"<text x="1" y="20">A</text>"#, 1)
                .unwrap_err()
                .contains("glyph outlines")
        );
        assert!(paths(r#"<text font-size="0">A</text>"#).is_empty());
    }
}
