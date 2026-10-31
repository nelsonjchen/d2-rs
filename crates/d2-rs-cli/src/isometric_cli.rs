// SPDX-License-Identifier: MPL-2.0
//! Master d2cli/isometric{,_image}.go and d2svg/isometric.go.
use super::*;
use d2_rs_isometric::{img, target::Diagram};
use d2_rs_scenebuild as foundation;

#[path = "isometric_paged.rs"]
mod paged;
#[path = "isometric_watch.rs"]
mod watch_preview;

const FORMATS: &str =
    "--isometric exports SVG, PNG, GIF, PDF or PPTX; use a matching filename or --stdout-format";

/// d2cli.compile: isometricExportExtension runs before appendix and size
/// admission when source config enables isometric after the initial flags.
pub(super) fn validate_source_format(format: ExportFormat) -> Result<(), String> {
    if format == ExportFormat::Ascii {
        return Err(FORMATS.into());
    }
    Ok(())
}

pub(super) fn validate(
    format: ExportFormat,
    output: Option<&str>,
    stdout_format: bool,
    appendix: bool,
    scale: f64,
    interval: i64,
) -> Result<(), String> {
    if !stdout_format
        && format == ExportFormat::Svg
        && output.is_some_and(|p| p != "-" && !p.ends_with(".svg"))
    {
        return Err(FORMATS.into());
    }
    if appendix {
        return Err("--isometric cannot be combined with --force-appendix".into());
    }
    if !(0..=600000).contains(&interval) || interval > 0 && format != ExportFormat::Gif {
        return Err("--isometric --animate-interval requires GIF and an interval from 1 to 600000 milliseconds".into());
    }
    dimensions(format, scale)?;
    if format == ExportFormat::Ascii {
        return Err(FORMATS.into());
    }
    Ok(())
}

fn dimensions(format: ExportFormat, mut scale: f64) -> Result<(i64, i64), String> {
    if format == ExportFormat::Svg {
        if scale != -1.0 && (!scale.is_finite() || scale <= 0.0) {
            return Err("isometric SVG scale must be finite and positive".into());
        }
        scale = 1.0;
    }
    let (size, result) = image_dimensions(format, scale);
    result.map(|()| size)
}

/// d2cli.isometricImageOptions, including the returned dimensions on failure.
/// SVG display scaling is applied by the quality wrapper above.
fn image_dimensions(format: ExportFormat, mut scale: f64) -> ((i64, i64), Result<(), String>) {
    let (w, h) = if format == ExportFormat::Gif {
        (1000.0, 625.0)
    } else {
        (1600.0, 1000.0)
    };
    if scale == -1.0 {
        scale = 1.0;
    }
    if !scale.is_finite() || scale <= 0.0 || scale > 4096.0 / w {
        return (
            (w as i64, h as i64),
            Err("--isometric requires a finite positive --scale within image limits".into()),
        );
    }
    let (w, h) = ((w * scale).round() as i64, (h * scale).round() as i64);
    if w < 64 || h < 64 || w > 4096 || h > 4096 || w * h > 12000000 {
        return ((w, h), Err(
            "--isometric dimensions must be at least 64 pixels per side and within image limits"
                .into(),
        ));
    }
    if format == ExportFormat::Gif && w * h * 104 > 100_000_000 {
        return (
            (w, h),
            Err("--isometric GIF aggregate pixels exceed limit; reduce --scale".into()),
        );
    }
    ((w, h), Ok(()))
}

