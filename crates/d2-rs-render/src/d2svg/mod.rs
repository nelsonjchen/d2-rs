// SPDX-License-Identifier: MPL-2.0
// Port of D2 d2renderers/d2svg and the helper packages it renders
// with (lib/geo, lib/label, lib/shape, lib/svg, d2themes). The goal is
// byte-identical SVG for the same d2target.Diagram.
//! Faithful d2svg port rendering a [`super::TargetDiagram`].

#![allow(dead_code)]

pub(crate) mod geo;
pub(crate) mod label;
pub(crate) mod shape;
pub(crate) mod svgpath;

pub(crate) mod animate;
pub mod color;
pub(crate) mod element;
pub(crate) mod fonts;
pub mod markup;
mod paint_validation;
pub(crate) mod render;
pub(crate) mod svgtext;
pub(crate) mod target_methods;
pub(crate) mod target_validation;
pub(crate) mod theme_css;
pub(crate) mod xml;

use crate::target::TargetDiagram;

/// Shared raw-target admission for external native renderers (e.g. isometric).
pub(super) fn validate_native_svg(
    diagram: &TargetDiagram,
    options: &crate::RenderOptions,
) -> Result<(), String> {
    paint_validation::links(diagram)?;
    target_validation::validate(diagram)?;
    paint_validation::validate(diagram, &cli_opts(options, false))
}

pub(super) fn validate_document(
    diagram: &TargetDiagram,
    options: &render::RenderOpts,
) -> Result<(), String> {
    paint_validation::links(diagram)?;
    target_validation::validate(diagram)?;
    paint_validation::validate(diagram, options)
}

/// Render an upstream e2e `board.exp.json` exactly as e2etests.run does:
/// pad 0, the case theme, the diagram config applied by d2lib.applyConfigs,
/// multiboard targets wrapped by d2animate at 1000ms with the master hash.
pub fn render_e2e(json: &[u8], theme_id: Option<i64>) -> Result<Vec<u8>, String> {
    let diagram = TargetDiagram::decode_json(json)
        .map_err(|error| error.to_string())?
        .ok_or("null target diagram")?;
    let config = diagram.config.as_ref();
    let mut opts = render::RenderOpts {
        pad: Some(0),
        theme_id,
        ..Default::default()
    };
    if let Some(config) = config {
        opts.theme_id = opts.theme_id.or(config.theme_id);
        opts.dark_theme_id = config.dark_theme_id;
        opts.sketch = config.sketch;
        opts.center = config.center;
        opts.theme_overrides.clone_from(&config.theme_overrides);
        opts.dark_theme_overrides
            .clone_from(&config.dark_theme_overrides);
    }
    opts.theme_id = Some(opts.theme_id.unwrap_or(0));
    opts.sketch = Some(opts.sketch.unwrap_or(false));
    target_validation::validate(&diagram)?;
    let multiboard = diagram.layers.as_ref().is_some_and(|l| !l.is_empty())
        || diagram.scenarios.as_ref().is_some_and(|s| !s.is_empty())
        || diagram.steps.as_ref().is_some_and(|s| !s.is_empty());
    if multiboard {
        opts.master_id = diagram.hash_id(None)?;
    }
    let boards = render::render_multiboard(&diagram, &opts)?;
    if boards.len() == 1 {
        return Ok(boards.into_iter().next().expect("one board"));
    }
    animate::wrap(&diagram, &boards, &opts, 1000)
}
pub(crate) mod appendix;
pub(crate) mod classtable;
pub(crate) mod code;
pub(crate) mod latex;
pub(crate) mod legend;
pub(crate) mod markdown;

