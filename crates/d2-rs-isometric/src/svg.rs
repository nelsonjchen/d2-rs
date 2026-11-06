// SPDX-License-Identifier: MPL-2.0
//! d2svg Render/RenderMultiboard mode dispatch and isometric.go:21–96.
//!
//! This facade lives above the flat renderer in the dependency graph: the mesh
//! builder itself consumes d2-rs-render geometry and cannot be its dependency.
use d2_rs_core::CancellationToken;
use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};

use crate::{document, img::render as image};

/// d2svg.Render, including explicit false and isometric document options.
pub fn render(
    diagram: Option<&TargetDiagram>,
    options: &SvgRenderOptions,
) -> Result<Vec<u8>, String> {
    let diagram = diagram.ok_or("render target is nil")?;
    if options.isometric == Some(true) {
        render_isometric(None, Some(diagram), options, None)
    } else {
        diagram.render_flat_svg(options)
    }
}

/// d2svg.RenderIsometric. Native dimensions remain unscaled; document scale
/// affects only the outer viewport. Assets are loaded only through a resolver.
pub fn render_isometric(
    context: Option<&CancellationToken>,
    diagram: Option<&TargetDiagram>,
    options: &SvgRenderOptions,
    native: Option<&image::Options>,
) -> Result<Vec<u8>, String> {
    let diagram = diagram.ok_or("isometric rendering requires a diagram")?;
    diagram.validate_svg_document(options)?;
    render_isometric_validated(context, diagram, options, native)
}

fn render_isometric_validated(
    context: Option<&CancellationToken>,
    diagram: &TargetDiagram,
    options: &SvgRenderOptions,
    native: Option<&image::Options>,
) -> Result<Vec<u8>, String> {
    if options.sketch == Some(true) {
        return Err("sketch cannot be combined with isometric rendering".into());
    }
    if options.dark_theme_id.is_some() || options.dark_theme_overrides.is_some() {
        return Err("isometric rendering does not yet support responsive dark themes; select a single theme instead".into());
    }
    if !options.master_id.is_empty() {
        return Err("isometric rendering does not yet support multi-board SVG animation".into());
    }
    if options
        .scale
        .is_some_and(|scale| !scale.is_finite() || scale <= 0.0)
    {
        return Err("isometric SVG scale must be finite and positive".into());
    }
    let mut native = native.cloned().unwrap_or_else(|| image::Options {
        fit_content: true,
        ..Default::default()
    });
    native.format = "svg".into();
    native.render.theme_id = options.theme_id;
    native.render.theme_overrides = options
        .theme_overrides
        .as_ref()
        .map(|value| serde_json::from_value(serde_json::to_value(value)?))
        .transpose()
        .map_err(|error| error.to_string())?;
    let target =
        serde_json::from_value(serde_json::to_value(diagram).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let svg = image::render_svg_with_context(context, Some(&target), Some(&native))
        .map_err(|error| error.to_string())?;
    // Use the supplied target tree, even when rendering just its root board.
    let hash = d2_rs_render::target_hash_id_from_input(
        &diagram.hash_input().map_err(|error| error.to_string())?,
        options.salt.as_deref(),
    );
    document::wrap_svg(
        &svg,
        &hash,
        options.pad.unwrap_or(100),
        options.scale,
        options.center == Some(true),
        options.no_xml_tag == Some(true),
        options.omit_version == Some(true),
        "v0.9.0",
    )
}

/// d2svg.RenderMultiboard: complete structural validation before traversal,
/// children evaluated first, root-first output, folder-only boards omitted.
pub fn render_multiboard(
    diagram: Option<&TargetDiagram>,
    options: &SvgRenderOptions,
) -> Result<Vec<Vec<u8>>, String> {
    let diagram = diagram.ok_or("render target is nil")?;
    diagram.validate_render_target()?;
    fn boards(d: &TargetDiagram, o: &SvgRenderOptions) -> Result<Vec<Vec<u8>>, String> {
        let mut output = Vec::new();
        for child in d.children() {
            output.extend(boards(child, o)?);
        }
        if !d.is_folder_only() {
            output.insert(0, render(Some(d), o)?);
        }
        Ok(output)
    }
    boards(diagram, options)
}