fn resources(
    input: &str,
) -> Result<(foundation::AssetOptions, foundation::FontFallbackOptions), String> {
    let base = if input == "-" {
        Path::new(".")
    } else {
        Path::new(input).parent().unwrap_or(Path::new("."))
    };
    let resolver = foundation::RasterImageResolver::new(
        base,
        foundation::ImageAssetLimits {
            max_fetched_bytes: 32 << 20,
            max_encoded_bytes: 32 << 20,
            max_decompressed_bytes: 32 << 20,
            max_svg_bytes: 64 << 10,
            max_decoded_width: 32768,
            max_decoded_height: 32768,
            max_decoded_pixels: 64 << 20,
            max_assets: 4032,
            max_cumulative_encoded_bytes: 448 << 20,
            max_cumulative_decoded_bytes: 512 << 20,
        },
        Arc::new(foundation::UnrestrictedImageFiles::new()),
    )
    .map_err(|e| e.to_string())?
    .with_http_client(Arc::new(foundation::NativeImageHttpClient {
        allow_private_networks: http_asset::allow_private_network(),
    }));
    let mut assets = img::surface_document::surface_assets();
    assets.resolver_impl = Some(Arc::new(resolver));
    assets.svg_import_limits = foundation::SvgImportLimits {
        max_bytes: 65536,
        max_depth: 32,
        max_elements: 256,
        max_attributes: 512,
        max_attribute_bytes: 65536,
        max_path_commands: 4096,
        max_transform_functions: 128,
        max_use_depth: 8,
        max_resources: 64,
    };
    assets.svg_import_budget = foundation::SvgImportBudget {
        max_source_bytes: 1 << 20,
        max_elements: 4096,
        max_attributes: 8192,
        max_attribute_bytes: 1 << 20,
        max_path_commands: 65536,
        max_transform_functions: 2048,
        max_declared_resources: 1024,
        max_expanded_use_instances: 1024,
    };
    let fonts = foundation::FontFallbackOptions {
        resolver_impl: Some(Arc::new(foundation::BundledFallbackResolver::new(
            None,
            foundation::BundledFallbackLimits {
                max_requested_runes: 100000,
                max_bundled_bytes: 8 << 20,
                max_resolved_bytes: 136 << 20,
            },
        )?)),
        max_assets: 64,
        max_bytes: 64 << 20,
        max_runes_per_text: 100000,
        max_total_runes: 10000000,
        max_coverage_checks: 50000000,
        max_font_faces_per_text: 64,
        max_shaping_runs: 100000,
        max_shaped_glyphs: 10000000,
        ..Default::default()
    };
    Ok((assets, fonts))
}

/// d2cli/isometric_image.go:247–299. Paths use full source board IDs without
/// the leading `root.`; `selected` supplies the source ID for local `root`.
fn relink_svg_board(
    value: &mut serde_json::Value,
    current_board: &str,
    selected: &str,
    paths: &BTreeMap<String, PathBuf>,
) {
    for field in ["shapes", "connections"] {
        if let Some(items) = value[field].as_array_mut() {
            for item in items {
                if let Some(link) = item["link"].as_str() {
                    let wanted = if link == "root" {
                        &selected
                    } else {
                        link.strip_prefix("root.").unwrap_or("\0")
                    };
                    if let Some(dest) = paths.get(wanted) {
                        let current = paths[current_board].parent().unwrap_or(Path::new(""));
                        let parent = current.components().collect::<Vec<_>>();
                        let to = dest.components().collect::<Vec<_>>();
                        let common = parent.iter().zip(&to).take_while(|(a, b)| a == b).count();
                        let mut rel = vec!["..".to_owned(); parent.len() - common];
                        rel.extend(
                            to[common..]
                                .iter()
                                .map(|p| url_path_escape(&p.as_os_str().to_string_lossy())),
                        );
                        let mut rel = rel.join("/");
                        if rel == "root.svg" || rel.starts_with("root.") {
                            rel = format!("./{rel}");
                        }
                        item["link"] = rel.into();
                    }
                }
            }
        }
    }
}

fn source_diagram_json(
    frame: &AnimationTarget,
    root: bool,
    copy_slices: bool,
) -> Result<serde_json::Value, String> {
    let mut value: serde_json::Value = serde_json::from_slice(
        frame
            .target_json
            .as_deref()
            .ok_or("isometric export requires target JSON")?,
    )
    .map_err(|e| e.to_string())?;
    // Export allocates empty slices. relinkIsometricSVGBoard copies only
    // this board with append(nil, slice...), making empty slices nil; child
    // pointers keep the original allocated empties for recursive HashID.
    if copy_slices {
        for field in ["shapes", "connections"] {
            if value[field].as_array().is_some_and(Vec::is_empty) {
                value[field] = serde_json::Value::Null;
            }
        }
    }
    if root {
        value["config"]["isometric"] = true.into();
    }
    Ok(value)
}

fn wrap(
    svg: &str,
    hash: &str,
    pad: i64,
    scale: Option<f64>,
    center: bool,
    no_xml: bool,
    omit: bool,
) -> Result<Vec<u8>, String> {
    d2_rs_isometric::document::wrap_svg(
        svg,
        hash,
        pad,
        scale,
        center,
        no_xml,
        omit,
        command_line::VERSION,
    )
}