/// CLI single-board render: d2svg.Render with d2cli's options (the XML tag
/// and version are always written; d2cli's finalize step strips them).
pub fn render_cli(
    json: &[u8],
    options: &crate::RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, String> {
    let diagram = TargetDiagram::decode_json(json)
        .map_err(|error| error.to_string())?
        .ok_or("null target diagram")?;
    render_cli_target(&diagram, options, sketch)
}

pub(crate) fn render_cli_target(
    diagram: &TargetDiagram,
    options: &crate::RenderOptions,
    sketch: bool,
) -> Result<Vec<u8>, String> {
    let opts = cli_opts(options, sketch);
    render::render(diagram, &opts)
}

/// The same key lookup as the hash's HashThemeOverrides::from_map.
fn theme_overrides_from_map(
    map: &std::collections::BTreeMap<String, String>,
) -> Option<crate::target::TargetThemeOverrides> {
    if map.is_empty() {
        return None;
    }
    let get = |name: &str| {
        map.get(name)
            .cloned()
            .or_else(|| map.get(&name.to_ascii_uppercase()).cloned())
    };
    Some(crate::target::TargetThemeOverrides {
        n1: get("n1"),
        n2: get("n2"),
        n3: get("n3"),
        n4: get("n4"),
        n5: get("n5"),
        n6: get("n6"),
        n7: get("n7"),
        b1: get("b1"),
        b2: get("b2"),
        b3: get("b3"),
        b4: get("b4"),
        b5: get("b5"),
        b6: get("b6"),
        aa2: get("aa2"),
        aa4: get("aa4"),
        aa5: get("aa5"),
        ab4: get("ab4"),
        ab5: get("ab5"),
    })
}

/// d2cli's animated SVG: MasterID, RenderMultiboard, d2animate.Wrap.
pub fn render_cli_animated(
    json: &[u8],
    options: &crate::RenderOptions,
    sketch: bool,
    interval_ms: i64,
) -> Result<Vec<u8>, String> {
    render_cli_animated_with(json, options, sketch, interval_ms, &mut Ok)
}

pub fn render_cli_animated_with(
    json: &[u8],
    options: &crate::RenderOptions,
    sketch: bool,
    interval_ms: i64,
    process: &mut impl FnMut(Vec<u8>) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let diagram = TargetDiagram::decode_json(json)
        .map_err(|error| error.to_string())?
        .ok_or("null target diagram")?;
    let mut opts = cli_opts(options, sketch);
    target_validation::validate(&diagram)?;
    opts.master_id = diagram.hash_id(opts.salt.as_deref())?;
    let boards = render::render_multiboard_with(&diagram, &opts, process)?;
    animate::wrap(&diagram, &boards, &opts, interval_ms)
}

/// d2cli RenderOpts from the hydrated CLI render options.
fn cli_opts(options: &crate::RenderOptions, sketch: bool) -> render::RenderOpts {
    render::RenderOpts {
        pad: Some(options.pad.round() as i64),
        sketch: Some(sketch),
        center: Some(options.center),
        theme_id: Some(options.theme_id.unwrap_or(0)),
        dark_theme_id: options.dark_theme_id,
        theme_overrides: theme_overrides_from_map(&options.theme_overrides),
        dark_theme_overrides: theme_overrides_from_map(&options.dark_theme_overrides),
        scale: options.scale,
        salt: options.salt.clone(),
        hash_input_override: options.d2svg_hash_input.clone(),
        custom_fonts: options.custom_fonts.clone(),
        ..Default::default()
    }
}

/// d2wasm.Render's renderSingleBoard/renderBoards and optional Wrap. The
/// caller has already selected the target and removed unwanted descendants.
pub(crate) fn render_wasm(
    diagram: &TargetDiagram,
    options: &crate::RenderOptions,
    sketch: bool,
    no_xml_tag: bool,
    interval_ms: u64,
    descendants: bool,
) -> Result<Vec<u8>, String> {
    render_wasm_impl(
        diagram,
        options,
        sketch,
        no_xml_tag,
        interval_ms,
        descendants,
        false,
    )
}

/// d2wasm.Render's complete rendering-stage errors: renderSingleBoard and
/// Render each wrap a failed SVG render, while Wrap has its own error prefix.
/// Request decoding, target selection and structural admission belong to the
/// caller. The lower-level TargetDiagram renderer keeps its direct errors.
pub(crate) fn render_wasm_api(
    diagram: &TargetDiagram,
    options: &crate::RenderOptions,
    sketch: bool,
    no_xml_tag: bool,
    interval_ms: u64,
    descendants: bool,
) -> Result<Vec<u8>, String> {
    render_wasm_impl(
        diagram,
        options,
        sketch,
        no_xml_tag,
        interval_ms,
        descendants,
        true,
    )
}

fn render_wasm_impl(
    diagram: &TargetDiagram,
    options: &crate::RenderOptions,
    sketch: bool,
    no_xml_tag: bool,
    interval_ms: u64,
    descendants: bool,
    api_errors: bool,
) -> Result<Vec<u8>, String> {
    let mut opts = cli_opts(options, sketch);
    opts.no_xml_tag = Some(no_xml_tag);
    if !descendants && !api_errors {
        paint_validation::links(diagram)?;
    }
    target_validation::validate(diagram)?;
    if interval_ms > 0 {
        opts.master_id = diagram.hash_id(opts.salt.as_deref())?;
    }
    fn single(
        diagram: &TargetDiagram,
        opts: &render::RenderOpts,
        appendix: bool,
        api_errors: bool,
    ) -> Result<Vec<u8>, String> {
        let rendered = (|| {
            paint_validation::links(diagram)?;
            render::render_validated(diagram, opts)
        })();
        let svg = rendered.map_err(|error| {
            if api_errors {
                format!("render failed: render failed: {error}")
            } else {
                error
            }
        })?;
        if appendix {
            let hash = diagram.hash_id(opts.salt.as_deref())?;
            self::appendix::append(diagram, &hash, &svg)
        } else {
            Ok(svg)
        }
    }
    fn boards(
        diagram: &TargetDiagram,
        opts: &render::RenderOpts,
        appendix: bool,
        api_errors: bool,
    ) -> Result<Vec<Vec<u8>>, String> {
        let mut output = Vec::new();
        for child in diagram.children() {
            output.extend(boards(child, opts, appendix, api_errors)?);
        }
        if !diagram.is_folder_only {
            output.insert(0, single(diagram, opts, appendix, api_errors)?);
        }
        Ok(output)
    }
    let boards = if descendants {
        boards(diagram, &opts, options.force_appendix, api_errors)?
    } else {
        vec![single(diagram, &opts, options.force_appendix, api_errors)?]
    };
    if boards.is_empty() {
        return Ok(Vec::new());
    }
    if interval_ms > 0 {
        let interval =
            i64::try_from(interval_ms).map_err(|_| "animate interval is too large".to_owned())?;
        interval
            .checked_mul(boards.len() as i64)
            .ok_or("animate interval is too large")?;
        animate::wrap(diagram, &boards, &opts, interval).map_err(|error| {
            if api_errors {
                format!("animation failed: {error}")
            } else {
                error
            }
        })
    } else {
        Ok(boards.into_iter().next().expect("nonempty boards"))
    }
}
pub(crate) mod sketch;
pub(crate) mod tooltip;

/// d2cli renderSingle's --force-appendix step (appendix.Append) on a board's
/// rendered SVG.
pub fn append_cli_appendix(
    json: &[u8],
    options: &crate::RenderOptions,
    svg: &[u8],
) -> Result<Vec<u8>, String> {
    let diagram = TargetDiagram::decode_json(json)
        .map_err(|error| error.to_string())?
        .ok_or("null target diagram")?;
    let hash = match &options.d2svg_hash_input {
        Some(bytes) => crate::target_hash_id_from_input(bytes, options.salt.as_deref()),
        None => diagram.hash_id(options.salt.as_deref())?,
    };
    appendix::append(&diagram, &hash, svg)
}