fn check_context() -> Result<(), String> {
    if let Some(error) = watch::active_cancellation_token().and_then(|token| token.error()) {
        return Err(error.to_string());
    }
    Ok(())
}

/// atomicIsometricImage: never truncate a previous file, preserve its mode,
/// and remove the private sibling on every failure.
fn atomic_image(path: &Path, data: &[u8]) -> Result<(), String> {
    use ring::rand::SecureRandom as _;
    check_context()?;
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut nonce = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| "failed to generate isometric output name")?;
    let temporary = parent.join(format!(
        ".d2-isometric-{}",
        nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let mut file = open.open(&temporary).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(data).map_err(|e| e.to_string())?;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.is_file() {
                file.set_permissions(meta.permissions())
                    .map_err(|e| e.to_string())?;
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        check_context()?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    input: &[u8],
    input_path: &str,
    output: Option<&str>,
    format: ExportFormat,
    interval_ms: i64,
    target: Option<&str>,
    layout: &str,
    seeds: Option<&str>,
    pad: Option<f64>,
    scale: Option<f64>,
    salt: Option<&str>,
    theme: Option<i64>,
    dark: Option<i64>,
    center: Option<bool>,
    adapter_args: &[String],
    no_xml: bool,
    omit: bool,
    preview: bool,
    started: std::time::Instant,
) -> Result<Vec<u8>, String> {
    if format == ExportFormat::Svg && dark.is_some() {
        return Err("isometric SVG does not yet support --dark-theme; render a separate SVG with --theme instead".into());
    }
    if preview {
        // main.go:578–600 prepares the preview before publishing any file.
        // Watch selects a subtree (including its children), whereas its SVG
        // preview is only the first drawable board with full-tree navigation.
        let data = watch_preview::render(
            input,
            input_path,
            target.unwrap_or(""),
            layout,
            seeds,
            pad,
            scale,
            salt,
            theme,
            center,
            adapter_args,
            no_xml,
            omit,
        )?;
        let selected = target
            .unwrap_or("")
            .trim_end_matches('*')
            .trim_end_matches('.');
        let subtree = if selected.is_empty() {
            "*".to_owned()
        } else {
            format!("{selected}.*")
        };
        run(
            input,
            input_path,
            output,
            format,
            interval_ms,
            Some(&subtree),
            layout,
            seeds,
            pad,
            scale,
            salt,
            theme,
            dark,
            center,
            adapter_args,
            no_xml,
            omit,
            false,
            started,
        )?;
        return Ok(data);
    }
    if !matches!(
        format,
        ExportFormat::Svg
            | ExportFormat::Png
            | ExportFormat::Gif
            | ExportFormat::Pdf
            | ExportFormat::Pptx
    ) {
        return Err(format!(
            "native isometric {} output is not implemented",
            format.extension()
        ));
    }
    let include_children = target.is_none_or(|t| t == "*" || t.ends_with(".*"));
    let selected = target
        .unwrap_or("")
        .trim_end_matches('*')
        .trim_end_matches('.');
    let (plan, selected) = source_board_plan_for_target(input, input_path, selected)?;
    let descendants = plan
        .all_targets
        .iter()
        .filter(|t| selected.is_empty() || t.starts_with(&format!("{selected}.")))
        .cloned()
        .collect::<Vec<_>>();
    let has_children = include_children && !descendants.is_empty();
    let mut targets = Vec::new();
    if (selected.is_empty() && plan.root_renderable) || plan.renderable_targets.contains(&selected)
    {
        targets.push(selected.clone());
    }
    if has_children {
        targets.extend(
            plan.targets
                .iter()
                .filter(|t| selected.is_empty() || t.starts_with(&format!("{selected}.")))
                .cloned(),
        );
    }
    let is_paged = matches!(format, ExportFormat::Pdf | ExportFormat::Pptx);
    if !is_paged && targets.len() > 128 {
        return Err("GIF board count exceeds the 1800-frame limit".into());
    }
    if targets.is_empty() {
        return Err(if is_paged {
            "isometric paged export requires at least one renderable board"
        } else {
            "isometric export has no drawable boards"
        }
        .into());
    }
    let output = output
        .map(PathBuf::from)
        .or_else(|| default_output_path(input_path, format))
        .unwrap_or_else(|| PathBuf::from("-"));
    if is_paged {
        return paged::run(
            input,
            input_path,
            &output,
            format,
            &selected,
            &targets,
            &plan.all_targets,
            &plan.renderable_targets,
            layout,
            seeds,
            pad,
            scale,
            theme,
            center,
            adapter_args,
            started,
        );
    }
    let mut base = source_path_args(input_path);
    base.push(format!("--engine={layout}"));
    for (name, value) in [
        ("seeds", seeds.map(str::to_owned)),
        ("pad", pad.map(|v| v.to_string())),
        ("scale", scale.map(|v| v.to_string())),
        ("theme", theme.map(|v| v.to_string())),
        ("center", center.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            base.push(format!("--{name}={value}"));
        }
    }
    base.extend(adapter_args.iter().cloned());
    let (assets, fonts) = resources(input_path)?;
    let (width, height) = dimensions(format, scale.unwrap_or(-1.0))?;
    // isometric_image.go:120–145: GIF traversal always writes one file.
    // A folder with one drawable child still uses the timeline hold interval.
    if format == ExportFormat::Gif {
        let mut boards = Vec::with_capacity(targets.len());
        let mut render_options = None;
        for t in &targets {
            check_context()?;
            let mut args = base.clone();
            args.push(format!("--target={t}"));
            let frame = render_source_target_animation(input, &args)?;
            render_options.get_or_insert(frame.render_options.theme_id);
            boards.push(Some(
                serde_json::from_value::<Diagram>(source_diagram_json(
                    &frame,
                    t.is_empty(),
                    false,
                )?)
                .map_err(|e| e.to_string())?,
            ));
        }
        let options = img::render::Options {
            format: "gif".into(),
            width,
            height,
            fit_content: true,
            render: d2_rs_isometric::scene::RenderOpts {
                theme_id: render_options.unwrap(),
                theme_overrides: boards[0]
                    .as_ref()
                    .and_then(|d| d.config.as_ref())
                    .and_then(|c| c.theme_overrides.clone()),
            },
            fonts: Some(fonts),
            assets: Some(assets),
            ..Default::default()
        };
        let token = watch::active_cancellation_token();
        let data = if has_children || boards.len() > 1 || interval_ms > 0 {
            img::timeline::render_timeline(
                token.as_ref(),
                &boards,
                Some(&options),
                if interval_ms == 0 { 1000 } else { interval_ms } * 1_000_000,
            )
        } else {
            img::render::render_with_context(token.as_ref(), boards[0].as_ref(), Some(&options))
        }
        .map_err(|e| e.to_string())?;
        if output.as_os_str() != "-" {
            atomic_image(&output, &data)?;
        }
        log_compile_success(Path::new(input_path), &output, started.elapsed());
        return Ok(if output.as_os_str() == "-" {
            data
        } else {
            Vec::new()
        });
    }
    if output.as_os_str() == "-" && targets.len() != 1 {
        return Err(format!(
            "isometric {} stdout requires one board; use --target or an output filename for multiple boards",
            format.extension().to_uppercase()
        ));
    }
    let out_dir = output.with_extension("");
    let local_targets = descendants
        .iter()
        .map(|t| {
            if selected.is_empty() {
                t.clone()
            } else {
                t[selected.len() + 1..].to_owned()
            }
        })
        .collect::<Vec<_>>();
    // resolveLinks includes folder-only boards too: links to a folder retain
    // its index.svg destination even though that folder emits no blank image.
    let paths = std::iter::once(&selected)
        .chain(descendants.iter())
        .filter(|t| has_children || targets.contains(t))
        .map(|t| {
            let local = if t == &selected {
                ""
            } else if selected.is_empty() {
                t
            } else {
                &t[selected.len() + 1..]
            };
            (
                t.clone(),
                if has_children {
                    if selected.is_empty() {
                        out_dir.join(board_output_path(Some(local), format, &local_targets))
                    } else {
                        out_dir.join(board_subtree_output_path(
                            t,
                            &selected,
                            format,
                            &plan.all_targets,
                        ))
                    }
                } else {
                    output.clone()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    board_output::validate_board_output_paths(
        &paths
            .iter()
            .map(|(k, v)| (format!("root.{k}"), v.clone()))
            .collect::<Vec<_>>(),
    )?;
    let workspace = if has_children && output.as_os_str() != "-" {
        Some(board_output::BoardOutputWorkspace::new(&out_dir)?)
    } else {
        None
    };
    let mut original_children = BTreeMap::new();
    if has_children && format == ExportFormat::Svg {
        for child in &descendants {
            let mut args = base.clone();
            args.push(format!("--target={child}"));
            let frame = render_source_target_animation(input, &args)?;
            original_children.insert(
                child.clone(),
                serde_json::to_vec(&source_diagram_json(&frame, false, false)?)
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    let mut last = Vec::new();
    for t in targets {
        let mut args = base.clone();
        args.push(format!("--target={t}"));
        let frame = render_source_target_animation(input, &args)?;
        let mut value = source_diagram_json(&frame, t.is_empty(), format == ExportFormat::Svg)?;
        // Isometric SVG hashes each local board after rebasing links. Descendant targets remain metadata for the recursive hash.
        if format == ExportFormat::Svg {
            relink_svg_board(&mut value, &t, &selected, &paths);
        }
        let json = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        let diagram: Diagram = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
        let options = img::render::Options {
            render: d2_rs_isometric::scene::RenderOpts {
                theme_id: frame.render_options.theme_id,
                theme_overrides: diagram
                    .config
                    .as_ref()
                    .and_then(|c| c.theme_overrides.clone()),
            },
            width,
            height,
            fit_content: true,
            fonts: Some(fonts.clone()),
            assets: Some(assets.clone()),
            ..Default::default()
        };
        // isometric_image.go:229–235 wraps rendering and writing errors only
        // for file-based multiboard exports, using the selected tree's key.
        let data = (|| -> Result<Vec<u8>, String> {
            let data = if format == ExportFormat::Svg {
                let hash_json = if has_children {
                    let prefix = if t.is_empty() {
                        String::new()
                    } else {
                        format!("{t}.")
                    };
                    let mut frames = vec![(t.clone(), json.clone(), true)];
                    frames.extend(
                        descendants
                            .iter()
                            .filter(|child| child.as_str() != t && child.starts_with(&prefix))
                            .map(|child| {
                                (
                                    child.clone(),
                                    original_children[child].clone(),
                                    plan.renderable_targets.contains(child),
                                )
                            }),
                    );
                    d2_rs_render::target_diagram_tree_json(&t, &frames)?
                } else {
                    json.clone()
                };
                let hash_data = d2_rs_render::target::TargetDiagram::decode_json(&hash_json)
                    .map_err(|e| e.to_string())?
                    .ok_or("null diagram")?
                    .hash_input()
                    .map_err(|e| e.to_string())?;
                let hash = d2_rs_render::target_hash_id_from_input(&hash_data, salt);
                wrap(
                    &img::render::render_svg(&diagram, &options)?,
                    &hash,
                    pad.unwrap_or(100.0) as i64,
                    scale,
                    center.unwrap_or(false),
                    no_xml,
                    omit,
                )?
            } else {
                img::raster::render_png(&diagram, &options)?
            };
            let final_path = &paths[&t];
            if output.as_os_str() != "-" {
                let path = if let Some(workspace) = &workspace {
                    workspace.stage_root().join(
                        final_path
                            .strip_prefix(&out_dir)
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    final_path.clone()
                };
                atomic_image(&path, &data)?;
            }
            Ok(data)
        })()
        .map_err(|error| {
            if workspace.is_some() {
                let local = t
                    .strip_prefix(&selected)
                    .unwrap_or(&t)
                    .trim_start_matches('.');
                let key = if local.is_empty() {
                    "root".to_owned()
                } else {
                    format!("root.{local}")
                };
                format!("isometric board {key}: {error}")
            } else {
                error
            }
        })?;
        last = data;
    }
    check_context()?;
    if let Some(workspace) = workspace {
        workspace.publish()?;
    }
    log_compile_success(Path::new(input_path), &output, started.elapsed());
    if output.as_os_str() == "-" || preview {
        Ok(last)
    } else {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_link_target_hash_roundtrip() {
        let input=b"a: {link: layers.detail}\nlayers: {detail: {b: {link: root}}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}\n";
        for target in ["", "layers.detail"] {
            let frame = render_source_target_animation(
                input,
                &["--engine=dagre".into(), format!("--target={target}")],
            )
            .unwrap();
            let decoded = d2_rs_render::target::TargetDiagram::decode_json(
                frame.target_json.as_deref().unwrap(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                String::from_utf8(frame.hash_input).unwrap(),
                String::from_utf8(decoded.hash_input().unwrap()).unwrap(),
                "{target}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_isometric_root_link_hash() {
        let input=b"a: {link: layers.detail}\nlayers: {detail: {b: {link: root}}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}\n";
        let frame = render_source_target_animation(
            input,
            &[
                "--engine=dagre".into(),
                "--target=".into(),
                "--scale=.2".into(),
            ],
        )
        .unwrap();
        let value = source_diagram_json(&frame, true, true).unwrap();
        let decoded =
            d2_rs_render::target::TargetDiagram::decode_json(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(
            String::from_utf8(decoded.hash_input().unwrap()).unwrap(),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/isometric-root-hash-master.bytes")
        );
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_isometric_descendant_slice_hash() {
        let input=b"a: {link: layers.detail}\nlayers: {detail: {b: {link: root}}}\nscenarios: {alternate: {c}}\nsteps: {next: {d}}\n";
        let mut frames = Vec::new();
        for target in ["", "layers.detail", "scenarios.alternate", "steps.next"] {
            let frame = render_source_target_animation(
                input,
                &["--engine=dagre".into(), format!("--target={target}")],
            )
            .unwrap();
            let mut value =
                source_diagram_json(&frame, target.is_empty(), target.is_empty()).unwrap();
            if target.is_empty() {
                value["shapes"][0]["link"] = "layers/detail.svg".into();
            }
            frames.push((target.to_owned(), serde_json::to_vec(&value).unwrap(), true));
        }
        let tree = d2_rs_render::target_diagram_tree_json("", &frames).unwrap();
        let target = d2_rs_render::target::TargetDiagram::decode_json(&tree)
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(target.hash_input().unwrap()).unwrap(),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/isometric-tree-hash-master.bytes")
        );
    }
    #[test]
    fn source_image_quality_contract() {
        // d2cli/isometric_image_test.go TestIsometricImageOptionsAndFailures,
        // plus isometric_test.go TestIsometricSVGScaleIsIndependentOfRasterLimits.
        for scale in [0.0, -2.0, 0.001, 100.0, f64::NAN, f64::INFINITY] {
            assert!(dimensions(ExportFormat::Png, scale).is_err(), "{scale}");
        }
        assert_eq!(dimensions(ExportFormat::Png, 0.5).unwrap(), (800, 500));
        assert_eq!(dimensions(ExportFormat::Gif, 0.5).unwrap(), (500, 313));
        for scale in [0.001, 10.0] {
            assert_eq!(dimensions(ExportFormat::Svg, scale).unwrap(), (1600, 1000));
        }
    }
    #[test]
    fn failed_image_keeps_existing_output() {
        let directory =
            std::env::temp_dir().join(format!("d2-rs-isometric-error-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("output.svg");
        std::fs::write(&output, b"previous output").unwrap();
        let args = vec![
            "--isometric".into(),
            "--scale=0".into(),
            "-".into(),
            output.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"previous output");
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn multiboard_render_errors_name_the_local_board() {
        // isometric_image.go:229–235: the prefix uses the selected subtree's
        // local key, including for the root; single-board errors stay direct.
        let directory = std::env::temp_dir().join(format!(
            "d2-rs-isometric-board-error-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("input.d2");
        std::fs::write(
            &input,
            b"a\nlayers: {parent: {b; layers: {broken: {c.icon: ./missing.png}}}}",
        )
        .unwrap();
        for format in ["svg", "png"] {
            for (target, prefix) in [
                ("*", "isometric board root.layers.parent.layers.broken: "),
                ("layers.parent.*", "isometric board root.layers.broken: "),
                ("layers.parent.layers.broken", ""),
            ] {
                let args = vec![
                    input.to_string_lossy().into_owned(),
                    "--isometric".into(),
                    "--scale=.1".into(),
                    format!("--target={target}"),
                    directory
                        .join(format!("views.{format}"))
                        .to_string_lossy()
                        .into_owned(),
                ];
                let error = run_document(&args).unwrap_err();
                assert!(error.contains(prefix), "{format}, {target}: {error}");
                assert_eq!(
                    error.contains("isometric board "),
                    target.ends_with('*'),
                    "{error}"
                );
                assert!(error.contains("./missing.png"), "{error}");
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod publication_api_tests;

#[cfg(test)]
mod image_api_tests;

#[cfg(test)]
#[path = "isometric_svg_api_tests.rs"]
mod svg_api_tests;
