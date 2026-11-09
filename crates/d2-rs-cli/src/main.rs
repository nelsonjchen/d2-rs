// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Native d2-rs command boundary.

// The TALA and Dagre ports allocate many small, short-lived values where Go
// amortizes through its GC; mimalloc cuts 3-16% of wall time on perf-bench
// with byte-identical output.
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use base64::Engine as _;
use d2_rs_core::{
    Graph, NodeId, Size,
    ast::{Child, Shared},
    theme,
};
use d2_rs_export::ExportFormat;
use d2_rs_language::{ParseOptions, compiler, format, parse};
use d2_rs_layout::EngineKind;
use d2_rs_render::RenderOptions;
#[cfg(test)]
use d2_rs_text::markdown::html::unescape_html;
use d2_rs_text::{
    markdown::plain_text::plain_text,
    measure::{
        TrueTypeFont, bundled_fuzzy_bubbles_bold, bundled_fuzzy_bubbles_regular,
        bundled_source_code_pro_bold, bundled_source_code_pro_italic,
        bundled_source_code_pro_regular, bundled_source_sans_pro_bold,
        bundled_source_sans_pro_italic, bundled_source_sans_pro_regular,
        bundled_source_sans_pro_semibold,
    },
};
use flate2::{Compression, write::DeflateEncoder};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

#[cfg(all(test, unix))]
mod cli_admission_api_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod engine_option_tests;
#[cfg(test)]
mod fuzz_layout_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod paged_link_tests;
#[cfg(test)]
mod structured_raster_tests;
#[cfg(all(test, feature = "oracle-fixtures"))]
mod subtree_output_tests;

mod board_output;
mod cli_log;
mod command_line;
mod deadline;
mod http_asset;
mod imgbundler_cli;
mod layout_features;
mod layout_resolver;
mod pptx_properties;
use pptx_properties::pptx_document_properties;
mod isometric_cli;
mod raster_asset;
mod watch;

const DEFAULT_SEEDS: &[i64] = &[1, 2, 3];
type ImportOpener =
    Box<dyn FnMut(&str) -> Result<Box<dyn d2_rs_language::stream::ByteReader>, String>>;

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| format!("could not read stdin: {error}"))?;
    Ok(input)
}

fn human_path(path: &Path) -> String {
    if path.as_os_str() == "-" {
        return "-".into();
    }
    let Ok(current_dir) = std::env::current_dir() else {
        return path.display().to_string();
    };
    human_path_from(path, &current_dir, std::env::var_os("HOME").as_deref())
}

fn human_path_from(path: &Path, current_dir: &Path, home: Option<&std::ffi::OsStr>) -> String {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        current_dir.join(path)
    };
    // filepath.Abs/Rel clean lexically. Do not canonicalize: paths may not
    // exist, and symlink traversal must not change diagnostic spelling.
    let mut cleaned = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                cleaned.pop();
            }
            _ => cleaned.push(component.as_os_str()),
        }
    }
    let abbreviate = |path: &Path| {
        if let Some(home) = home {
            let path_text = path.to_string_lossy();
            let home_text = home.to_string_lossy();
            if let Some(suffix) = path_text.strip_prefix(home_text.as_ref()) {
                return Path::new("~").join(suffix.trim_start_matches(std::path::MAIN_SEPARATOR));
            }
        }
        path.to_owned()
    };
    let cleaned = abbreviate(&cleaned);
    let current_dir = abbreviate(current_dir);
    #[cfg(windows)]
    let current_dir = clean_asset_path(&current_dir);
    let base = current_dir.components().collect::<Vec<_>>();
    let target = cleaned.components().collect::<Vec<_>>();
    let same_component = |left: &std::path::Component<'_>, right: &std::path::Component<'_>| {
        // filepath/path_windows.go sameWord uses strings.EqualFold, including
        // Unicode simple folding, for both volumes and directory components.
        #[cfg(windows)]
        return d2_rs_core::ir::equal_fold(
            left.as_os_str().to_string_lossy().as_bytes(),
            right.as_os_str().to_string_lossy().as_bytes(),
        );
        #[cfg(not(windows))]
        (left == right)
    };
    if !base
        .first()
        .zip(target.first())
        .is_some_and(|(left, right)| same_component(left, right))
    {
        // Rel fails across volumes. xmain returns the original absolute
        // spelling, preserving forward slashes and dot components there.
        #[cfg(windows)]
        return abbreviate(&absolute).display().to_string();
        #[cfg(not(windows))]
        return cleaned.display().to_string();
    }
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| same_component(left, right))
        .count();
    let mut relative = PathBuf::new();
    for _ in &base[common..] {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative.display().to_string()
}

fn document_compile_error(path: &Path, error: String) -> String {
    format!("err: failed to compile {}: {error}", human_path(path))
}

fn filesystem_import_opener() -> ImportOpener {
    Box::new(|path| {
        let file = watch::open_tracked_file(
            &d2_rs_scenebuild::ImageLocalFiles::Unrestricted,
            Path::new(path),
        )
        .map_err(|error| error.to_string())?;
        Ok(Box::new(file))
    })
}

fn parse_source_options(args: &[String]) -> Result<(String, bool), String> {
    let mut path = "-".to_owned();
    let mut utf16 = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--utf16" {
            utf16 = true;
        } else if let Some(value) = argument.strip_prefix("--path=") {
            // An empty path is d2lib.Parse without a source path; ranges then
            // print without a path prefix.
            path = value.to_owned();
        } else if argument == "--path" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err("--path requires a value".into());
            };
            if value.is_empty() {
                return Err("--path must not be empty".into());
            }
            path = value.clone();
        } else {
            return Err(format!("unrecognized source option {argument}"));
        }
        index += 1;
    }
    Ok((path, utf16))
}

fn extract_font_option(args: &[String]) -> Result<(Vec<String>, FontPaths), String> {
    let mut source_args = Vec::new();
    let mut font_paths = FontPaths::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some(value) = argument.strip_prefix("--font=") {
            if value.is_empty() {
                return Err("--font must not be empty".into());
            }
            font_paths = FontPaths::all(value.to_owned());
        } else if argument == "--font" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err("--font requires a value".into());
            };
            if value.is_empty() {
                return Err("--font must not be empty".into());
            }
            font_paths = FontPaths::all(value.clone());
        } else if let Some((slot, value)) = font_argument(argument) {
            if value.is_empty() {
                return Err(format!("{slot} must not be empty"));
            }
            set_font_path(&mut font_paths, slot, value);
        } else if matches!(
            argument.as_str(),
            "--font-regular"
                | "--font-italic"
                | "--font-bold"
                | "--font-semibold"
                | "--font-mono"
                | "--font-mono-bold"
                | "--font-mono-italic"
                | "--font-mono-semibold"
        ) {
            let flag = argument.clone();
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.is_empty() {
                return Err(format!("{flag} must not be empty"));
            }
            set_font_path(&mut font_paths, flag.trim_start_matches('-'), value);
        } else {
            source_args.push(argument.clone());
        }
        index += 1;
    }
    Ok((source_args, font_paths))
}

fn extract_render_options(
    args: &[String],
) -> Result<(Vec<String>, RenderOptions, bool, bool), String> {
    let mut source_args = Vec::new();
    let mut options = RenderOptions::default();
    options.custom_fonts = load_render_fonts(args)?;
    let mut center_explicit = false;
    let mut sketch_explicit = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some(value) = argument.strip_prefix("--ascii-mode=") {
            options.ascii_mode = Some(value.to_owned());
        } else if argument == "--ascii-mode" {
            index += 1;
            options.ascii_mode = Some(
                args.get(index)
                    .ok_or("--ascii-mode requires a value")?
                    .clone(),
            );
        } else if argument == "--sketch" {
            sketch_explicit = true;
        } else if argument == "--force-appendix" {
            options.force_appendix = true;
        } else if let Some(value) = argument.strip_prefix("--force-appendix=") {
            options.force_appendix = parse_bool_flag(value, "force-appendix")?;
        } else if argument == "--bundle" || argument == "-b" {
            // `bundle` is a document-level switch. It is consumed here so it
            // does not leak into source parser options; the caller consults
            // `bundle_requested` before loading document assets.
        } else if argument == "--no-bundle" {
            // Compatibility spelling used by the native CLI's boolean flag.
        } else if let Some(value) = argument.strip_prefix("--bundle=") {
            parse_bool_flag(value, "bundle")?;
        } else if let Some(value) = argument.strip_prefix("--pad=") {
            let pad = value
                .parse::<f64>()
                .map_err(|error| format!("invalid pad value {value:?}: {error}"))?;
            // d2svg.validatePadding reports pads that collapse the canvas.
            if !pad.is_finite() {
                return Err("pad must be a finite number".into());
            }
            options.pad = pad;
        } else if argument == "--pad" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--pad requires a value".to_owned())?;
            let pad = value
                .parse::<f64>()
                .map_err(|error| format!("invalid pad value {value:?}: {error}"))?;
            // d2svg.validatePadding reports pads that collapse the canvas.
            if !pad.is_finite() {
                return Err("pad must be a finite number".into());
            }
            options.pad = pad;
        } else if let Some(value) = argument.strip_prefix("--scale=") {
            let scale = value
                .parse::<f64>()
                .map_err(|error| format!("invalid scale value {value:?}: {error}"))?;
            if !scale.is_finite() {
                return Err("scale must be finite".into());
            }
            options.scale = (scale > 0.0).then_some(scale);
        } else if argument == "--scale" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--scale requires a value".to_owned())?;
            let scale = value
                .parse::<f64>()
                .map_err(|error| format!("invalid scale value {value:?}: {error}"))?;
            if !scale.is_finite() {
                return Err("scale must be finite".into());
            }
            options.scale = (scale > 0.0).then_some(scale);
        } else if let Some(value) = argument.strip_prefix("--salt=") {
            if value.is_empty() {
                return Err("salt must not be empty".into());
            }
            options.salt = Some(value.to_owned());
        } else if argument == "--salt" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--salt requires a value".to_owned())?;
            if value.is_empty() {
                return Err("salt must not be empty".into());
            }
            options.salt = Some(value.clone());
        } else if let Some(value) = argument
            .strip_prefix("--theme=")
            .or_else(|| argument.strip_prefix("--theme-id="))
        {
            let id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
            if theme::find(id).is_none() {
                return Err(format!("unknown theme ID {id}"));
            }
            options.theme_id = Some(id);
        } else if matches!(argument.as_str(), "--theme" | "--theme-id") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--theme requires a value".to_owned())?;
            let id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
            if theme::find(id).is_none() {
                return Err(format!("unknown theme ID {id}"));
            }
            options.theme_id = Some(id);
        } else if let Some(value) = argument
            .strip_prefix("--dark-theme=")
            .or_else(|| argument.strip_prefix("--dark-theme-id="))
        {
            let id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid dark theme value {value:?}: {error}"))?;
            if theme::find(id).is_none() {
                return Err(format!("unknown dark theme ID {id}"));
            }
            options.dark_theme_id = Some(id);
        } else if matches!(argument.as_str(), "--dark-theme" | "--dark-theme-id") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--dark-theme requires a value".to_owned())?;
            let id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid dark theme value {value:?}: {error}"))?;
            if theme::find(id).is_none() {
                return Err(format!("unknown dark theme ID {id}"));
            }
            options.dark_theme_id = Some(id);
        } else if let Some(value) = argument.strip_prefix("--center=") {
            options.center = parse_bool_flag(value, "center")?;
            center_explicit = true;
        } else if argument == "--center" || argument == "-c" {
            options.center = true;
            center_explicit = true;
        } else if argument == "--no-center" {
            options.center = false;
            center_explicit = true;
        } else {
            source_args.push(argument.clone());
        }
        index += 1;
    }
    Ok((source_args, options, center_explicit, sketch_explicit))
}

/// D2 v0.9.0 bundles document assets by default. Keep the decision at the
/// document boundary so the lower-level source commands remain deterministic
/// and callers can still request external SVG references with `--bundle=false`.
fn bundle_requested(args: &[String]) -> Result<bool, String> {
    let mut bundle = std::env::var("D2_BUNDLE")
        .ok()
        .map(|value| parse_bool_flag(&value, "D2_BUNDLE"))
        .transpose()?
        .unwrap_or(true);
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--bundle" || argument == "-b" {
            bundle = true;
        } else if argument == "--no-bundle" {
            bundle = false;
        } else if let Some(value) = argument.strip_prefix("--bundle=") {
            bundle = parse_bool_flag(value, "bundle")?;
        }
        index += 1;
    }
    Ok(bundle)
}

fn load_asset_data_with_policy(
    source_args: &[String],
    graph: &Graph,
    styles: &BTreeMap<d2_rs_core::NodeId, compiler::SourceNodeStyle>,
    include_remote: bool,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let loaded = load_asset_data_partial(source_args, graph, styles, include_remote);
    if loaded.failures.is_empty() {
        Ok(loaded.assets)
    } else {
        Err(loaded
            .failures
            .into_iter()
            .map(|(_, error)| error)
            .collect::<Vec<_>>()
            .join("; "))
    }
}

#[derive(Default)]
struct LoadedAssets {
    assets: BTreeMap<String, Vec<u8>>,
    content_types: BTreeMap<String, String>,
    failures: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AssetData {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

impl From<Vec<u8>> for AssetData {
    fn from(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            content_type: None,
        }
    }
}

fn load_asset_data_partial(
    source_args: &[String],
    graph: &Graph,
    styles: &BTreeMap<d2_rs_core::NodeId, compiler::SourceNodeStyle>,
    include_remote: bool,
) -> LoadedAssets {
    let mut icons = BTreeSet::new();
    icons.extend(styles.values().filter_map(|style| style.icon.clone()));
    // Edge icons live on the measured graph rather than in the node-style
    // projection. D2's image bundler walks both target shapes and
    // connections, so include them before resolving relative paths.
    icons.extend(
        graph
            .edges()
            .filter_map(|(edge_id, _)| graph.edge_icon(edge_id).map(str::to_owned)),
    );
    load_asset_hrefs(source_args, icons, include_remote, true)
}

fn collect_asset_results_concurrent(
    icons: Vec<String>,
    load: impl Fn(&str) -> Result<Option<AssetData>, String> + Sync,
) -> LoadedAssets {
    let context = watch::AssetWorkerContext::capture();
    let next = std::sync::atomic::AtomicUsize::new(0);
    // imgbundler.runResolverWorkersWithLimit cancels the worker context on
    // the first failure, so no further references are dispatched.
    let failed = std::sync::atomic::AtomicBool::new(false);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..icons.len().min(16) {
            let sender = sender.clone();
            let context = &context;
            let icons = &icons;
            let next = &next;
            let failed = &failed;
            let load = &load;
            scope.spawn(move || {
                context.run(|| {
                    loop {
                        if failed.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(icon) = icons.get(index) else { break };
                        let result = load(icon);
                        let stop = result.is_err();
                        if stop {
                            failed.store(true, std::sync::atomic::Ordering::Release);
                        }
                        if sender.send((icon.clone(), result)).is_err() || stop {
                            break;
                        }
                    }
                })
            });
        }
        drop(sender);
        let mut loaded = LoadedAssets::default();
        let progress_interval = std::time::Duration::from_secs(5);
        let mut next_progress = std::time::Instant::now() + progress_interval;
        loop {
            let (icon, result) = match receiver
                .recv_timeout(next_progress.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(result) => result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!("info: fetching images...");
                    next_progress += progress_interval;
                    continue;
                }
            };
            match result {
                Ok(Some(asset)) => {
                    if let Some(content_type) = asset.content_type {
                        loaded.content_types.insert(icon.clone(), content_type);
                    }
                    loaded.assets.insert(icon, asset.bytes);
                }
                Ok(None) => {}
                Err(error) => {
                    // maxReportedImageFailures / maxImageErrorLabelBytes.
                    if loaded.failures.len() < 8 {
                        eprintln!(
                            "err: failed to bundle image: {}",
                            bounded_label(&error, 1024)
                        );
                    }
                    loaded.failures.push((icon, error));
                }
            }
        }
        loaded
    })
}

/// imgbundler.boundedLabel: a byte-bounded diagnostic label.
fn bounded_label(label: &str, max_bytes: usize) -> String {
    if label.len() <= max_bytes {
        return label.to_owned();
    }
    let mut bytes = label.as_bytes()[..max_bytes - 3].to_vec();
    bytes.extend_from_slice(b"...");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// imgbundler.classifyReference: an http-prefixed URL scheme is remote.
fn imgbundler_remote_reference(href: &str) -> bool {
    raster_asset::uri_scheme(href)
        .is_some_and(|scheme| scheme.to_ascii_lowercase().starts_with("http"))
}

/// imgbundler.BundleWithResolver over one imageasset resolver session
/// (d2cli svgBundleResolver): local references always, remote references
/// only when bundling. Data URIs are already bundled.
fn load_asset_hrefs(
    source_args: &[String],
    icons: impl IntoIterator<Item = String>,
    include_remote: bool,
    validate: bool,
) -> LoadedAssets {
    let base = raster_asset::resolver_base(source_args);
    let load = |icon: &str| {
        if icon.is_empty() {
            return Ok(None);
        }
        if icon
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
        {
            if validate {
                validate_data_uri_asset(icon)?;
            }
            return Ok(None);
        }
        if imgbundler_remote_reference(icon) && !include_remote {
            return Ok(None);
        }
        match raster_asset::resolve(icon, &base)? {
            raster_asset::Outcome::Data(resolved) => Ok(Some(AssetData {
                bytes: resolved.bytes,
                content_type: Some(resolved.mime.to_owned()),
            })),
            raster_asset::Outcome::Unavailable(error) => Err(error),
        }
    };
    collect_asset_results_concurrent(icons.into_iter().collect(), load)
}

fn local_asset_read_error(path: &Path, error: io::Error) -> String {
    let operation = if error.kind() == io::ErrorKind::IsADirectory {
        "read"
    } else {
        "open"
    };
    #[cfg(unix)]
    let detail = match error.kind() {
        io::ErrorKind::NotFound => "no such file or directory".to_owned(),
        io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        io::ErrorKind::IsADirectory => "is a directory".to_owned(),
        _ => error.to_string(),
    };
    #[cfg(windows)]
    let detail = d2_rs_core::windows_io::error_message(&error);
    #[cfg(not(any(unix, windows)))]
    let detail = error.to_string();
    format!("{operation} {}: {detail}", path.display())
}

fn clean_asset_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match cleaned.components().next_back() {
                Some(Component::Normal(_)) => {
                    cleaned.pop();
                }
                Some(Component::RootDir) => {}
                _ => cleaned.push(".."),
            },
            _ => cleaned.push(component.as_os_str()),
        }
    }
    if cleaned.as_os_str().is_empty() {
        cleaned.push(".");
    }
    cleaned
}

#[cfg(test)]
fn svg_image_hrefs(svg: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(svg);
    let mut seen = BTreeSet::new();
    let mut hrefs = Vec::new();
    for suffix in text.split("<image ").skip(1) {
        let Some((tag, _)) = suffix.split_once('>') else {
            continue;
        };
        let value = tag
            .strip_prefix("href=\"")
            .or_else(|| tag.split_once(" href=\"").map(|(_, value)| value));
        let Some((href, _)) = value.and_then(|value| value.split_once('"')) else {
            continue;
        };
        let href = String::from_utf8_lossy(&unescape_html(href.as_bytes())).into_owned();
        if !href.starts_with("data:") && seen.insert(href.clone()) {
            hrefs.push(href);
        }
    }
    hrefs
}

const MAX_ASSET_BYTES: u64 = 1 << 25;
const MAX_DECODED_ASSET_DIMENSION: usize = 32_768;
const MAX_DECODED_ASSET_PIXELS: usize = 64 * 1024 * 1024;

fn has_http_asset_scheme(value: &str) -> bool {
    value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

/// Fetch one remote image using the same bounded, content-decoding boundary as
/// D2 v0.9.0's `imgbundler`.  Keeping this at the CLI/document boundary means
/// the core render and WASM APIs remain deterministic and callers can provide
/// their own asset loader when network access is unavailable.
#[cfg(test)]
fn fetch_remote_asset(url: &str) -> Result<Vec<u8>, String> {
    fetch_remote_asset_with_timeout(url, std::time::Duration::from_secs(60))
}

#[cfg(test)]
fn fetch_remote_asset_with_timeout(
    url: &str,
    timeout: std::time::Duration,
) -> Result<Vec<u8>, String> {
    fetch_remote_asset_data(url, timeout).map(|asset| asset.bytes)
}

#[cfg(test)]
fn fetch_remote_asset_data(url: &str, timeout: std::time::Duration) -> Result<AssetData, String> {
    if let Some(asset) = watch::cached_remote_asset_data(url) {
        return Ok(asset);
    }
    if let Some(error) = watch::active_cancellation_token().and_then(|token| token.error()) {
        return Err(format!("Get {url:?}: {error}"));
    }
    let (bytes, content_encoding, content_type) =
        http_asset::fetch(url, timeout, watch::active_cancellation_token())?;
    let bytes = decode_remote_content_encoding(bytes, &content_encoding).map_err(|error| {
        format!("failed to decode {content_encoding:?} response for {url}: {error}")
    })?;
    let asset = AssetData {
        bytes,
        content_type,
    };
    watch::cache_remote_asset_data(url, &asset);
    Ok(asset)
}

fn read_decoded_asset(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    reader
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut decoded)
        .map_err(decoded_asset_error)?;
    if decoded.len() as u64 > MAX_ASSET_BYTES {
        return Err(format!(
            "decoded image exceeds maximum size of {MAX_ASSET_BYTES} bytes"
        ));
    }
    Ok(decoded)
}

fn decoded_asset_error(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        "unexpected EOF".into()
    } else {
        match error.to_string().as_str() {
            "invalid gzip header" => "gzip: invalid header".into(),
            "corrupt gzip stream does not have a matching checksum" => {
                "gzip: invalid checksum".into()
            }
            _ => error.to_string(),
        }
    }
}

fn decode_remote_content_encoding(mut bytes: Vec<u8>, encodings: &str) -> Result<Vec<u8>, String> {
    for encoding in encodings.split(',').rev() {
        let encoding = encoding.trim().to_ascii_lowercase();
        bytes = match encoding.as_str() {
            "" | "identity" => bytes,
            "gzip" | "x-gzip" => {
                read_decoded_asset(flate2::read::MultiGzDecoder::new(bytes.as_slice()))?
            }
            "br" => read_decoded_asset(brotli_decompressor::Decompressor::new(
                bytes.as_slice(),
                4096,
            ))?,
            "deflate" => decode_remote_deflate(&bytes)?,
            _ => return Err(format!("unsupported content encoding {encoding:?}")),
        };
    }
    Ok(bytes)
}

/// Go's imgbundler accepts both RFC 1950 wrapped and raw RFC 1951 data.
fn decode_remote_deflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let zlib_header = bytes.len() >= 2
        && bytes[0] & 15 == 8
        && bytes[0] >> 4 <= 7
        && u16::from_be_bytes([bytes[0], bytes[1]]).is_multiple_of(31)
        && bytes[1] & 32 == 0;
    let reader: Box<dyn Read + '_> = if zlib_header {
        Box::new(flate2::read::ZlibDecoder::new(bytes))
    } else {
        Box::new(flate2::read::DeflateDecoder::new(bytes))
    };
    let mut decoded = Vec::new();
    reader
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut decoded)
        .map_err(decoded_asset_error)?;
    if decoded.len() as u64 > MAX_ASSET_BYTES {
        return Err(format!(
            "decoded image exceeds maximum size of {MAX_ASSET_BYTES} bytes"
        ));
    }
    Ok(decoded)
}

/// Applies D2 v0.9.0's bounded raster admission rules before an asset enters
/// the retained SVG/raster asset map. SVG XML stays provider-owned; recognized
/// raster signatures are probed here so malformed headers and oversized
/// decoded canvases fail before resvg or a package encoder sees them.
fn validate_asset_bytes(source: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(format!("asset {source:?} is empty"));
    }
    if bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err(format!(
            "asset {source:?} exceeds maximum size of {MAX_ASSET_BYTES} bytes"
        ));
    }
    let lower = source.to_ascii_lowercase();
    let signature = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("PNG")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("JPEG")
    } else if bytes.starts_with(b"GIF8") {
        Some("GIF")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("WebP")
    } else {
        None
    };
    let raster_hint = lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp");
    let Some(format) = signature else {
        if raster_hint {
            return Err(format!("asset {source:?} is not a valid raster image"));
        }
        return Ok(());
    };
    let size = imagesize::blob_size(bytes)
        .map_err(|error| format!("malformed {format} asset {source:?}: {error}"))?;
    if size.width == 0 || size.height == 0 {
        return Err(format!(
            "asset {source:?} has invalid decoded dimensions {}x{}",
            size.width, size.height
        ));
    }
    if size.width > MAX_DECODED_ASSET_DIMENSION || size.height > MAX_DECODED_ASSET_DIMENSION {
        return Err(format!(
            "asset {source:?} dimensions {}x{} exceed the {MAX_DECODED_ASSET_DIMENSION}-pixel limit",
            size.width, size.height
        ));
    }
    let pixels = size
        .width
        .checked_mul(size.height)
        .ok_or_else(|| format!("asset {source:?} decoded pixel count overflows usize"))?;
    if pixels > MAX_DECODED_ASSET_PIXELS {
        return Err(format!(
            "asset {source:?} decoded pixels {pixels} exceed the {MAX_DECODED_ASSET_PIXELS}-pixel limit"
        ));
    }
    Ok(())
}

fn validate_data_uri_asset(source: &str) -> Result<(), String> {
    let Some(payload) = source.strip_prefix("data:") else {
        return Ok(());
    };
    let Some(separator) = payload.find(',') else {
        return Err("data URI is missing its comma separator".into());
    };
    let metadata = &payload[..separator];
    let encoded = &payload[separator + 1..];
    let mut metadata_parts = metadata.split(';');
    let mime = metadata_parts
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_base64 = metadata_parts.any(|part| part.eq_ignore_ascii_case("base64"));
    let bytes = if is_base64 {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("invalid base64 data URI: {error}"))?
    } else {
        percent_decode_data_uri(encoded)?
    };
    match mime.as_str() {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => {
            validate_asset_bytes(source, &bytes)
        }
        "image/svg+xml" | "" => {
            if bytes.len() as u64 > MAX_ASSET_BYTES {
                Err(format!(
                    "asset {source:?} exceeds maximum size of {MAX_ASSET_BYTES} bytes"
                ))
            } else {
                Ok(())
            }
        }
        _ => Err(format!("unsupported image data URI MIME type {mime:?}")),
    }
}

fn percent_decode_data_uri(value: &str) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("data URI has an incomplete percent escape".into());
        }
        let high = hex_nibble(bytes[index + 1])
            .ok_or_else(|| "data URI has an invalid percent escape".to_owned())?;
        let low = hex_nibble(bytes[index + 2])
            .ok_or_else(|| "data URI has an invalid percent escape".to_owned())?;
        output.push((high << 4) | low);
        index += 3;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_bool_flag(value: &str, name: &str) -> Result<bool, String> {
    match value {
        "1" | "true" | "TRUE" | "True" | "t" | "T" => Ok(true),
        "0" | "false" | "FALSE" | "False" | "f" | "F" => Ok(false),
        _ => Err(format!(
            "invalid {name} value {value:?}; expected true or false"
        )),
    }
}

struct FontGraphMeasurer {
    font: TrueTypeFont,
    bold_font: TrueTypeFont,
    italic_font: TrueTypeFont,
    semibold_font: TrueTypeFont,
    mono_font: TrueTypeFont,
    mono_bold_font: TrueTypeFont,
    mono_semibold_font: TrueTypeFont,
    mono_italic_font: TrueTypeFont,
    unicode_mono_font: TrueTypeFont,
    unicode_mono_bold_font: TrueTypeFont,
    unicode_mono_italic_font: TrueTypeFont,
}

/// Font files selected by the D2 v0.9.0 command boundary.
///
/// D2 exposes a separate face selector for each text role.  Keeping the
/// paths together lets the graph measurer apply the same fallback rule as the
/// Go CLI: an omitted face uses its bundled Source Sans/Code Pro asset, while
/// the legacy `--font` spelling selects one file for every face.
#[derive(Clone, Debug, Default)]
struct FontPaths {
    regular: Option<String>,
    italic: Option<String>,
    bold: Option<String>,
    semibold: Option<String>,
    mono: Option<String>,
    mono_bold: Option<String>,
    mono_italic: Option<String>,
    mono_semibold: Option<String>,
}

impl FontPaths {
    fn all(path: String) -> Self {
        Self {
            regular: Some(path.clone()),
            italic: Some(path.clone()),
            bold: Some(path.clone()),
            semibold: Some(path.clone()),
            mono: Some(path.clone()),
            mono_bold: Some(path.clone()),
            mono_italic: Some(path.clone()),
            mono_semibold: Some(path),
        }
    }
}

/// d2cli.loadFonts / d2fonts.AddFontFamily. Keep raw faces alongside the
/// independent measurement fonts so d2svg subsets the actual supplied bytes.
fn load_render_fonts(args: &[String]) -> Result<d2_rs_text::d2fonts::CustomFonts, String> {
    use d2_rs_text::d2fonts::{CustomFonts, FontStyle};
    let (_, explicit) = extract_font_option(args)?;
    let environment = font_paths_from_environment();
    let mut fonts = CustomFonts::default();
    for (mono, style, path) in [
        (
            false,
            FontStyle::Regular,
            explicit.regular.or(environment.regular),
        ),
        (
            false,
            FontStyle::Italic,
            explicit.italic.or(environment.italic),
        ),
        (false, FontStyle::Bold, explicit.bold.or(environment.bold)),
        (
            false,
            FontStyle::Semibold,
            explicit.semibold.or(environment.semibold),
        ),
        (true, FontStyle::Regular, explicit.mono.or(environment.mono)),
        (
            true,
            FontStyle::Italic,
            explicit.mono_italic.or(environment.mono_italic),
        ),
        (
            true,
            FontStyle::Bold,
            explicit.mono_bold.or(environment.mono_bold),
        ),
        (
            true,
            FontStyle::Semibold,
            explicit.mono_semibold.or(environment.mono_semibold),
        ),
    ] {
        if let Some(path) = path.filter(|path| !path.is_empty()) {
            watch::record_dependency(&path);
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("could not read font {path:?}: {error}"))?;
            let faces = if mono {
                &mut fonts.mono
            } else {
                &mut fonts.regular
            };
            faces.insert(style, bytes);
        }
    }
    Ok(fonts)
}

fn font_paths_from_environment() -> FontPaths {
    fn value(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
    FontPaths {
        regular: value("D2_FONT_REGULAR"),
        italic: value("D2_FONT_ITALIC"),
        bold: value("D2_FONT_BOLD"),
        semibold: value("D2_FONT_SEMIBOLD"),
        mono: value("D2_FONT_MONO"),
        mono_bold: value("D2_FONT_MONO_BOLD"),
        mono_italic: value("D2_FONT_MONO_ITALIC"),
        mono_semibold: value("D2_FONT_MONO_SEMIBOLD"),
    }
}

fn font_argument(argument: &str) -> Option<(&str, &str)> {
    const FLAGS: [&str; 8] = [
        "--font-regular=",
        "--font-italic=",
        "--font-bold=",
        "--font-semibold=",
        "--font-mono=",
        "--font-mono-bold=",
        "--font-mono-italic=",
        "--font-mono-semibold=",
    ];
    FLAGS.iter().find_map(|prefix| {
        argument
            .strip_prefix(prefix)
            .map(|value| (prefix.trim_end_matches('=').trim_start_matches('-'), value))
    })
}

fn set_font_path(paths: &mut FontPaths, slot: &str, value: &str) {
    match slot {
        "font-regular" => paths.regular = Some(value.to_owned()),
        "font-italic" => paths.italic = Some(value.to_owned()),
        "font-bold" => paths.bold = Some(value.to_owned()),
        "font-semibold" => paths.semibold = Some(value.to_owned()),
        "font-mono" => paths.mono = Some(value.to_owned()),
        "font-mono-bold" => paths.mono_bold = Some(value.to_owned()),
        "font-mono-italic" => paths.mono_italic = Some(value.to_owned()),
        "font-mono-semibold" => paths.mono_semibold = Some(value.to_owned()),
        _ => {}
    }
}

impl FontGraphMeasurer {
    fn markdown_dimensions(
        &self,
        source: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
        mono_default: bool,
    ) -> Size {
        d2_rs_render::markdown_measure::MarkdownMeasurer {
            font: &self.font,
            bold_font: &self.bold_font,
            italic_font: &self.italic_font,
            semibold_font: &self.semibold_font,
            mono_font: &self.mono_font,
            mono_bold_font: &self.mono_bold_font,
            mono_semibold_font: &self.mono_semibold_font,
            mono_italic_font: &self.mono_italic_font,
        }
        .markdown_dimensions(source, font_size, bold, italic, mono_default)
    }
}

impl compiler::GraphMeasurer for FontGraphMeasurer {
    fn validate_markdown_label(&self, label: &str) -> Result<(), String> {
        d2_rs_text::markdown_measure::validate_markdown_label(label)
    }

    fn supports_shape_fitting(&self) -> bool {
        true
    }

    fn measure(&self, label: &str) -> Size {
        self.measure_at_size(label, 16)
    }

    fn measure_label(&self, label: &str) -> Size {
        self.measure_label_at_size(label, 16)
    }

    fn measure_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_at_size(label, font_size, false, false)
    }

    fn measure_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        let metrics = self.measure_styled_label_at_size(label, font_size, bold, italic);
        Size {
            // D2's default shape path measures bold labels with the Go ruler,
            // rounds each dimension upward, adds INNER_LABEL_PADDING (5 px),
            // then applies the rectangle's 40 px default shape padding.
            width: (metrics.width.ceil() + 45.0).max(40.0),
            height: (metrics.height.ceil() + 45.0).max(24.0),
        }
    }

    fn measure_mono_styled_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        let metrics = self.measure_mono_styled_label_at_size(label, font_size, bold, italic);
        Size {
            width: (metrics.width.ceil() + 45.0).max(40.0),
            height: (metrics.height.ceil() + 45.0).max(24.0),
        }
    }

    fn measure_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_label_at_size(label, font_size, false, false)
    }

    fn measure_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        let font = if bold {
            &self.bold_font
        } else if italic {
            &self.italic_font
        } else {
            &self.font
        };
        let fallback = if bold {
            &self.unicode_mono_bold_font
        } else if italic {
            &self.unicode_mono_italic_font
        } else {
            &self.unicode_mono_font
        };
        let metrics = font.measure_with_fallback(label, f64::from(font_size), fallback);
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn measure_code_label_at_size(&self, label: &str, font_size: u32) -> Size {
        let metrics = self.mono_font.measure_d2_code_label(
            label,
            f64::from(font_size),
            &self.unicode_mono_font,
        );
        Size {
            width: metrics.width.ceil(),
            height: metrics.height,
        }
    }

    fn measure_mono_label_at_size(&self, label: &str, font_size: u32) -> Size {
        let metrics = self.unicode_mono_font.measure_with_fallback(
            label,
            f64::from(font_size),
            &self.unicode_mono_font,
        );
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn measure_mono_styled_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        // d2graph.GetLabelSize maps style.font to the bundled D2 family;
        // the custom mono family is used for code and Markdown instead.
        let font = if bold {
            &self.unicode_mono_bold_font
        } else if italic {
            &self.unicode_mono_italic_font
        } else {
            &self.unicode_mono_font
        };
        let fallback = if bold {
            &self.unicode_mono_bold_font
        } else if italic {
            &self.unicode_mono_italic_font
        } else {
            &self.unicode_mono_font
        };
        let metrics = font.measure_with_fallback(label, f64::from(font_size), fallback);
        Size {
            width: metrics.width.ceil(),
            height: metrics.height.ceil(),
        }
    }

    fn measure_markdown_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_markdown_at_size(label, font_size, false, false)
    }

    fn measure_markdown_label_at_size(&self, label: &str, font_size: u32) -> Size {
        self.measure_styled_markdown_label_at_size(label, font_size, false, false)
    }

    fn measure_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        let metrics = self.markdown_dimensions(label, font_size, bold, italic, false);
        Size {
            width: (metrics.width + 45.0).max(40.0),
            height: (metrics.height + 45.0).max(24.0),
        }
    }

    fn measure_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.markdown_dimensions(label, font_size, bold, italic, false)
    }

    fn measure_mono_styled_markdown_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        let metrics = self.markdown_dimensions(label, font_size, bold, italic, true);
        Size {
            width: (metrics.width + 45.0).max(40.0),
            height: (metrics.height + 45.0).max(24.0),
        }
    }

    fn measure_mono_styled_markdown_label_at_size(
        &self,
        label: &str,
        font_size: u32,
        bold: bool,
        italic: bool,
    ) -> Size {
        self.markdown_dimensions(label, font_size, bold, italic, true)
    }

    fn measure_latex_at_size(&self, label: &str, font_size: u32) -> Size {
        let Ok(metrics) = d2_rs_math::measure(label) else {
            return self.measure_at_size(label, font_size);
        };
        // Original Object.GetLabelSize uses intrinsic TeX dimensions at every font size.
        Size {
            width: (metrics.width + 45.0).ceil().max(40.0),
            height: (metrics.height + 45.0).ceil().max(24.0),
        }
    }

    fn measure_latex_label_at_size(&self, label: &str, font_size: u32) -> Size {
        let Ok(metrics) = d2_rs_math::measure(label) else {
            return self.measure_label_at_size(label, font_size);
        };
        // Original Object.GetLabelSize uses intrinsic TeX dimensions at every font size.
        Size {
            width: (metrics.width).ceil().max(1.0),
            height: (metrics.height).ceil().max(1.0),
        }
    }
}

struct FallbackGraphMeasurer;

impl compiler::GraphMeasurer for FallbackGraphMeasurer {
    fn measure(&self, label: &str) -> Size {
        self.measure_at_size(label, 16)
    }

    fn measure_at_size(&self, label: &str, _font_size: u32) -> Size {
        Size {
            width: (label.chars().count() as f64 * 8.0 + 32.0).max(40.0),
            height: 40.0,
        }
    }

    fn measure_markdown_at_size(&self, label: &str, font_size: u32) -> Size {
        let rendered = plain_text(label.as_bytes());
        self.measure_at_size(&rendered, font_size)
    }

    fn measure_markdown_label_at_size(&self, label: &str, _font_size: u32) -> Size {
        let rendered = plain_text(label.as_bytes());
        Size {
            width: (rendered.chars().count() as f64 * 8.0).max(8.0),
            height: 16.0,
        }
    }

    fn measure_latex_at_size(&self, label: &str, font_size: u32) -> Size {
        let Ok(metrics) = d2_rs_math::measure(label) else {
            return self.measure_at_size(label, font_size);
        };
        // Original Object.GetLabelSize uses intrinsic TeX dimensions at every font size.
        Size {
            width: (metrics.width + 45.0).ceil().max(40.0),
            height: (metrics.height + 45.0).ceil().max(24.0),
        }
    }

    fn measure_latex_label_at_size(&self, label: &str, font_size: u32) -> Size {
        let Ok(metrics) = d2_rs_math::measure(label) else {
            return self.measure_label_at_size(label, font_size);
        };
        // Original Object.GetLabelSize uses intrinsic TeX dimensions at every font size.
        Size {
            width: (metrics.width).ceil().max(1.0),
            height: (metrics.height).ceil().max(1.0),
        }
    }
}

fn load_graph_measurer_with_paths(paths: &FontPaths) -> Result<Option<FontGraphMeasurer>, String> {
    load_graph_measurer_with_mode(paths, false)
}

fn load_graph_measurer_with_mode(
    paths: &FontPaths,
    hand_drawn: bool,
) -> Result<Option<FontGraphMeasurer>, String> {
    // d2lib.applyDefaults selects HandDrawn only without a primary family.
    // d2cli.loadFonts creates that family if any proportional face is supplied;
    // AddFontFamily fills its other styles from SourceSansPro.
    let hand_drawn = hand_drawn
        && paths.regular.is_none()
        && paths.italic.is_none()
        && paths.bold.is_none()
        && paths.semibold.is_none();
    fn load_face<F>(path: Option<&str>, bundled: F, role: &str) -> Result<TrueTypeFont, String>
    where
        F: FnOnce() -> Result<TrueTypeFont, String>,
    {
        if let Some(path) = path {
            watch::record_dependency(path);
            let bytes = std::fs::read(path)
                .map_err(|error| format!("could not read {role} font {path:?}: {error}"))?;
            TrueTypeFont::parse(&bytes)
                .map_err(|error| format!("could not parse {role} font {path:?}: {error}"))
        } else {
            bundled()
        }
    }

    let font = load_face(
        paths.regular.as_deref(),
        || {
            if hand_drawn {
                bundled_fuzzy_bubbles_regular()
                    .map_err(|error| format!("could not parse bundled Fuzzy Bubbles font: {error}"))
            } else {
                bundled_source_sans_pro_regular().map_err(|error| {
                    format!("could not parse bundled Source Sans Pro font: {error}")
                })
            }
        },
        "regular",
    )?;
    let bold_font = load_face(
        paths.bold.as_deref(),
        || {
            if hand_drawn {
                bundled_fuzzy_bubbles_bold().map_err(|error| {
                    format!("could not parse bundled Fuzzy Bubbles Bold font: {error}")
                })
            } else {
                bundled_source_sans_pro_bold().map_err(|error| {
                    format!("could not parse bundled Source Sans Pro Bold font: {error}")
                })
            }
        },
        "bold",
    )?;
    let italic_font = load_face(
        paths.italic.as_deref(),
        || {
            if hand_drawn {
                bundled_fuzzy_bubbles_regular().map_err(|error| {
                    format!("could not parse bundled Fuzzy Bubbles italic fallback: {error}")
                })
            } else {
                bundled_source_sans_pro_italic().map_err(|error| {
                    format!("could not parse bundled Source Sans Pro Italic font: {error}")
                })
            }
        },
        "italic",
    )?;
    let semibold_font = load_face(
        paths.semibold.as_deref(),
        || {
            if hand_drawn {
                bundled_fuzzy_bubbles_bold().map_err(|error| {
                    format!("could not parse bundled Fuzzy Bubbles semibold fallback: {error}")
                })
            } else {
                bundled_source_sans_pro_semibold().map_err(|error| {
                    format!("could not parse bundled Source Sans Pro Semibold font: {error}")
                })
            }
        },
        "semibold",
    )?;
    let custom_mono = paths.mono.is_some()
        || paths.mono_bold.is_some()
        || paths.mono_italic.is_some()
        || paths.mono_semibold.is_some();
    let mono_font = load_face(
        paths.mono.as_deref(),
        || {
            if custom_mono {
                bundled_source_sans_pro_regular()
            } else {
                bundled_source_code_pro_regular()
            }
            .map_err(|error| format!("could not parse bundled Source Code Pro font: {error}"))
        },
        "mono",
    )?;
    let mono_bold_font = load_face(
        paths.mono_bold.as_deref(),
        || {
            (if custom_mono {
                bundled_source_sans_pro_bold()
            } else {
                bundled_source_code_pro_bold()
            })
            .map_err(|error| format!("could not parse bundled Source Code Pro Bold font: {error}"))
        },
        "mono-bold",
    )?;
    let mono_italic_font = load_face(
        paths.mono_italic.as_deref(),
        || {
            (if custom_mono {
                bundled_source_sans_pro_italic()
            } else {
                bundled_source_code_pro_italic()
            })
            .map_err(|error| {
                format!("could not parse bundled Source Code Pro Italic font: {error}")
            })
        },
        "mono-italic",
    )?;
    let mono_semibold_font = load_face(
        paths.mono_semibold.as_deref(),
        || {
            (if custom_mono {
                bundled_source_sans_pro_semibold()
            } else {
                d2_rs_text::measure::bundled_source_code_pro_semibold()
            })
            .map_err(|error| {
                format!("could not parse bundled Source Code Pro Semibold font: {error}")
            })
        },
        "mono-semibold",
    )?;
    // Go's scaleUnicode always uses the bundled Source Code Pro face, even
    // when the caller supplies a different mono family.
    let unicode_mono_font = if custom_mono {
        bundled_source_code_pro_regular().map_err(|e| e.to_string())?
    } else {
        mono_font.clone()
    };
    let unicode_mono_bold_font = if custom_mono {
        bundled_source_code_pro_bold().map_err(|e| e.to_string())?
    } else {
        mono_bold_font.clone()
    };
    let unicode_mono_italic_font = if custom_mono {
        bundled_source_code_pro_italic().map_err(|e| e.to_string())?
    } else {
        mono_italic_font.clone()
    };
    Ok(Some(FontGraphMeasurer {
        font,
        bold_font,
        italic_font,
        semibold_font,
        mono_font,
        mono_bold_font,
        mono_semibold_font,
        mono_italic_font,
        unicode_mono_font,
        unicode_mono_bold_font,
        unicode_mono_italic_font,
    }))
}

fn parse_source(input: &[u8], args: &[String]) -> Result<d2_rs_core::ast::Map, String> {
    let (path, utf16_positions) = parse_source_options(args)?;
    let output = parse(
        path,
        input,
        Some(ParseOptions {
            utf16_positions,
            errors: None,
        }),
    );
    if let Some(error) = output.error {
        return Err(error.to_string());
    }
    Ok(output.map)
}

fn reject_compiler_errors(compiler: &compiler::Compiler) -> Result<(), String> {
    if !compiler.errors.is_empty() {
        return Err(
            compiler::GraphCompileError::InvalidAttributes(compiler.errors.clone()).to_string(),
        );
    }
    Ok(())
}

fn format_source(input: &[u8], args: &[String]) -> Result<Vec<u8>, String> {
    // d2cli/fmt.go passes its caller's context to d2parser.ParseContext.
    // Keep the parser's polling and context-error precedence before any write.
    let (path, utf16_positions) = parse_source_options(args)?;
    let context = watch::active_cancellation_token()
        .map(|token| Box::new(move || token.error()) as d2_rs_language::parser::ParseContext);
    let parsed = d2_rs_language::parse_with_context(
        path,
        input,
        Some(ParseOptions {
            utf16_positions,
            errors: None,
        }),
        context,
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    Ok(format::format(&Child::Map(Shared::new(parsed.map))))
}

fn compile_source(input: &[u8], args: &[String]) -> Result<Vec<u8>, String> {
    let (path, utf16_positions) = parse_source_options(args)?;
    let parsed = parse(
        path,
        input,
        Some(ParseOptions {
            utf16_positions,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(filesystem_import_opener()),
            utf16_positions,
        },
    )
    .map_err(|error| error.to_string())?;
    reject_compiler_errors(&compiler)?;
    serde_json::to_vec(&compiler::summarize_ir(&compiler, root))
        .map_err(|error| format!("could not encode IR: {error}"))
}

fn validate_source(input: &[u8], args: &[String]) -> Result<(), String> {
    let (path, utf16_positions) = parse_source_options(args)?;
    let parsed = parse(
        path,
        input,
        Some(ParseOptions {
            utf16_positions,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    // v0.9.0's `validate` command is a syntax/parse check. Compiler
    // diagnostics (for example an unknown theme or an image without an
    // icon) are raised by the document/layout path instead of this command.
    // Keep the parsed map alive until this boundary so the check remains
    // explicit without lowering or resolving imports.
    let _ = (parsed.map, utf16_positions);
    Ok(())
}

fn graph_source(input: &[u8], args: &[String]) -> Result<Vec<u8>, String> {
    let (source_args, font_paths) = extract_font_option(args)?;
    let (path, utf16_positions) = parse_source_options(&source_args)?;
    let parsed = parse(
        path,
        input,
        Some(ParseOptions {
            utf16_positions,
            errors: None,
        }),
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(filesystem_import_opener()),
            utf16_positions,
        },
    )
    .map_err(|error| error.to_string())?;
    let measurer = load_graph_measurer_with_paths(&font_paths)?;
    let graph = match measurer.as_ref() {
        Some(measurer) => compiler::lower_graph_with_measurer(&compiler, root, measurer),
        None => compiler::lower_graph_with_measurer(&compiler, root, &FallbackGraphMeasurer),
    }
    .map_err(|error| error.to_string())?;
    // Graph keeps sequence-diagram roots as adapter metadata so the ordinary
    // shape enum remains the same for layout and rendering. The source graph
    // boundary mirrors d2graph's JSON, where those roots are exposed as the
    // public `sequence_diagram` shape.
    let mut output =
        serde_json::to_value(&graph).map_err(|error| format!("could not encode graph: {error}"))?;
    if let Some(nodes) = output
        .get_mut("nodes")
        .and_then(serde_json::Value::as_array_mut)
    {
        for (index, node) in nodes.iter_mut().enumerate() {
            if graph.is_sequence_diagram(NodeId(index as u32)) {
                node["shape"] = serde_json::Value::String("sequence_diagram".into());
            }
        }
    }
    serde_json::to_vec(&output).map_err(|error| format!("could not encode graph: {error}"))
}

type SourceConfigResult = Result<Option<compiler::DiagramConfig>, String>;

/// One document's source-config result, shared with its nested renders. The
/// borrowed input cannot change while its scope is live. Direct render callers
/// establish their own scope and compile config once.
#[derive(Clone)]
struct SourceConfigMemo {
    input: (usize, usize),
    path: String,
    result: Option<SourceConfigResult>,
}

thread_local! {
    static SOURCE_CONFIG_MEMO: std::cell::RefCell<Option<SourceConfigMemo>> =
        const { std::cell::RefCell::new(None) };
}

struct SourceConfigScope(Option<SourceConfigMemo>);

impl SourceConfigScope {
    fn enter(input: &[u8]) -> Self {
        let key = (input.as_ptr() as usize, input.len());
        SOURCE_CONFIG_MEMO.with(|slot| {
            let mut slot = slot.borrow_mut();
            // A document's board renders read the same borrowed source. Carry
            // its config into nested renders, while restoring the caller's
            // memo (including its path) when each render finishes.
            let memo = slot
                .as_ref()
                .filter(|memo| memo.input == key)
                .cloned()
                .unwrap_or_else(|| SourceConfigMemo {
                    input: key,
                    path: String::new(),
                    result: None,
                });
            Self(slot.replace(memo))
        })
    }
}

impl Drop for SourceConfigScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        SOURCE_CONFIG_MEMO.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// One parsed and compiled document source, shared by board planning and
/// every board render of that document. d2lib.compileInput compiles a source
/// once; parsing and compiling the same borrowed bytes again with the same
/// path and options yields the same IR, so renders reuse it. Lowering reads
/// the IR without changing it, except for the compilation-wide copy budget,
/// which each reuse restores to its value when compilation returned.
struct CompiledSource {
    input: (usize, usize),
    path: String,
    compiler: compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
    variable_expansion_used: u64,
}

thread_local! {
    static COMPILED_SOURCE_MEMO: std::cell::RefCell<Option<std::rc::Rc<CompiledSource>>> =
        const { std::cell::RefCell::new(None) };
}

impl CompiledSource {
    /// `compiler` must come straight from `compile_board_source(input, path)`.
    fn new(
        input: &[u8],
        path: &str,
        compiler: compiler::Compiler,
        root: d2_rs_core::ir::NodeId,
    ) -> std::rc::Rc<Self> {
        let variable_expansion_used = compiler.variable_expansion_usage().1;
        // Planning validates each board before its render lowers it again.
        compiler.share_graph_validation();
        std::rc::Rc::new(Self {
            input: (input.as_ptr() as usize, input.len()),
            path: path.to_owned(),
            compiler,
            root,
            variable_expansion_used,
        })
    }
}

/// Publishes a document's compilation to its renders while the document's
/// borrowed input is live, then restores the caller's memo.
struct CompiledSourceScope(Option<std::rc::Rc<CompiledSource>>);

impl CompiledSourceScope {
    fn enter(compiled: std::rc::Rc<CompiledSource>) -> Self {
        Self(COMPILED_SOURCE_MEMO.with(|slot| slot.borrow_mut().replace(compiled)))
    }
}

impl Drop for CompiledSourceScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        COMPILED_SOURCE_MEMO.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// The document compilation of `input` at `path`, when the caller's parse and
/// compile options match `compile_board_source` (no UTF-16 positions).
fn reuse_compiled_source(
    input: &[u8],
    path: &str,
    utf16_positions: bool,
) -> Option<std::rc::Rc<CompiledSource>> {
    if utf16_positions {
        return None;
    }
    let key = (input.as_ptr() as usize, input.len());
    let compiled = COMPILED_SOURCE_MEMO.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|compiled| compiled.input == key && compiled.path == path)
            .cloned()
    })?;
    compiled
        .compiler
        .restore_variable_expansion_used(compiled.variable_expansion_used);
    Some(compiled)
}

fn source_config(input: &[u8], path: &str) -> SourceConfigResult {
    let key = (input.as_ptr() as usize, input.len());
    let cached = SOURCE_CONFIG_MEMO.with(|slot| {
        slot.borrow().as_ref().and_then(|memo| {
            (memo.input == key && memo.path == path)
                .then(|| memo.result.clone())
                .flatten()
        })
    });
    if let Some(result) = cached {
        return result;
    }
    let result = compute_source_config(input, path);
    remember_source_config(input, path, &result);
    result
}

fn remember_source_config(input: &[u8], path: &str, result: &SourceConfigResult) {
    let key = (input.as_ptr() as usize, input.len());
    SOURCE_CONFIG_MEMO.with(|slot| {
        if let Some(memo) = slot.borrow_mut().as_mut()
            && memo.input == key
        {
            memo.path = path.to_owned();
            memo.result = Some(result.clone());
        }
    });
}

fn compute_source_config(input: &[u8], path: &str) -> SourceConfigResult {
    let (compiler, root) = compile_board_source(input, path)?;
    source_config_from_compiled(&compiler, root)
}

fn source_config_from_compiled(
    compiler: &compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
) -> SourceConfigResult {
    compiler::extract_config_checked(compiler, root).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Returns the synthetic target-root metadata used by D2's target hash. The
/// document root carries the Config pointer; selected child boards carry
/// their board name (or explicit label) and no Config pointer.
fn source_target_hash_metadata_from_compiled(
    compiler: &compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
    source_args: &[String],
) -> Result<(Option<String>, bool), String> {
    let target = source_args.iter().find_map(|argument| {
        argument
            .strip_prefix("--target=")
            .map(str::to_owned)
            .or_else(|| {
                (argument == "--target")
                    .then(|| source_args.windows(2).find(|pair| pair[0] == "--target"))
                    .flatten()
                    .map(|pair| pair[1].clone())
            })
    });
    let Some(target) = target.filter(|target| !target.is_empty() && target != "*") else {
        return Ok((compiler::board_label(compiler, root), true));
    };
    let target = target.strip_suffix(".*").unwrap_or(&target);
    let board = compiler::resolve_board_target(compiler, root, target)
        .map_err(|error| error.to_string())?;
    if let Some(label) = compiler::board_label(compiler, board) {
        return Ok((Some(label), false));
    }
    let field = match compiler.arena.node(board) {
        d2_rs_core::ir::Node::Field(_) => Some(board),
        _ => compiler.arena.parent_field(board),
    };
    let name = field.and_then(|field| match compiler.arena.node(field) {
        d2_rs_core::ir::Node::Field(field) => Some(field.name.scalar_string().to_owned()),
        _ => None,
    });
    Ok((name, false))
}

fn source_path_from_args(args: &[String]) -> String {
    args.iter()
        .find_map(|argument| argument.strip_prefix("--path=").map(str::to_owned))
        .unwrap_or_else(|| "-".to_owned())
}

fn hydrate_render_theme(
    input: &[u8],
    source_args: &[String],
    theme_id: Option<i64>,
) -> Result<Option<i64>, String> {
    if theme_id.is_some() {
        return Ok(theme_id);
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(None);
    };
    let Some(configured_theme) = configured.theme_id else {
        return Ok(None);
    };
    if theme::find(configured_theme).is_none() {
        return Err(format!("unknown theme ID {configured_theme}"));
    }
    Ok(Some(configured_theme))
}

fn hydrate_render_center(
    input: &[u8],
    source_args: &[String],
    center: bool,
    explicit: bool,
) -> Result<bool, String> {
    if explicit {
        return Ok(center);
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(center);
    };
    Ok(configured.center.unwrap_or(center))
}

fn hydrate_render_sketch(
    input: &[u8],
    source_args: &[String],
    explicit: bool,
) -> Result<bool, String> {
    if explicit {
        return Ok(true);
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(false);
    };
    Ok(configured.sketch.unwrap_or(false))
}

fn hydrate_render_theme_overrides(
    input: &[u8],
    source_args: &[String],
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    if !overrides.is_empty() {
        return Ok(overrides.clone());
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(BTreeMap::new());
    };
    Ok(configured.theme_overrides)
}

fn hydrate_render_dark_theme_overrides(
    input: &[u8],
    source_args: &[String],
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    if !overrides.is_empty() {
        return Ok(overrides.clone());
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(BTreeMap::new());
    };
    Ok(configured.dark_theme_overrides)
}

fn hydrate_render_dark_theme(
    input: &[u8],
    source_args: &[String],
    dark_theme_id: Option<i64>,
) -> Result<Option<i64>, String> {
    if dark_theme_id.is_some() {
        return Ok(dark_theme_id);
    }
    let Some(configured) = source_config(input, &source_path_from_args(source_args))? else {
        return Ok(None);
    };
    if let Some(id) = configured.dark_theme_id
        && theme::find(id).is_none()
    {
        return Err(format!("unknown dark theme ID {id}"));
    }
    Ok(configured.dark_theme_id)
}

#[derive(Clone, Debug, Default)]
struct SourceBoardPlan {
    targets: Vec<String>,
    all_targets: Vec<String>,
    postorder_targets: Vec<String>,
    root_renderable: bool,
    renderable_targets: std::collections::BTreeSet<String>,
}

fn animation_subtree_plan(
    plan: SourceBoardPlan,
    target: &str,
) -> Result<(String, SourceBoardPlan), String> {
    let parts = d2_rs_language::parse_key(target.as_bytes())
        .map_err(|e| format!("invalid board target: {e}"))?
        .string_ida()
        .ok_or_else(|| "board target has no components".to_owned())?;
    let root = plan
        .all_targets
        .iter()
        .find(|p| compiled_board_components(p) == parts)
        .cloned()
        .ok_or_else(|| format!("render target {target:?} not found"))?;
    let inside = |p: &String| {
        let child = compiled_board_components(p);
        child.len() > parts.len() && child.starts_with(&parts)
    };
    let root_renderable = plan.renderable_targets.contains(&root);
    let selected = SourceBoardPlan {
        targets: plan.targets.into_iter().filter(&inside).collect(),
        all_targets: plan.all_targets.into_iter().filter(&inside).collect(),
        postorder_targets: plan.postorder_targets.into_iter().filter(&inside).collect(),
        root_renderable,
        renderable_targets: plan
            .renderable_targets
            .into_iter()
            .filter(|p| p == &root || inside(p))
            .collect(),
    };
    Ok((root, selected))
}

fn validate_animation_destination(
    input_path: &str,
    output_path: Option<&str>,
    root_target: &str,
    children: &[String],
) -> Result<(), String> {
    let stdout = output_path == Some("-") || (output_path.is_none() && input_path == "-");
    // Go prepends a selected board's name before recursively checking its
    // output path. Named subtree animations therefore support stdout; an
    // unnamed document root with children rejects it, even for empty folders.
    let named = !root_target.is_empty()
        && compiled_board_components(root_target)
            .last()
            .is_some_and(|name| !name.is_empty());
    if stdout && !children.is_empty() && !named {
        return Err("multiboard output cannot be written to stdout".to_owned());
    }
    Ok(())
}

#[cfg(test)]
fn source_board_plan(input: &[u8], path: &str) -> Result<SourceBoardPlan, String> {
    source_board_plan_for_target(input, path, "").map(|(plan, _)| plan)
}

fn source_board_plan_for_target(
    input: &[u8],
    path: &str,
    target: &str,
) -> Result<(SourceBoardPlan, String), String> {
    let (compiler, root) = compile_board_source(input, path)?;
    source_board_plan_from_compiled(&compiler, root, target)
}

/// Parse and compile the board tree for planning, with planning's errors.
fn compile_board_source(
    input: &[u8],
    path: &str,
) -> Result<(compiler::Compiler, d2_rs_core::ir::NodeId), String> {
    let parsed = parse(
        path,
        input,
        Some(ParseOptions {
            errors: None,
            ..Default::default()
        }),
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(filesystem_import_opener()),
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    if !compiler.errors.is_empty() {
        return Err(
            compiler::GraphCompileError::InvalidAttributes(compiler.errors.clone()).to_string(),
        );
    }
    validate_graph_compilation(&compiler, root)?;
    Ok((compiler, root))
}

fn validate_graph_compilation(
    compiler: &compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
) -> Result<(), String> {
    // Go compiles every board with one shared budget before selecting a
    // target. Later immutable planning/rendering views start from the IR
    // checkpoint after this complete compilation has passed its budget.
    let expansion_used = compiler.variable_expansion_usage().1;
    compiler
        .validate_graph_materialization(root)
        .map_err(|error| error.to_string())?;
    compiler.restore_variable_expansion_used(expansion_used);
    Ok(())
}

fn source_board_plan_from_compiled(
    compiler: &compiler::Compiler,
    root: d2_rs_core::ir::NodeId,
    target: &str,
) -> Result<(SourceBoardPlan, String), String> {
    // The Go compiler marks a board as folder-only when it has children but
    // no concrete objects of its own. Lowering the root through the same
    // source graph path gives the native CLI that distinction without
    // treating metadata such as `label` as a drawable node.
    let selected = compiler::resolve_board_target(compiler, root, target)
        .map_err(|error| error.to_string())?;
    let mut selected_target = String::new();
    let variable_expansion_used = compiler.variable_expansion_usage().1;
    let root_renderable =
        compiler::board_is_renderable(compiler, root).map_err(|error| error.to_string())?;
    let mut targets = Vec::new();
    let mut all_targets = Vec::new();
    let mut renderable_targets = std::collections::BTreeSet::new();
    for (target, board) in compiler::board_targets(compiler, root) {
        if board == selected {
            selected_target = target.clone();
        }
        all_targets.push(target.clone());
        // The root plan has compiled the complete board tree and charged
        // every automatic copy. Child plans inspect the same compilation;
        // they must not accumulate a second document's copy work.
        compiler.restore_variable_expansion_used(variable_expansion_used);
        let renderable =
            compiler::board_is_renderable(compiler, board).map_err(|error| error.to_string())?;
        if renderable {
            renderable_targets.insert(target.clone());
            targets.push(target);
        }
    }
    let postorder_targets = compiler::board_targets_postorder(compiler, root)
        .into_iter()
        .filter_map(|(target, _)| renderable_targets.contains(&target).then_some(target))
        .collect();
    Ok((
        SourceBoardPlan {
            postorder_targets,
            targets,
            all_targets,
            root_renderable,
            renderable_targets,
        },
        selected_target,
    ))
}

#[derive(Clone, Debug, Default)]
struct SourceBoardTitles {
    root_label: Option<String>,
    board_labels: std::collections::BTreeMap<String, Option<String>>,
}

fn source_board_titles(
    input: &[u8],
    path: &str,
    targets: &[String],
) -> Result<SourceBoardTitles, String> {
    let parsed = parse(
        path,
        input,
        Some(ParseOptions {
            errors: None,
            ..Default::default()
        }),
    );
    if let Some(error) = parsed.error {
        return Err(error.to_string());
    }
    let (compiler, root) = compiler::compile_ir_with_options(
        Shared::new(parsed.map),
        compiler::CompileOptions {
            import_opener: Some(filesystem_import_opener()),
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    if !compiler.errors.is_empty() {
        return Err(
            compiler::GraphCompileError::InvalidAttributes(compiler.errors.clone()).to_string(),
        );
    }
    let mut result = SourceBoardTitles {
        root_label: compiler::board_label(&compiler, root).filter(|label| !label.is_empty()),
        ..Default::default()
    };
    for target in targets.iter().filter(|target| !target.is_empty()) {
        let board = compiler::resolve_board_target(&compiler, root, target)
            .map_err(|error| error.to_string())?;
        result
            .board_labels
            .insert(target.clone(), compiler::board_label(&compiler, board));
    }
    Ok(result)
}

type SourceLayout = (
    d2_rs_core::Graph,
    d2_rs_tala::LayoutResult,
    std::collections::BTreeMap<d2_rs_core::NodeId, String>,
    std::collections::BTreeMap<d2_rs_core::NodeId, compiler::SourceNodeStyle>,
);

fn node_sequence_owner(graph: &Graph, node_id: NodeId) -> Option<NodeId> {
    let mut current = Some(node_id);
    while let Some(id) = current {
        if graph.is_sequence_diagram(id) {
            return Some(id);
        }
        current = graph.node(id).and_then(|node| node.parent);
    }
    None
}

fn edge_is_sequence_scoped(graph: &Graph, edge: &d2_rs_core::Edge) -> bool {
    if graph.root_sequence_diagram {
        return true;
    }
    let source_owner = node_sequence_owner(graph, edge.source);
    source_owner.is_some() && source_owner == node_sequence_owner(graph, edge.target)
}

/// Carry the target-root metadata out of the same IR compilation as layout,
/// as d2lib.compileInput/d2exporter.Export do. Keep its result separate so
/// render callers consume errors at their existing post-layout boundary;
/// layout-only callers do not validate render metadata.
#[derive(Clone)]
struct SourceRenderLayout {
    layout: SourceLayout,
    target_metadata: Result<(Option<String>, bool), String>,
}

// d2lib.compileBoard lays out the whole tree before d2cli starts rendering.
// Retain those layouts only for this document's traversal; subsequent renders
// and recursive target hashes reuse them without running an engine again.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct BoardLayoutKey {
    input: (usize, usize),
    args: Vec<String>,
    theme: Option<i64>,
    sketch: bool,
}

type BoardLayouts = BTreeMap<BoardLayoutKey, SourceRenderLayout>;

thread_local! {
    static BOARD_LAYOUTS: std::cell::RefCell<Option<BoardLayouts>> =
        const { std::cell::RefCell::new(None) };
}

/// The board tree compiled for recursive d2svg hashes. Every board of one
/// document traversal hashes against the same source, so the traversal
/// compiles it once and plans its complete target list at most once.
struct SubtreeHashPlan {
    compiled: Result<(compiler::Compiler, d2_rs_core::ir::NodeId), String>,
    all_targets: Option<Result<Vec<String>, String>>,
}

type SubtreeHashPlans = BTreeMap<((usize, usize), String), SubtreeHashPlan>;

thread_local! {
    static SUBTREE_HASH_PLANS: std::cell::RefCell<Option<SubtreeHashPlans>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` on this document's hash plan, retained across the boards of one
/// traversal (BoardLayoutScope) and built afresh outside one.
fn with_subtree_hash_plan<T>(
    input: &[u8],
    path: &str,
    f: impl FnOnce(&mut SubtreeHashPlan) -> T,
) -> T {
    let fresh = || SubtreeHashPlan {
        compiled: compile_board_source(input, path),
        all_targets: None,
    };
    let key = ((input.as_ptr() as usize, input.len()), path.to_owned());
    let cached = SUBTREE_HASH_PLANS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let plans = slot.as_mut()?;
        Some(plans.remove(&key).unwrap_or_else(fresh))
    });
    let Some(mut plan) = cached else {
        return f(&mut fresh());
    };
    let result = f(&mut plan);
    SUBTREE_HASH_PLANS.with(|slot| {
        if let Some(plans) = slot.borrow_mut().as_mut() {
            plans.insert(key, plan);
        }
    });
    result
}

struct BoardLayoutScope(Option<BoardLayouts>, Option<SubtreeHashPlans>);

impl BoardLayoutScope {
    fn enter() -> Self {
        Self(
            BOARD_LAYOUTS.with(|slot| slot.borrow_mut().replace(BTreeMap::new())),
            SUBTREE_HASH_PLANS.with(|slot| slot.borrow_mut().replace(BTreeMap::new())),
        )
    }
}

impl Drop for BoardLayoutScope {
    fn drop(&mut self) {
        BOARD_LAYOUTS.with(|slot| *slot.borrow_mut() = self.0.take());
        SUBTREE_HASH_PLANS.with(|slot| *slot.borrow_mut() = self.1.take());
    }
}

fn source_layout(input: &[u8], args: &[String]) -> Result<SourceLayout, String> {
    source_layout_with_theme(input, args, None, false)
}

fn source_layout_with_theme(
    input: &[u8],
    args: &[String],
    requested_theme_id: Option<i64>,
    requested_sketch: bool,
) -> Result<SourceLayout, String> {
    source_render_layout_with_theme(input, args, requested_theme_id, requested_sketch)
        .map(|compiled| compiled.layout)
}

fn source_render_layout_with_theme(
    input: &[u8],
    args: &[String],
    requested_theme_id: Option<i64>,
    requested_sketch: bool,
) -> Result<SourceRenderLayout, String> {
    let key = BOARD_LAYOUTS.with(|slot| {
        slot.borrow().as_ref().map(|_| BoardLayoutKey {
            input: (input.as_ptr() as usize, input.len()),
            args: args.to_vec(),
            theme: requested_theme_id,
            sketch: requested_sketch,
        })
    });
    if let Some(cached) = key
        .as_ref()
        .and_then(|key| BOARD_LAYOUTS.with(|slot| slot.borrow().as_ref()?.get(key).cloned()))
    {
        return Ok(cached);
    }
    let compiled =
        source_render_layout_uncached(input, args, requested_theme_id, requested_sketch)?;
    if let Some(key) = key {
        BOARD_LAYOUTS.with(|slot| {
            if let Some(layouts) = slot.borrow_mut().as_mut() {
                layouts.insert(key, compiled.clone());
            }
        });
    }
    Ok(compiled)
}

fn source_render_layout_uncached(
    input: &[u8],
    args: &[String],
    requested_theme_id: Option<i64>,
    requested_sketch: bool,
) -> Result<SourceRenderLayout, String> {
    let mut source_args = Vec::new();
    let mut font_paths = font_paths_from_environment();
    let mut seeds = DEFAULT_SEEDS.to_vec();
    let mut engine = EngineKind::Tala;
    let mut engine_explicit = false;
    let mut engine_options = d2_rs_core::LayoutEngineOptions::default();
    let mut target = None;
    let mut document_link_format = None;
    let mut document_link_extensionless = false;
    let mut timeout = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if let Some((name, value)) = command_line::engine_option(args, &mut index)? {
            command_line::set_engine_option(&mut engine_options, &name, value);
            index += 1;
            continue;
        }
        let value = if let Some(value) = argument.strip_prefix("--seeds=") {
            Some(value)
        } else if argument == "--seeds" {
            index += 1;
            args.get(index).map(String::as_str)
        } else if let Some(value) = argument.strip_prefix("--font=") {
            if value.is_empty() {
                return Err("--font must not be empty".into());
            }
            font_paths = FontPaths::all(value.to_owned());
            None
        } else if argument == "--font" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err("--font requires a value".into());
            };
            if value.is_empty() {
                return Err("--font must not be empty".into());
            }
            font_paths = FontPaths::all(value.clone());
            None
        } else if let Some((slot, value)) = font_argument(argument) {
            if value.is_empty() {
                return Err(format!("{slot} must not be empty"));
            }
            set_font_path(&mut font_paths, slot, value);
            None
        } else if matches!(
            argument.as_str(),
            "--font-regular"
                | "--font-italic"
                | "--font-bold"
                | "--font-semibold"
                | "--font-mono"
                | "--font-mono-bold"
                | "--font-mono-italic"
                | "--font-mono-semibold"
        ) {
            let flag = argument.clone();
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.is_empty() {
                return Err(format!("{flag} must not be empty"));
            }
            set_font_path(&mut font_paths, flag.trim_start_matches('-'), value);
            None
        } else if let Some(value) = argument.strip_prefix("--engine=") {
            engine = EngineKind::from_str(value).map_err(|error| error.to_string())?;
            engine_explicit = true;
            None
        } else if argument == "--engine" {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err("--engine requires a value".into());
            };
            engine = EngineKind::from_str(value).map_err(|error| error.to_string())?;
            engine_explicit = true;
            None
        } else if let Some(value) = argument.strip_prefix("--document-link-format=") {
            document_link_format = Some(ExportFormat::parse(value).map_err(|e| e.to_string())?);
            None
        } else if argument == "--document-link-extensionless" {
            document_link_extensionless = true;
            None
        } else if let Some(value) = argument.strip_prefix("--target=") {
            target = Some(value.to_owned());
            None
        } else if argument == "--target" {
            index += 1;
            target = Some(
                args.get(index)
                    .ok_or_else(|| "--target requires a value".to_owned())?
                    .clone(),
            );
            None
        } else if let Some(value) = argument.strip_prefix("--timeout=") {
            let seconds = value
                .parse::<i64>()
                .map_err(|error| format!("invalid timeout value {value:?}: {error}"))?;
            timeout = deadline::duration(seconds);
            None
        } else if argument == "--timeout" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--timeout requires a value".to_owned())?;
            let seconds = value
                .parse::<i64>()
                .map_err(|error| format!("invalid timeout value {value:?}: {error}"))?;
            timeout = deadline::duration(seconds);
            None
        } else {
            source_args.push(argument.clone());
            None
        };
        if let Some(value) = value {
            seeds = value
                .split(',')
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse::<i64>()
                        .map_err(|error| format!("invalid seeds value {value:?}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if seeds.is_empty() {
                return Err("seeds must contain at least one seed".into());
            }
        }
        index += 1;
    }
    let _deadline_scope = deadline::Scope::layout(timeout);
    let (path, utf16_positions) = parse_source_options(&source_args)?;
    let compiled = match reuse_compiled_source(input, &path, utf16_positions) {
        Some(compiled) => compiled,
        None => {
            let parsed = parse(
                &path,
                input,
                Some(ParseOptions {
                    utf16_positions,
                    errors: None,
                }),
            );
            if let Some(error) = parsed.error {
                return Err(error.to_string());
            }
            let (compiler, root) = compiler::compile_ir_with_options(
                Shared::new(parsed.map),
                compiler::CompileOptions {
                    import_opener: Some(filesystem_import_opener()),
                    utf16_positions,
                },
            )
            .map_err(|error| error.to_string())?;
            reject_compiler_errors(&compiler)?;
            validate_graph_compilation(&compiler, root)?;
            CompiledSource::new(input, &path, compiler, root)
        }
    };
    let (compiler, root) = (&compiled.compiler, compiled.root);
    let mut configured_theme_id = None;
    let mut configured_sketch = false;
    let mut tala_seed_data = None;
    if let Some(config) = compiler::extract_config_checked(compiler, root).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })? {
        configured_theme_id = config.theme_id;
        configured_sketch = config.sketch.unwrap_or(false);
        if !engine_explicit && let Some(layout_engine) = config.layout_engine.as_deref() {
            engine = EngineKind::from_str(layout_engine).map_err(|error| error.to_string())?;
        }
        if engine == EngineKind::Tala {
            tala_seed_data = config.tala_seeds().transpose();
        }
    }
    let effective_theme_id = requested_theme_id.or(configured_theme_id);
    let theme_rules = effective_theme_id
        .and_then(theme::palette)
        .map(|palette| palette.special_rules);
    let caps_lock = theme_rules.is_some_and(|rules| rules.caps_lock);
    let theme_mono = theme_rules.is_some_and(|rules| rules.mono);
    let measurer =
        load_graph_measurer_with_mode(&font_paths, requested_sketch || configured_sketch)?;
    let graph_root = match target.as_deref() {
        None | Some("") | Some("*") => root,
        Some(target) => {
            let target = target.strip_suffix(".*").unwrap_or(target);
            compiler::resolve_board_target(compiler, root, target)
                .map_err(|error| error.to_string())?
        }
    };
    if graph_root != root {
        // Go compiles the entire document before selecting a render target.
        // Validate its copy work even when only a small child board is drawn.
        let used = compiler.variable_expansion_usage().1;
        compiler::board_is_renderable(compiler, root).map_err(|error| error.to_string())?;
        compiler.restore_variable_expansion_used(used);
    }
    let (mut graph, labels, mut styles) = match measurer.as_ref() {
        Some(measurer) => compiler::lower_render_graph_with_measurer(
            compiler, graph_root, measurer, caps_lock, theme_mono,
        ),
        None => compiler::lower_render_graph_with_measurer(
            compiler,
            graph_root,
            &FallbackGraphMeasurer,
            caps_lock,
            theme_mono,
        ),
    }
    .map_err(|error| error.to_string())?;
    graph.target_theme_id = effective_theme_id;
    graph.tala_seed_data = tala_seed_data;
    let layout_options = d2_rs_tala::LayoutOptions {
        seeds,
        max_concurrency: 0,
    };
    let is_dagre = matches!(engine, EngineKind::Dagre);
    let resolver = layout_resolver::LayoutResolver::new(&engine_options, &layout_options);
    let resolved = resolver.resolve(engine.as_str())?;
    let active_cancellation = watch::active_cancellation_token();
    let result = resolved
        .layout_compiled(&mut graph, active_cancellation.as_ref())
        .map_err(|error| error.to_string())?;
    // d2cli.compile checks the graph returned by d2lib.Compile, which is
    // always the document root even when GetBoard selects a child to render.
    if graph_root == root {
        layout_features::check(engine.as_str(), &graph)?;
    } else {
        let root_graph =
            compiler::lower_graph(compiler, root).map_err(|error| error.to_string())?;
        layout_features::check(engine.as_str(), &root_graph)?;
    }
    // Dagre's adapter marks ordinary bent routes as cubic, while sequence
    // routes retain orthogonal polylines for D2's rounded-corner renderer.
    // TALA and ELK carry their own curve metadata through the graph.
    if is_dagre {
        for (edge_id, route) in &result.routes {
            let sequence_edge = graph
                .edges()
                .nth(edge_id.0 as usize)
                .is_some_and(|(_, edge)| edge_is_sequence_scoped(&graph, edge));
            if !sequence_edge {
                graph.set_edge_is_curve(*edge_id, route.len() >= 4);
            }
        }
    }
    if let Some(format) = document_link_format {
        let boards = compiler::board_targets(compiler, root);
        let current = boards
            .iter()
            .find(|(_, board)| *board == graph_root)
            .map(|(target, _)| target.as_str())
            .unwrap_or("");
        // main.go clears the selected diagram's children before resolveLinks
        // walks the root tree. Other branches still participate in relinking.
        let retain_children = target
            .as_deref()
            .is_none_or(|target| target == "*" || target.ends_with(".*"));
        let targets = boards
            .iter()
            .filter(|(candidate, _)| {
                retain_children
                    || !(current.is_empty()
                        || candidate
                            .strip_prefix(current)
                            .is_some_and(|suffix| suffix.starts_with('.')))
            })
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>();
        relink_document_links(&mut graph, &mut styles, |link| {
            relink_board_link(link, current, &targets, format, document_link_extensionless)
        });
    }
    Ok(SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata: source_target_hash_metadata_from_compiled(compiler, root, args),
    })
}

fn layout_source(input: &[u8], args: &[String]) -> Result<Vec<u8>, String> {
    let (_, result, _, _) = source_layout(input, args)?;
    serde_json::to_vec(&result).map_err(|error| format!("could not encode layout: {error}"))
}

fn render_source(input: &[u8], args: &[String]) -> Result<Vec<u8>, String> {
    let (svg, failures) = render_source_partial(input, args)?;
    if failures.is_empty() {
        Ok(svg)
    } else {
        Err(failures
            .into_iter()
            .map(|(_, error)| error)
            .collect::<Vec<_>>()
            .join("; "))
    }
}

fn render_source_partial(
    input: &[u8],
    args: &[String],
) -> Result<(Vec<u8>, Vec<(String, String)>), String> {
    render_source_partial_with_board_targets(input, args, None)
}

fn render_source_partial_with_board_targets(
    input: &[u8],
    args: &[String],
    board_targets: Option<&[String]>,
) -> Result<(Vec<u8>, Vec<(String, String)>), String> {
    let _source_config = SourceConfigScope::enter(input);
    let bundle = bundle_requested(args)?;
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata,
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    let (target_root_label, include_config_in_hash) = target_metadata?;
    render_options.target_root_label = target_root_label;
    render_options.include_config_in_hash = include_config_in_hash;
    // Discover actual rendered images in SVG order, as imgbundler does.
    let mut d2svg_options: Option<d2_rs_render::RenderOptions> = None;
    let unbundled = if d2svg_requested() {
        let mut options = render_options.clone();
        options.d2svg_hash_input = d2svg_subtree_hash_input(input, args, board_targets, || {
            d2_rs_render::target_hash_input(
                &graph,
                &result,
                &labels,
                &styles,
                &render_options,
                sketch,
            )
        })?;
        d2svg_options = Some(options.clone());
        d2_rs_render::render_d2svg(&graph, &result, &labels, &styles, &options, sketch)?
    } else {
        d2_rs_render::render_svg_with_sketch_checked(
            &graph,
            &result,
            &labels,
            &styles,
            render_options.clone(),
            sketch,
        )
        .map_err(|error| error.to_string())?
    };
    // imgbundler resolves each unique reference once, in rendered order.
    let (mut svg, failures) = imgbundler_cli::bundle(&source_args, bundle, &unbundled);
    // d2cli renderSingle: appendix.Append after bundling, then a trailing
    // newline.
    if let Some(options) = d2svg_options.as_ref().filter(|o| o.force_appendix) {
        svg = d2_rs_render::d2svg_append_appendix(
            &graph, &result, &labels, &styles, options, sketch, &svg,
        )?;
    }
    if d2svg_requested() && svg.last().is_some_and(|b| *b != b'\n') {
        svg.push(b'\n');
    }
    Ok((svg, failures))
}

/// Rebuilds the compact target bytes used by the v0.9.0 Diagram.HashID. This
/// intentionally shares the same option hydration and source-layout boundary
/// as SVG rendering; the multiboard wrapper then appends each child board in
/// D2's root/layers/scenarios/steps order before hashing once.
/// One animation frame's target data: hash bytes, legacy canvas, and (for
/// the d2svg path) the board's target JSON with its hydrated options.
struct AnimationTarget {
    hash_input: Vec<u8>,
    canvas: d2_rs_export::SvgAnimationCanvas,
    target_json: Option<Vec<u8>>,
    render_options: d2_rs_render::RenderOptions,
    sketch: bool,
}

impl AnimationTarget {
    fn package_svg(
        &self,
        svg: Vec<u8>,
        interval: u64,
        salt: Option<&str>,
        source_args: &[String],
    ) -> Result<Vec<u8>, String> {
        if let Some(json) = &self.target_json {
            // d2cli.compile uses RenderMultiboard and d2animate.Wrap even
            // for a single board. Share the same target-based wrapper as
            // document animation so fonts, styles and keyframes match Go.
            let bundle = bundle_requested(source_args)?;
            return d2_rs_render::render_d2svg_animated_with(
                "",
                &[(String::new(), json.clone(), true)],
                &self.render_options,
                self.sketch,
                interval,
                &mut |svg| bundle_animation_board(source_args, bundle, svg),
            );
        }
        let hash = d2_rs_render::target_hash_id_from_input(&self.hash_input, salt);
        d2_rs_export::package_svg_boards_with_animation_canvas(
            &[svg],
            interval,
            Some(&hash),
            Some(self.canvas),
        )
        .map_err(|error| error.to_string())
    }
}

fn render_source_target_animation(
    input: &[u8],
    args: &[String],
) -> Result<AnimationTarget, String> {
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata,
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    let (target_root_label, include_config_in_hash) = target_metadata?;
    render_options.target_root_label = target_root_label;
    render_options.include_config_in_hash = include_config_in_hash;
    let bounds = d2_rs_render::target_diagram_bounds(
        &graph,
        &result,
        &labels,
        &styles,
        &render_options,
        sketch,
    );
    let canvas = d2_rs_export::SvgAnimationCanvas {
        bounds,
        pad: render_options.pad,
        scale: render_options.scale,
    };
    let target_json = if d2svg_requested() {
        Some(d2_rs_render::target_diagram_json(
            &graph,
            &result,
            &labels,
            &styles,
            &render_options,
            sketch,
        )?)
    } else {
        None
    };
    Ok(AnimationTarget {
        hash_input: d2_rs_render::target_hash_input(
            &graph,
            &result,
            &labels,
            &styles,
            &render_options,
            sketch,
        ),
        canvas,
        target_json,
        render_options,
        sketch,
    })
}

/// Diagram.HashID includes descendants unless d2cli.compile clears them for
/// an explicit single-board target (main.go:368–377,542–545). Internal document
/// traversal uses `*`/`board.*` to retain each board's complete hash subtree.
fn d2svg_subtree_hash_input(
    input: &[u8],
    args: &[String],
    board_targets: Option<&[String]>,
    own: impl FnOnce() -> Vec<u8>,
) -> Result<Option<Vec<u8>>, String> {
    // The target-diff tool requests the own-board bytes even for leaf SVGs.
    // Preserve that diagnostic side effect without constructing its arguments
    // in ordinary builds or serializing twice for recursive hashes.
    #[cfg(feature = "diagnostic-traces")]
    let own = diagnostic_hash_input(own, std::env::var_os("D2_RS_TRACE_TARGET_BYTES").is_some());
    let path = args
        .iter()
        .find_map(|a| a.strip_prefix("--path="))
        .unwrap_or("-");
    let target = args.iter().enumerate().rev().find_map(|(index, arg)| {
        arg.strip_prefix("--target=").or_else(|| {
            (arg == "--target")
                .then(|| args.get(index + 1).map(String::as_str))
                .flatten()
        })
    });
    let explicit_target = target.is_some();
    let target = match target {
        None | Some("*") => "",
        Some(target) => match target.strip_suffix(".*") {
            Some(target) => target,
            None => return Ok(None),
        },
    };
    // d2cli compiles the tree once before choosing/rendering its boards
    // (main.go:510–557); Diagram.Bytes hashes that existing tree. Reuse the
    // CLI's already validated traversal instead of lowering it again here.
    // Direct rendering callers still discover/validate their own tree.
    let prefix = if target.is_empty() {
        String::new()
    } else {
        format!("{target}.")
    };
    let fallback_targets;
    let board_targets = match board_targets {
        Some(targets) => targets,
        None => {
            let targets = with_subtree_hash_plan(input, path, |plan| {
                let SubtreeHashPlan {
                    compiled,
                    all_targets,
                } = plan;
                let (compiler, root) = compiled.as_ref().map_err(Clone::clone)?;
                // Without --target the caller has already lowered the root,
                // the only board a tree without children validates; lower the
                // whole tree only when there are descendants to hash.
                if !explicit_target
                    && !compiler::board_targets(compiler, *root)
                        .iter()
                        .any(|(name, _)| !name.is_empty() && name.starts_with(&prefix))
                {
                    return Ok(None);
                }
                all_targets
                    .get_or_insert_with(|| {
                        source_board_plan_from_compiled(compiler, *root, "")
                            .map(|(plan, _)| plan.all_targets)
                    })
                    .clone()
                    .map(Some)
            })?;
            let Some(targets) = targets else {
                return Ok(None);
            };
            fallback_targets = targets;
            &fallback_targets
        }
    };
    let descendants: Vec<&String> = board_targets
        .iter()
        .filter(|t| !t.is_empty() && t.as_str() != target && t.starts_with(&prefix))
        .collect();
    if descendants.is_empty() {
        return Ok(None);
    }
    let mut boards = vec![(String::new(), own())];
    let is_target_arg = |index: usize, arg: &String| {
        arg.starts_with("--target=")
            || arg.as_str() == "--target"
            || (index != 0 && args[index - 1] == "--target")
    };
    // The descendant's target takes the place of the first target argument,
    // so its arguments equal the ones its own board render used and the
    // traversal's retained layout is reused. Option parsing reads one target
    // value, so its position does not change the board.
    let target_position = args
        .iter()
        .enumerate()
        .filter(|(index, arg)| !is_target_arg(*index, arg))
        .count()
        .min(
            args.iter()
                .enumerate()
                .position(|(index, arg)| is_target_arg(index, arg))
                .unwrap_or(args.len()),
        );
    for descendant in descendants {
        let mut board_args: Vec<String> = args
            .iter()
            .enumerate()
            .filter(|(index, arg)| !is_target_arg(*index, arg))
            .map(|(_, arg)| arg.clone())
            .collect();
        // These are children of an unpruned target, not explicit single-board
        // CLI selections. Keep their descendants in the board-link map.
        board_args.insert(target_position, format!("--target={descendant}.*"));
        let frame = render_source_target_animation(input, &board_args)?;
        boards.push((descendant[prefix.len()..].to_owned(), frame.hash_input));
    }
    Ok(Some(d2_rs_render::recursive_target_hash_input(&boards)))
}

#[cfg(any(test, feature = "diagnostic-traces"))]
fn diagnostic_hash_input(
    own: impl FnOnce() -> Vec<u8>,
    requested: bool,
) -> impl FnOnce() -> Vec<u8> {
    let (cached, pending) = if requested {
        (Some(own()), None)
    } else {
        (None, Some(own))
    };
    move || cached.unwrap_or_else(|| pending.expect("unconsumed own-board hash")())
}

/// imgbundler.BundleWithResolver on a d2svg board, in rendered order, as
/// render_source_partial does.
fn bundle_animation_board(
    source_args: &[String],
    bundle: bool,
    svg: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let (svg, failures) = imgbundler_cli::bundle(source_args, bundle, &svg);
    if failures.is_empty() {
        Ok(svg)
    } else {
        Err(report_asset_failures(&failures))
    }
}

fn imgbundle_d2svg(source_args: &[String], bundle: bool, svg: Vec<u8>) -> Vec<u8> {
    imgbundler_cli::bundle(source_args, bundle, &svg).0
}

/// SVG output uses the byte-identical d2svg port (PLAN S6);
/// D2_RS_SVG_RENDERER=legacy selects the earlier native renderer.
fn d2svg_requested() -> bool {
    #[cfg(test)]
    if LEGACY_SVG_FOR_TEST.with(std::cell::Cell::get) {
        return false;
    }
    std::env::var("D2_RS_SVG_RENDERER").as_deref() != Ok("legacy")
}

#[cfg(test)]
thread_local! {
    /// Tests of the legacy SVG renderer's own contract pin it per thread
    /// (the environment variable is process-global under parallel tests).
    static LEGACY_SVG_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) struct LegacySvgGuard;

#[cfg(test)]
impl LegacySvgGuard {
    pub(crate) fn new() -> Self {
        LEGACY_SVG_FOR_TEST.with(|flag| flag.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for LegacySvgGuard {
    fn drop(&mut self) {
        LEGACY_SVG_FOR_TEST.with(|flag| flag.set(false));
    }
}

fn render_source_format(
    input: &[u8],
    args: &[String],
    format: ExportFormat,
) -> Result<Vec<u8>, String> {
    render_source_format_with_pixel_scale(input, args, format, None)
}

/// Renders one board for a paged document and keeps its semantic hyperlink
/// regions alongside the PNG. The ordinary format helper intentionally
/// returns bytes only; paged PDF/PPTX packaging needs the graph-backed link
/// metadata before that information is rasterized away.
fn render_source_png_with_links(
    input: &[u8],
    args: &[String],
    frame_budget: &mut d2_rs_export::PagedFrameBudget,
    page_fill: bool,
) -> Result<
    (
        Vec<u8>,
        Vec<d2_rs_export::DocumentLinkMetadata>,
        Option<[u8; 3]>,
    ),
    String,
> {
    render_source_paged_frame_with_links(
        input,
        args,
        frame_budget,
        page_fill,
        d2_rs_export::render_paged_png_rgba_at_scale,
    )
}

/// `render_source_png_with_links` for a PDF page: the frame's pixels stay
/// decoded for the page image instead of passing through a PNG.
fn render_source_pdf_page_with_links(
    input: &[u8],
    args: &[String],
    frame_budget: &mut d2_rs_export::PagedFrameBudget,
) -> Result<
    (
        d2_rs_export::PdfPage,
        Vec<d2_rs_export::DocumentLinkMetadata>,
        Option<[u8; 3]>,
    ),
    String,
> {
    render_source_paged_frame_with_links(
        input,
        args,
        frame_budget,
        true,
        d2_rs_export::render_paged_pdf_page_at_scale,
    )
}

/// Renders one paged frame with `render_frame` (see
/// `render_source_png_with_links`).
fn render_source_paged_frame_with_links<T>(
    input: &[u8],
    args: &[String],
    frame_budget: &mut d2_rs_export::PagedFrameBudget,
    page_fill: bool,
    render_frame: d2_rs_export::PagedFrameRenderer<T>,
) -> Result<(T, Vec<d2_rs_export::DocumentLinkMetadata>, Option<[u8; 3]>), String> {
    let _bundle = bundle_requested(args)?;
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata,
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    let (target_root_label, include_config_in_hash) = target_metadata?;
    render_options.target_root_label = target_root_label;
    render_options.include_config_in_hash = include_config_in_hash;
    // Documents are self-contained. Match the existing raster path by
    // resolving local and remote assets even when SVG bundling was explicitly disabled.
    raster_asset::load_into_options(&source_args, &graph, &styles, &mut render_options, None)?;
    // paged.go AddPDFPage resolves the page fill (GetFillRGB) only for PDF,
    // after the board rendered; PowerPoint slides never read the root fill.
    let fill = page_fill.then(|| d2_rs_export::document_page_fill(&graph, &render_options));
    // paged.go renders transparent frames for both PDF and PowerPoint.
    let png = render_frame(
        &graph,
        &result,
        &labels,
        &styles,
        render_options.clone(),
        sketch,
        2,
        frame_budget,
    )
    .map_err(|error| error.to_string())?;
    // Go builds and validates the scene before extracting paged link regions.
    let links = d2_rs_export::document_link_metadata(
        &graph,
        &result,
        &labels,
        &styles,
        render_options,
        sketch,
    )
    .map_err(|error| error.to_string())?;
    // AddPDFPage returns GetFillRGB's color error unwrapped.
    let fill = fill.transpose().map_err(|error| match error {
        d2_rs_export::ExportError::Pdf(message) => message,
        other => other.to_string(),
    })?;
    Ok((png, links, fill))
}

fn render_source_format_with_pixel_scale(
    input: &[u8],
    args: &[String],
    format: ExportFormat,
    png_pixel_scale: Option<u32>,
) -> Result<Vec<u8>, String> {
    render_source_format_with_preview(input, args, format, png_pixel_scale, None, false, false)
        .map(|(rendered, _)| rendered)
}

/// Renders a requested document format and, when watch mode needs it, the SVG
/// preview from the same compiled graph and layout result. Keeping both outputs
/// on this boundary avoids running expensive layout engines twice per watch
/// update.
// Source raster.go and paged.go wrap scene construction at the board boundary.
// Encoding and source/SVG errors keep their own stage context.
fn format_scene_error(format: ExportFormat, board: usize, error: String) -> String {
    if format == ExportFormat::Gif && error.starts_with("GIF ") && !error.starts_with("GIF board ")
    {
        return format!("GIF board {board}: {error}");
    }
    if matches!(format, ExportFormat::Pdf | ExportFormat::Pptx) && error.starts_with("d2raster: ") {
        return format!("paged board {board} frame: {error}");
    }
    if !error.starts_with("scene: ") {
        return error;
    }
    match format {
        ExportFormat::Gif => format!("GIF board {board} scene: {error}"),
        ExportFormat::Pdf | ExportFormat::Pptx => format!("paged board {board} scene: {error}"),
        _ => error,
    }
}

fn render_source_format_with_preview(
    input: &[u8],
    args: &[String],
    format: ExportFormat,
    png_pixel_scale: Option<u32>,
    gif_interval_ms: Option<u64>,
    want_preview: bool,
    preview_appendix: bool,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    render_source_format_with_pptx_properties(
        input,
        args,
        format,
        png_pixel_scale,
        gif_interval_ms,
        want_preview,
        preview_appendix,
        None,
    )
}

fn render_source_format_with_pptx_properties(
    input: &[u8],
    args: &[String],
    format: ExportFormat,
    png_pixel_scale: Option<u32>,
    gif_interval_ms: Option<u64>,
    want_preview: bool,
    preview_appendix: bool,
    pptx_properties: Option<&d2_rs_export::PptxDocumentProperties>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let bundle = bundle_requested(args)?;
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata,
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    let (target_root_label, include_config_in_hash) = target_metadata?;
    render_options.target_root_label = target_root_label;
    render_options.include_config_in_hash = include_config_in_hash;
    let mut preview_options = render_options.clone();
    if preview_appendix {
        preview_options.force_appendix = true;
    }
    // Local images always bundle; SVG's bundle switch controls remote images.
    // Other export scenes resolve all assets for native rendering.
    if !matches!(format, ExportFormat::Svg | ExportFormat::Ascii) {
        let max_decoded_pixels =
            (format == ExportFormat::Gif).then_some(raster_asset::GIF_MAX_DECODED_PIXELS);
        // raster.go renderGIFWithSession reports scene asset preflight
        // failures at the board boundary ("GIF board N scene: ...").
        raster_asset::load_into_options(
            &source_args,
            &graph,
            &styles,
            &mut render_options,
            max_decoded_pixels,
        )
        .map_err(|error| {
            if format == ExportFormat::Gif {
                format_scene_error(format, 0, error)
            } else {
                error
            }
        })?;
    } else if format == ExportFormat::Svg {
        render_options.asset_data = load_asset_data_with_policy(
            &source_args,
            &graph,
            &styles,
            bundle || format != ExportFormat::Svg,
        )?;
    }
    preview_options.asset_data = render_options.asset_data.clone();
    preview_options.unavailable_images = render_options.unavailable_images.clone();
    let rendered = match (format, gif_interval_ms) {
        (ExportFormat::Gif, Some(interval_ms)) => d2_rs_export::render_animated_gif_with_options(
            &graph,
            &result,
            &labels,
            &styles,
            render_options,
            sketch,
            interval_ms,
        )
        .map_err(|error| error.to_string()),
        _ => match format {
            // PLAN S6: opt into the byte-identical d2svg port while its
            // coverage grows; the legacy renderer stays the default.
            ExportFormat::Svg if d2svg_requested() => d2_rs_render::render_d2svg(
                &graph,
                &result,
                &labels,
                &styles,
                &render_options,
                sketch,
            )
            .map(|svg| imgbundle_d2svg(&source_args, bundle, svg)),
            ExportFormat::Svg => d2_rs_render::render_svg_with_sketch_checked(
                &graph,
                &result,
                &labels,
                &styles,
                render_options,
                sketch,
            )
            .map_err(|error| error.to_string()),
            ExportFormat::Png if png_pixel_scale.is_some() => d2_rs_export::render_png_at_scale(
                &graph,
                &result,
                &labels,
                &styles,
                render_options,
                sketch,
                png_pixel_scale.unwrap_or(2),
            )
            .map_err(|error| error.to_string()),
            ExportFormat::Pptx if pptx_properties.is_some() => {
                let include_nav = render_options
                    .target_root_label
                    .as_deref()
                    .is_some_and(|label| !label.is_empty());
                let links = d2_rs_export::document_link_metadata(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    render_options.clone(),
                    sketch,
                )
                .map_err(|error| error.to_string())?;
                let png = d2_rs_export::render_png_rgba_at_scale(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    render_options,
                    sketch,
                    2,
                )
                .map_err(|error| error.to_string())?;
                d2_rs_export::package_pptx_slides_with_document_properties(
                    &[png],
                    &[vec!["root".into()]],
                    &[vec!["root".into()]],
                    &[links],
                    include_nav,
                    pptx_properties.unwrap(),
                )
                .map_err(|error| error.to_string())
            }
            _ => d2_rs_export::export_with_sketch(
                &graph,
                &result,
                &labels,
                &styles,
                format,
                render_options,
                sketch,
            )
            .map_err(|error| error.to_string()),
        },
    }
    .map_err(|error| format_scene_error(format, 0, error))?;
    let rendered = if format == ExportFormat::Png {
        d2_rs_export::add_png_exif_to_encoder_output(rendered, command_line::VERSION.trim_end())
            .map_err(|error| format_scene_error(format, 0, error.to_string()))?
    } else {
        rendered
    };
    let preview = if want_preview {
        if matches!(format, ExportFormat::Svg | ExportFormat::Ascii) {
            Some(rendered.clone())
        } else {
            Some(
                d2_rs_render::render_svg_with_sketch_checked(
                    &graph,
                    &result,
                    &labels,
                    &styles,
                    preview_options,
                    sketch,
                )
                .map_err(|error| error.to_string())?,
            )
        }
    } else {
        None
    };
    Ok((rendered, preview))
}

fn render_source_animated_gif(
    input: &[u8],
    args: &[String],
    interval_ms: u64,
) -> Result<Vec<u8>, String> {
    let _bundle = bundle_requested(args)?;
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        ..
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    // GIF is a self-contained raster package, so local and remote assets must be
    // resolved even when the caller disabled SVG bundling.
    raster_asset::load_into_options(
        &source_args,
        &graph,
        &styles,
        &mut render_options,
        Some(raster_asset::GIF_MAX_DECODED_PIXELS),
    )
    .map_err(|error| format_scene_error(ExportFormat::Gif, 0, error))?;
    d2_rs_export::render_animated_gif_with_options(
        &graph,
        &result,
        &labels,
        &styles,
        render_options,
        sketch,
        interval_ms,
    )
    .map_err(|error| format_scene_error(ExportFormat::Gif, 0, error.to_string()))
}

/// d2cli/raster.go renderGIFWithSession's schedule: the frame and board
/// ceilings, and the running normalized canvas every board is charged on.
struct GifSchedule {
    total_boards: usize,
    total_frames: usize,
    max_width: u32,
    max_height: u32,
    fonts: d2_rs_export::GifFontResources,
}

impl GifSchedule {
    fn new(interval_ms: u64, total_boards: usize) -> Result<Self, String> {
        const GIF_MAX_FRAMES: usize = 1_800;
        let frames_per_board =
            d2_rs_export::gif_frames_per_board(interval_ms).map_err(|error| error.to_string())?;
        if frames_per_board > GIF_MAX_FRAMES {
            return Err(format!(
                "GIF requires {frames_per_board} frames per board, exceeding the {GIF_MAX_FRAMES}-frame limit"
            ));
        }
        // collectGIFBoards admits at most gifMaxFrames/framesPerBoard boards.
        if total_boards > GIF_MAX_FRAMES / frames_per_board {
            return Err(format!(
                "GIF board count exceeds the {GIF_MAX_FRAMES}-frame limit"
            ));
        }
        Ok(Self {
            total_boards,
            total_frames: total_boards * frames_per_board,
            max_width: 0,
            max_height: 0,
            fonts: d2_rs_export::GifFontResources::new().map_err(|error| error.to_string())?,
        })
    }
}

enum GifBoardFrames {
    Encoded(Vec<u8>),
    Samples(Vec<Vec<u8>>),
}

/// Encodes a single board incrementally; multiboard samples are packaged
/// after every board's canvas is known (d2cli/raster.go renderGIF).
fn render_source_gif_frames(
    input: &[u8],
    args: &[String],
    interval_ms: u64,
    schedule: &mut GifSchedule,
) -> Result<GifBoardFrames, String> {
    let _bundle = bundle_requested(args)?;
    let (source_args, mut render_options, center_explicit, sketch_explicit) =
        extract_render_options(args)?;
    render_options.theme_id = hydrate_render_theme(input, &source_args, render_options.theme_id)?;
    render_options.dark_theme_id =
        hydrate_render_dark_theme(input, &source_args, render_options.dark_theme_id)?;
    render_options.center =
        hydrate_render_center(input, &source_args, render_options.center, center_explicit)?;
    render_options.theme_overrides =
        hydrate_render_theme_overrides(input, &source_args, &render_options.theme_overrides)?;
    render_options.dark_theme_overrides = hydrate_render_dark_theme_overrides(
        input,
        &source_args,
        &render_options.dark_theme_overrides,
    )?;
    let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
    let SourceRenderLayout {
        layout: (graph, result, labels, styles),
        target_metadata,
    } = source_render_layout_with_theme(input, &source_args, render_options.theme_id, sketch)?;
    raster_asset::load_into_options(
        &source_args,
        &graph,
        &styles,
        &mut render_options,
        Some(raster_asset::GIF_MAX_DECODED_PIXELS),
    )?;
    if !d2_rs_export::has_animated_content(&graph, &styles) {
        let (target_root_label, include_config_in_hash) = target_metadata?;
        render_options.target_root_label = target_root_label;
        render_options.include_config_in_hash = include_config_in_hash;
    }
    // Scene, planned bounds, the cumulative normalized footprint, then the
    // board's frames, in renderGIFWithSession's order.
    let board = d2_rs_export::plan_gif_board_with_fonts(
        &graph,
        &result,
        &labels,
        &styles,
        &render_options,
        sketch,
        schedule.total_boards,
        schedule.total_frames,
        &schedule.fonts,
    )
    .map_err(|error| error.to_string())?;
    let (width, height) = board.size();
    schedule.max_width = schedule.max_width.max(width);
    schedule.max_height = schedule.max_height.max(height);
    d2_rs_export::validate_gif_normalized_footprint(
        schedule.max_width,
        schedule.max_height,
        schedule.total_frames,
    )
    .map_err(|error| error.to_string())?;
    if schedule.total_boards == 1 {
        return d2_rs_export::encode_planned_gif_board(
            board,
            d2_rs_export::has_animated_content(&graph, &styles),
            interval_ms,
        )
        .map(GifBoardFrames::Encoded)
        .map_err(|error| error.to_string());
    }
    d2_rs_export::render_planned_gif_board(
        board,
        &graph,
        &result,
        &labels,
        &styles,
        render_options,
        sketch,
        interval_ms,
    )
    .map(GifBoardFrames::Samples)
    .map_err(|error| error.to_string())
}

// d2cli/main.go writeWithStatus and util-go/xmain.State.AtomicWritePath.
// Successful publication replaces the destination with a private temporary
// file (including existing symlinks); callers retain the non-atomic fallback.
fn atomic_write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use ring::rand::SecureRandom as _;
    let mut nonce = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| std::io::Error::other("failed to generate temporary output name"))?;
    let temporary = path.with_file_name(format!(
        "tmp-{}-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::fd::IntoRawFd;
            // Go checks Close before Rename; dropping File alone loses it.
            if unsafe { libc::close(file.into_raw_fd()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        #[cfg(not(unix))]
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

// d2cli writeWithStatus and writePNGWithStatus publish private staged files.
// PDF and PPTX instead use os.Create, retaining an existing destination's mode.
fn write_document_file(path: &Path, bytes: &[u8], format: ExportFormat) -> std::io::Result<()> {
    if !matches!(format, ExportFormat::Pdf | ExportFormat::Pptx)
        && atomic_write_file(path, bytes).is_ok()
    {
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The non-atomic fallback uses 0644; os.Create uses 0666.
        if !matches!(format, ExportFormat::Pdf | ExportFormat::Pptx) {
            options.mode(0o644);
        }
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

// d2cli's text branch returns before SVG's MkdirAll and falls back from an
// atomic write to OpenFile(O_WRONLY|O_CREATE|O_TRUNC, 0644).
fn write_ascii_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if atomic_write_file(path, bytes).is_ok() {
        return Ok(());
    }
    let error_text = |operation: &str, error: std::io::Error| {
        let message = error.to_string();
        let message = message
            .split(" (os error ")
            .next()
            .unwrap_or(&message)
            .to_ascii_lowercase();
        format!("{operation} {}: {message}", path.display())
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut file = options
        .open(path)
        .map_err(|error| error_text("open", error))?;
    file.write_all(bytes)
        .map_err(|error| error_text("write", error))
}

fn write_stdout(output: &[u8]) -> Result<(), String> {
    io::stdout()
        .write_all(output)
        .map_err(|error| format!("could not write stdout: {error}"))
}

/// Apply the v0.9.0 document-level SVG metadata contract. The native target
/// intentionally returns a fragment so library callers can embed it; the CLI
/// adds the XML declaration and version marker that D2 writes to files.
fn finalize_svg_output(mut output: Vec<u8>, no_xml_tag: bool, omit_version: bool) -> Vec<u8> {
    // d2svg output already carries Go's XML tag and data-d2-version; strip
    // them the way d2svg.Render omits them (the version slot stays empty).
    const XML_TAG: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>";
    const D2SVG_VERSION: &[u8] = b"data-d2-version=\"v0.9.0\"";
    if output.starts_with(XML_TAG) && output[XML_TAG.len()..].starts_with(b"<svg xmlns=") {
        if omit_version {
            let head_end = output
                .iter()
                .skip(XML_TAG.len())
                .position(|b| *b == b'>')
                .map_or(0, |n| n + XML_TAG.len());
            if let Some(at) = output[..head_end]
                .windows(D2SVG_VERSION.len())
                .position(|w| w == D2SVG_VERSION)
            {
                output.drain(at..at + D2SVG_VERSION.len());
            }
        }
        if no_xml_tag {
            output.drain(..XML_TAG.len());
        }
        return output;
    }
    let root_has_version = output
        .iter()
        .position(|byte| *byte == b'>')
        .is_some_and(|end| {
            output[..end]
                .windows(b"d2Version=".len())
                .any(|window| window == b"d2Version=")
                || output[..end]
                    .windows(b"data-d2-version=".len())
                    .any(|window| window == b"data-d2-version=")
        });
    if !omit_version && output.starts_with(b"<svg") && !root_has_version {
        output.splice(4..4, b" d2Version=\"v0.9.0\"".iter().copied());
    }
    if !no_xml_tag && output.starts_with(b"<svg") {
        let mut with_tag = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>".to_vec();
        with_tag.extend_from_slice(&output);
        output = with_tag;
    }
    output
}

const SOURCE_COMMANDS: &[&str] = &[
    "parse",
    "format",
    "compile",
    "graph",
    "layout-source",
    "render-svg",
    "validate",
    "version",
];

fn read_path(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        read_stdin()
    } else {
        std::fs::read(path).map_err(|error| format!("could not read {path:?}: {error}"))
    }
}

fn source_path_args(path: &str) -> Vec<String> {
    if path == "-" {
        Vec::new()
    } else {
        vec![format!("--path={path}")]
    }
}

fn default_output_path(input: &str, format: ExportFormat) -> Option<PathBuf> {
    if input == "-" {
        return None;
    }
    let mut output = PathBuf::from(input);
    output.set_extension(format.extension());
    Some(output)
}

/// Targets here come from compiler::board_targets or validated --target.
fn compiled_board_components(target: &str) -> Vec<String> {
    d2_rs_language::parse_key(target.as_bytes())
        .expect("validated board target")
        .string_ida()
        .expect("concrete board target")
}

/// Return renderable ancestors in tree order, preserving their original key
/// spellings for IDs/labels while comparing decoded literal name components.
fn rendered_board_ancestors(target: &str, targets: &BTreeSet<String>) -> Vec<(String, String)> {
    if target.is_empty() {
        return Vec::new();
    }
    let components = compiled_board_components(target);
    let mut ancestors: Vec<_> = targets
        .iter()
        .filter_map(|candidate| {
            let parts = compiled_board_components(candidate);
            (!parts.is_empty() && components.starts_with(&parts)).then(|| {
                (
                    parts.len(),
                    candidate.clone(),
                    parts.last().unwrap().clone(),
                )
            })
        })
        .collect();
    ancestors.sort_by_key(|(len, _, _)| *len);
    ancestors
        .into_iter()
        .map(|(_, id, name)| (id, name))
        .collect()
}

// paged.go walks the selected diagram as "root". Its descendants retain
// collection/name components, but ancestors outside that diagram disappear.
fn paged_board_id(target: &str, selected: &str) -> String {
    let parts = if target.is_empty() {
        Vec::new()
    } else {
        compiled_board_components(target)
    };
    let base = if selected.is_empty() {
        Vec::new()
    } else {
        compiled_board_components(selected)
    };
    let mut id = vec!["root".to_owned()];
    id.extend_from_slice(&parts[base.len()..]);
    id.join(".")
}

fn board_output_path(
    target: Option<&str>,
    format: ExportFormat,
    board_targets: &[String],
) -> PathBuf {
    let Some(target) = target.filter(|target| !target.is_empty()) else {
        return PathBuf::from(format!("index.{}", format.extension()));
    };

    // D2 keeps a single child collection flat (`one.svg`) and introduces a
    // `layers/`, `scenarios/`, or `steps/` directory only when a board has
    // more than one collection kind. Reconstruct that path from the compiled
    // target list so nested output remains stable without a second board AST.
    let components = compiled_board_components(target);
    let mut output = PathBuf::new();
    let mut offset = 0;
    while offset + 1 < components.len() {
        let parent = &components[..offset];
        let mut wrappers = std::collections::BTreeSet::new();
        for candidate in board_targets {
            let candidate_parts = compiled_board_components(candidate);
            if candidate_parts.len() > offset && candidate_parts[..offset] == *parent {
                wrappers.insert(candidate_parts[offset].clone());
            }
        }
        if wrappers.len() > 1 {
            output.push(&components[offset]);
        }
        output = board_output::contained_board_output_component(
            &output,
            &board_output::board_output_component(&components[offset + 1]),
        )
        .expect("portable board output component stays contained");
        offset += 2;
    }
    if board_targets.iter().any(|candidate| {
        let parts = compiled_board_components(candidate);
        parts.len() > components.len() && parts.starts_with(&components)
    }) {
        output.push("index");
    }
    // A dot in the literal board name is not an existing file extension.
    output
        .as_mut_os_string()
        .push(format!(".{}", format.extension()));
    output
}

/// d2cli/main.go resolveLinks/relink: board links resolve against the
/// complete document output tree, relative to the current board.
fn relink_document_links(
    graph: &mut Graph,
    styles: &mut BTreeMap<NodeId, compiler::SourceNodeStyle>,
    resolve: impl Fn(&str) -> Option<String>,
) {
    for style in styles.values_mut() {
        if let Some(link) = style.link.as_deref().and_then(&resolve) {
            style.link = Some(link);
        }
    }
    let edge_ids = graph.edges().map(|(id, _)| id).collect::<Vec<_>>();
    for edge_id in edge_ids {
        if let Some(link) = graph.edge_link(edge_id).and_then(&resolve) {
            graph.set_edge_link(edge_id, Some(link));
        }
    }
}

fn relink_board_link(
    link: &str,
    current: &str,
    targets: &[String],
    format: ExportFormat,
    extensionless: bool,
) -> Option<String> {
    let mut current_path = board_output_path(Some(current), format, targets);
    if extensionless {
        current_path.set_extension("");
    }
    let key = d2_rs_language::parse_key(link.as_bytes()).ok()?;
    let parts = key.string_ida()?;
    if parts.first().map(String::as_str) != Some("root") {
        return None;
    }
    let target = if parts.len() == 1 {
        ""
    } else {
        targets
            .iter()
            .find(|t| compiled_board_components(t) == parts[1..])?
            .as_str()
    };
    let mut destination = board_output_path(Some(target), format, targets);
    if extensionless {
        destination.set_extension("");
    }
    Some(relative_board_output_link(&current_path, &destination))
}

/// d2cli/main.go relink: paths are relative to the current output's directory.
fn relative_board_output_link(current: &Path, destination: &Path) -> String {
    let parent = current
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect::<Vec<_>>();
    let destination = destination.components().collect::<Vec<_>>();
    let common = parent
        .iter()
        .zip(&destination)
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative = Vec::new();
    for _ in common..parent.len() {
        relative.push("..".to_owned());
    }
    for component in &destination[common..] {
        relative.push(component.as_os_str().to_string_lossy().into_owned());
    }
    board_output_link(&relative.join("/"))
}

/// d2cli/main.go boardOutputLink (Unix): preserve dot segments, escape others.
fn board_output_link(path: &str) -> String {
    path.split('/')
        .map(url_path_escape)
        .collect::<Vec<_>>()
        .join("/")
}

/// net/url.PathEscape (encodePathSegment).
fn url_path_escape(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'~' | b'$' | b'&' | b'+' | b',' | b';' | b'=' | b':' | b'@'
            )
        {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// Recursive render starts at the selected board's name, even when navigation
// selected that board through a longer path in the complete document.
fn board_subtree_output_path(
    target: &str,
    selected: &str,
    format: ExportFormat,
    targets: &[String],
) -> PathBuf {
    if selected.is_empty() {
        return board_output_path(Some(target), format, targets);
    }
    let name = board_output::board_output_component(
        compiled_board_components(selected)
            .pop()
            .expect("named board"),
    );
    let prefix = format!("{selected}.");
    let children = targets
        .iter()
        .filter_map(|child| child.strip_prefix(&prefix).map(str::to_owned))
        .collect::<Vec<_>>();
    if children.is_empty() {
        return PathBuf::from(format!("{name}.{}", format.extension()));
    }
    PathBuf::from(name).join(board_output_path(
        target.strip_prefix(&prefix),
        format,
        &children,
    ))
}

#[allow(clippy::too_many_arguments)]
fn run_multiboard_document(
    input_path: &str,
    input: &[u8],
    output_format: ExportFormat,
    layout: &str,
    seeds: Option<&str>,
    pad: Option<f64>,
    scale: Option<f64>,
    theme_id: Option<i64>,
    dark_theme_id: Option<i64>,
    center: Option<bool>,
    sketch: Option<bool>,
    bundle: bool,
    force_appendix: bool,
    salt: Option<&str>,
    animate_interval: Option<u64>,
    timeout: Option<i64>,
    source_adapter_args: &[String],
    output_path: Option<&str>,
    board_targets: &[String],
    hash_targets: &[String],
    animation_root_target: &str,
    postorder_targets: &[String],
    root_renderable: bool,
    renderable_targets: &std::collections::BTreeSet<String>,
    ascii_mode: &str,
    no_xml_tag: bool,
    omit_version: bool,
    preview_target: Option<&str>,
    recursive_target: Option<&str>,
    output_written: &mut bool,
) -> Result<Vec<u8>, String> {
    let started = std::time::Instant::now();
    let svg_animation = output_format == ExportFormat::Svg && animate_interval.is_some();
    if svg_animation {
        validate_animation_destination(
            input_path,
            output_path,
            animation_root_target,
            hash_targets,
        )?;
    } else if (output_path == Some("-") || (output_path.is_none() && input_path == "-"))
        && recursive_target.is_none_or(str::is_empty)
        && !matches!(
            output_format,
            ExportFormat::Pdf | ExportFormat::Pptx | ExportFormat::Gif
        )
    {
        return Err("multiboard output cannot be written to stdout".into());
    }
    let requested_output = output_path
        .map(PathBuf::from)
        .or_else(|| default_output_path(input_path, output_format))
        .or_else(|| (svg_animation && input_path == "-").then(|| PathBuf::from("-")))
        .or_else(|| recursive_target.is_some().then(|| PathBuf::from("-")))
        .or_else(|| {
            matches!(
                output_format,
                ExportFormat::Pdf | ExportFormat::Pptx | ExportFormat::Gif
            )
            .then(|| PathBuf::from("-"))
        })
        .ok_or_else(|| "multiple boards require a file output directory".to_owned())?;
    if output_format == ExportFormat::Pptx {
        let mut slides = Vec::with_capacity(board_targets.len() + 1);
        let mut slide_links = Vec::with_capacity(board_targets.len() + 1);
        let mut targets = Vec::with_capacity(board_targets.len() + usize::from(root_renderable));
        if root_renderable {
            targets.push(animation_root_target.to_owned());
        }
        targets.extend(board_targets.iter().cloned());
        let mut title_targets = targets.clone();
        if !animation_root_target.is_empty() {
            title_targets.push(animation_root_target.to_owned());
        }
        let mut source_titles = source_board_titles(input, input_path, &title_targets)?;
        if !animation_root_target.is_empty() {
            source_titles.root_label = source_titles
                .board_labels
                .get(animation_root_target)
                .cloned()
                .flatten()
                // d2exporter/export.go:25–28 uses the board name only
                // when no label was declared, preserving explicit empties.
                .or_else(|| compiled_board_components(animation_root_target).pop())
                .filter(|label| !label.is_empty());
        }
        let board_paths = targets
            .iter()
            .map(|target| {
                let mut path = if root_renderable {
                    vec!["root".to_owned()]
                } else {
                    Vec::new()
                };
                path.extend(
                    rendered_board_ancestors(target, renderable_targets)
                        .into_iter()
                        .filter(|(id, _)| id != animation_root_target)
                        .map(|(id, _)| paged_board_id(&id, animation_root_target)),
                );

                path
            })
            .collect::<Vec<_>>();
        let title_paths = targets
            .iter()
            .map(|target| {
                if target == animation_root_target {
                    return vec!["root".to_owned()];
                }
                let mut names = if root_renderable {
                    vec!["root".to_owned()]
                } else {
                    Vec::new()
                };
                names.extend(
                    rendered_board_ancestors(target, renderable_targets)
                        .into_iter()
                        .filter(|(id, _)| id != animation_root_target)
                        .map(|(_, name)| name),
                );
                names
            })
            .collect::<Vec<_>>();
        let mut frame_budget = d2_rs_export::PagedFrameBudget::new(targets.len());
        for target in targets {
            let mut render_args = source_path_args(input_path);
            render_args.push(format!("--ascii-mode={ascii_mode}"));
            render_args.push(format!("--engine={layout}"));
            if let Some(seeds) = seeds {
                render_args.push(format!("--seeds={seeds}"));
            }
            if let Some(pad) = pad {
                render_args.push(format!("--pad={pad}"));
            }
            if let Some(scale) = scale {
                render_args.push(format!("--scale={scale}"));
            }
            if let Some(salt) = salt {
                render_args.push(format!("--salt={salt}"));
            }
            render_args.push(format!("--bundle={bundle}"));
            render_args.push(format!("--force-appendix={force_appendix}"));
            if let Some(theme_id) = theme_id {
                render_args.push(format!("--theme={theme_id}"));
            }
            if let Some(dark_theme_id) = dark_theme_id {
                render_args.push(format!("--dark-theme={dark_theme_id}"));
            }
            if let Some(center) = center {
                render_args.push(format!("--center={center}"));
            }
            if sketch == Some(true) {
                render_args.push("--sketch".into());
            }
            if let Some(timeout) = timeout {
                render_args.push(format!("--timeout={timeout}"));
            }
            render_args.extend(source_adapter_args.iter().cloned());
            render_args.push(format!("--target={target}"));
            let (slide, links, _) =
                render_source_png_with_links(input, &render_args, &mut frame_budget, false)
                    .map_err(|error| format_scene_error(ExportFormat::Pptx, slides.len(), error))?;
            slides.push(slide);
            slide_links.push(links);
        }
        let properties =
            pptx_document_properties(input_path, Some(&requested_output.to_string_lossy()));
        let packaged = d2_rs_export::package_pptx_slides_with_document_properties(
            &slides,
            &board_paths,
            &title_paths,
            &slide_links,
            source_titles.root_label.is_some(),
            &properties,
        )
        .map_err(|error| match error {
            d2_rs_export::ExportError::Pdf(message) | d2_rs_export::ExportError::Pptx(message) => {
                message
            }
            other => other.to_string(),
        })?;
        // d2cli compile logs success for these exports, stdout included.
        if requested_output.as_os_str() == "-" {
            log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
            return Ok(packaged);
        }
        if let Some(parent) = requested_output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        write_document_file(&requested_output, &packaged, output_format)
            .map_err(|error| format!("could not write {}: {error}", requested_output.display()))?;
        log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
        return Ok(Vec::new());
    }
    if output_format == ExportFormat::Pdf {
        let mut page_fills = Vec::new();
        let mut pages = Vec::with_capacity(board_targets.len() + 1);
        let mut page_links = Vec::with_capacity(board_targets.len() + 1);
        let mut targets = Vec::with_capacity(board_targets.len() + usize::from(root_renderable));
        if root_renderable {
            targets.push(animation_root_target.to_owned());
        }
        targets.extend(board_targets.iter().cloned());
        let mut title_targets = targets.clone();
        if !animation_root_target.is_empty() {
            title_targets.push(animation_root_target.to_owned());
        }
        let mut source_titles = source_board_titles(input, input_path, &title_targets)?;
        if !animation_root_target.is_empty() {
            source_titles.root_label = source_titles
                .board_labels
                .get(animation_root_target)
                .cloned()
                .flatten()
                // d2exporter/export.go:25–28 uses the board name only
                // when no label was declared, preserving explicit empties.
                .or_else(|| compiled_board_components(animation_root_target).pop())
                .filter(|label| !label.is_empty());
        }
        let board_paths = targets
            .iter()
            .map(|target| {
                let mut path = if root_renderable {
                    vec!["root".to_owned()]
                } else {
                    Vec::new()
                };
                path.extend(
                    rendered_board_ancestors(target, renderable_targets)
                        .into_iter()
                        .filter(|(id, _)| id != animation_root_target)
                        .map(|(id, _)| paged_board_id(&id, animation_root_target)),
                );

                path
            })
            .collect::<Vec<_>>();
        let title_paths = targets
            .iter()
            .map(|target| {
                if target == animation_root_target {
                    return if root_renderable {
                        vec![
                            source_titles
                                .root_label
                                .clone()
                                .unwrap_or_else(|| "root".to_owned()),
                        ]
                    } else {
                        Vec::new()
                    };
                }
                let mut names = if root_renderable {
                    vec![
                        source_titles
                            .root_label
                            .clone()
                            .unwrap_or_else(|| "root".to_owned()),
                    ]
                } else {
                    Vec::new()
                };
                for (prefix, board_id) in rendered_board_ancestors(target, renderable_targets) {
                    if prefix == animation_root_target {
                        continue;
                    }
                    let label = source_titles
                        .board_labels
                        .get(&prefix)
                        .and_then(Clone::clone)
                        .unwrap_or(board_id);
                    names.push(label);
                }

                names
            })
            .collect::<Vec<_>>();
        let mut frame_budget = d2_rs_export::PagedFrameBudget::new(targets.len());
        for target in targets {
            let mut render_args = source_path_args(input_path);
            render_args.push(format!("--ascii-mode={ascii_mode}"));
            render_args.push(format!("--engine={layout}"));
            if let Some(seeds) = seeds {
                render_args.push(format!("--seeds={seeds}"));
            }
            if let Some(pad) = pad {
                render_args.push(format!("--pad={pad}"));
            }
            if let Some(scale) = scale {
                render_args.push(format!("--scale={scale}"));
            }
            if let Some(salt) = salt {
                render_args.push(format!("--salt={salt}"));
            }
            render_args.push(format!("--bundle={bundle}"));
            render_args.push(format!("--force-appendix={force_appendix}"));
            if let Some(theme_id) = theme_id {
                render_args.push(format!("--theme={theme_id}"));
            }
            if let Some(dark_theme_id) = dark_theme_id {
                render_args.push(format!("--dark-theme={dark_theme_id}"));
            }
            if let Some(center) = center {
                render_args.push(format!("--center={center}"));
            }
            if sketch == Some(true) {
                render_args.push("--sketch".into());
            }
            if let Some(timeout) = timeout {
                render_args.push(format!("--timeout={timeout}"));
            }
            render_args.extend(source_adapter_args.iter().cloned());
            render_args.push(format!("--target={target}"));
            let (page, links, fill) =
                render_source_pdf_page_with_links(input, &render_args, &mut frame_budget)
                    .map_err(|error| format_scene_error(ExportFormat::Pdf, pages.len(), error))?;
            pages.push(page);
            page_fills.push(fill.unwrap_or([255; 3]));
            page_links.push(links);
        }
        let packaged = d2_rs_export::package_pdf_page_rasters_with_navigation_metadata_for_theme(
            pages,
            &page_links,
            &board_paths,
            &title_paths,
            source_titles.root_label.is_some(),
            theme_id,
            &page_fills,
        )
        .map_err(|error| match error {
            d2_rs_export::ExportError::Pdf(message) | d2_rs_export::ExportError::Pptx(message) => {
                message
            }
            other => other.to_string(),
        })?;
        // d2cli compile logs success for these exports, stdout included.
        if requested_output.as_os_str() == "-" {
            log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
            return Ok(packaged);
        }
        if let Some(parent) = requested_output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        write_document_file(&requested_output, &packaged, output_format)
            .map_err(|error| format!("could not write {}: {error}", requested_output.display()))?;
        log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
        return Ok(Vec::new());
    }
    if output_format == ExportFormat::Svg && animate_interval.is_some() {
        let mut svg_boards = Vec::with_capacity(board_targets.len() + usize::from(root_renderable));
        let mut target_hash_inputs = Vec::new();
        let mut animation_canvas: Option<d2_rs_export::SvgAnimationCanvas> = None;
        let mut d2svg_frames: Vec<(String, Vec<u8>, bool)> = Vec::new();
        let mut d2svg_options: Option<(d2_rs_render::RenderOptions, bool)> = None;
        // HashID always includes the document root, even when it is a
        // folder-only board that does not produce an animation frame.
        let mut targets = Vec::with_capacity(hash_targets.len() + 1);
        targets.push(animation_root_target.to_owned());
        // HashID includes folder-only boards even when they do not produce an
        // animation frame; their serialized bytes carry nested descendants.
        targets.extend(hash_targets.iter().cloned());
        for target in targets {
            let mut render_args = source_path_args(input_path);
            render_args.push(format!("--ascii-mode={ascii_mode}"));
            render_args.push(format!("--engine={layout}"));
            if let Some(seeds) = seeds {
                render_args.push(format!("--seeds={seeds}"));
            }
            if let Some(pad) = pad {
                render_args.push(format!("--pad={pad}"));
            }
            if let Some(scale) = scale {
                render_args.push(format!("--scale={scale}"));
            }
            if let Some(salt) = salt {
                render_args.push(format!("--salt={salt}"));
            }
            render_args.push(format!("--bundle={bundle}"));
            render_args.push(format!("--force-appendix={force_appendix}"));
            if let Some(theme_id) = theme_id {
                render_args.push(format!("--theme={theme_id}"));
            }
            if let Some(dark_theme_id) = dark_theme_id {
                render_args.push(format!("--dark-theme={dark_theme_id}"));
            }
            if let Some(center) = center {
                render_args.push(format!("--center={center}"));
            }
            if sketch == Some(true) {
                render_args.push("--sketch".into());
            }
            if let Some(timeout) = timeout {
                render_args.push(format!("--timeout={timeout}"));
            }
            render_args.extend(source_adapter_args.iter().cloned());
            render_args.push(format!("--target={target}"));
            let frame = render_source_target_animation(input, &render_args)?;
            let (hash_input, mut canvas) = (frame.hash_input, frame.canvas);
            if let Some(json) = frame.target_json {
                let renderable = !((target.is_empty() && !root_renderable)
                    || (!target.is_empty() && !renderable_targets.contains(&target)));
                d2svg_frames.push((target.clone(), json, renderable));
                d2svg_options.get_or_insert((frame.render_options, frame.sketch));
            }
            if let Some(previous) = animation_canvas {
                for axis in 0..2 {
                    canvas.bounds[axis] = canvas.bounds[axis].min(previous.bounds[axis]);
                    canvas.bounds[axis + 2] =
                        canvas.bounds[axis + 2].max(previous.bounds[axis + 2]);
                }
            }
            animation_canvas = Some(canvas);
            target_hash_inputs.push((target.clone(), hash_input));
            if (target.is_empty() && !root_renderable)
                || (!target.is_empty() && !renderable_targets.contains(&target))
            {
                continue;
            }
            if !d2svg_requested() {
                svg_boards.push(render_source_format(
                    input,
                    &render_args,
                    ExportFormat::Svg,
                )?);
            }
        }
        if let Some((options, sketch)) = d2svg_options {
            let packaged = d2_rs_render::render_d2svg_animated_with(
                animation_root_target,
                &d2svg_frames,
                &options,
                sketch,
                animate_interval.unwrap_or(1000),
                &mut |svg| bundle_animation_board(&source_path_args(input_path), bundle, svg),
            )?;
            // d2cli.renderToPaths bundles each rendered frame before
            // d2animate.Wrap and propagates any resolver error. The native
            // animation renderer retains image references through wrapping.
            let (packaged, failures) =
                imgbundler_cli::bundle(&source_path_args(input_path), bundle, &packaged);
            if !failures.is_empty() {
                return Err(report_asset_failures(&failures));
            }
            let packaged = finalize_svg_output(packaged, no_xml_tag, omit_version);
            if requested_output.as_os_str() == "-" {
                log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
                return Ok(packaged);
            }
            if let Some(parent) = requested_output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            write_document_file(&requested_output, &packaged, output_format).map_err(|error| {
                format!("could not write {}: {error}", requested_output.display())
            })?;
            log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
            return Ok(Vec::new());
        }
        let target_hash_input = d2_rs_render::recursive_target_hash_input(&target_hash_inputs);
        let diagram_hash = d2_rs_render::target_hash_id_from_input(&target_hash_input, salt);
        let packaged = d2_rs_export::package_svg_boards_with_animation_canvas(
            &svg_boards,
            animate_interval.unwrap_or(1000),
            Some(&diagram_hash),
            animation_canvas,
        )
        .map_err(|error| error.to_string())?;
        let packaged = finalize_svg_output(packaged, no_xml_tag, omit_version);
        if let Some(parent) = requested_output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        if requested_output.as_os_str() == "-" {
            log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
            return Ok(packaged);
        }
        write_document_file(&requested_output, &packaged, output_format)
            .map_err(|error| format!("could not write {}: {error}", requested_output.display()))?;
        log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
        return Ok(Vec::new());
    }
    if output_format == ExportFormat::Gif {
        let mut board_frames = Vec::with_capacity(board_targets.len() + 1);
        let mut encoded = None;
        let mut targets = Vec::with_capacity(board_targets.len() + usize::from(root_renderable));
        if root_renderable {
            targets.push(animation_root_target.to_owned());
        }
        targets.extend(board_targets.iter().cloned());
        let mut schedule = GifSchedule::new(animate_interval.unwrap_or(1000), targets.len())?;
        for target in targets {
            let mut render_args = source_path_args(input_path);
            render_args.push(format!("--ascii-mode={ascii_mode}"));
            render_args.push(format!("--engine={layout}"));
            if let Some(seeds) = seeds {
                render_args.push(format!("--seeds={seeds}"));
            }
            if let Some(pad) = pad {
                render_args.push(format!("--pad={pad}"));
            }
            if let Some(scale) = scale {
                render_args.push(format!("--scale={scale}"));
            }
            if let Some(salt) = salt {
                render_args.push(format!("--salt={salt}"));
            }
            render_args.push(format!("--bundle={bundle}"));
            render_args.push(format!("--force-appendix={force_appendix}"));
            if let Some(theme_id) = theme_id {
                render_args.push(format!("--theme={theme_id}"));
            }
            if let Some(dark_theme_id) = dark_theme_id {
                render_args.push(format!("--dark-theme={dark_theme_id}"));
            }
            if let Some(center) = center {
                render_args.push(format!("--center={center}"));
            }
            if sketch == Some(true) {
                render_args.push("--sketch".into());
            }
            if let Some(timeout) = timeout {
                render_args.push(format!("--timeout={timeout}"));
            }
            render_args.extend(source_adapter_args.iter().cloned());
            render_args.push(format!("--target={target}"));
            let samples = render_source_gif_frames(
                input,
                &render_args,
                animate_interval.unwrap_or(1000),
                &mut schedule,
            )
            .map_err(|error| format_scene_error(ExportFormat::Gif, board_frames.len(), error))?;
            match samples {
                GifBoardFrames::Encoded(bytes) => encoded = Some(bytes),
                GifBoardFrames::Samples(samples) => board_frames.push(samples),
            }
        }
        let packaged = encoded
            .map_or_else(
                || {
                    d2_rs_export::package_gif_sampled_boards_with_interval(
                        &board_frames,
                        animate_interval.unwrap_or(1000),
                    )
                },
                Ok,
            )
            .map_err(|error| match error {
                d2_rs_export::ExportError::Pdf(message)
                | d2_rs_export::ExportError::Pptx(message) => message,
                other => other.to_string(),
            })?;
        // GIF is a single composed artifact, so unlike recursive SVG/PNG it
        // can be written to stdout (d2cli/main.go's compile GIF branch).
        // d2cli compile logs success for these exports, stdout included.
        if requested_output.as_os_str() == "-" {
            log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
            return Ok(packaged);
        }
        if let Some(parent) = requested_output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        write_document_file(&requested_output, &packaged, output_format)
            .map_err(|error| format!("could not write {}: {error}", requested_output.display()))?;
        log_compile_success(Path::new(input_path), &requested_output, started.elapsed());
        return Ok(Vec::new());
    }
    let extensionless = requested_output == Path::new("-");
    let board_render_args = |target: &str| {
        let mut render_args = source_path_args(input_path);
        render_args.push(format!("--ascii-mode={ascii_mode}"));
        render_args.push(format!("--engine={layout}"));
        if let Some(seeds) = seeds {
            render_args.push(format!("--seeds={seeds}"));
        }
        if let Some(pad) = pad {
            render_args.push(format!("--pad={pad}"));
        }
        if let Some(scale) = scale {
            render_args.push(format!("--scale={scale}"));
        }
        if let Some(salt) = salt {
            render_args.push(format!("--salt={salt}"));
        }
        render_args.push(format!("--bundle={bundle}"));
        render_args.push(format!("--force-appendix={force_appendix}"));
        if let Some(theme_id) = theme_id {
            render_args.push(format!("--theme={theme_id}"));
        }
        if let Some(dark_theme_id) = dark_theme_id {
            render_args.push(format!("--dark-theme={dark_theme_id}"));
        }
        if let Some(center) = center {
            render_args.push(format!("--center={center}"));
        }
        if sketch == Some(true) {
            render_args.push("--sketch".into());
        }
        if let Some(timeout) = timeout {
            render_args.push(format!("--timeout={timeout}"));
        }
        render_args.extend(source_adapter_args.iter().cloned());
        render_args.push(if target.is_empty() {
            "--target=*".into()
        } else {
            format!("--target={target}.*")
        });
        if output_format == ExportFormat::Svg {
            render_args.push("--document-link-format=svg".into());
            if extensionless {
                render_args.push("--document-link-extensionless".into());
            }
        }
        render_args
    };
    let _board_layouts = BoardLayoutScope::enter();
    // Compile preorder, then render postorder, matching d2lib.compileBoard
    // and d2cli.renderWithPNGEncoder. In particular a bad parent must fail
    // before a valid child can render, load assets or report success.
    for target in std::iter::once("")
        .filter(|_| root_renderable)
        .chain(hash_targets.iter().map(String::as_str))
    {
        if !renderable_targets.contains(target) && !(target.is_empty() && root_renderable) {
            continue;
        }
        let args = board_render_args(target);
        let (source_args, options, _, sketch_explicit) = extract_render_options(&args)?;
        let theme = hydrate_render_theme(input, &source_args, options.theme_id)?;
        let sketch = hydrate_render_sketch(input, &source_args, sketch_explicit)?;
        source_render_layout_with_theme(input, &source_args, theme, sketch)?.target_metadata?;
    }
    let output_dir = requested_output.with_extension("");
    // main.go skips the workspace for stdout. planBoardOutput then appends a
    // named recursive target before testing for stdout (board_output.go:159),
    // so these boards are ordinary extensionless files below the literal '-'.
    let selected_target = preview_target.or(recursive_target);
    // A preview navigation of a leaf board recompiles only that board;
    // sibling outputs written by earlier visits remain.
    let renders_child_boards = selected_target.is_none_or(|target| {
        target.is_empty()
            || hash_targets
                .iter()
                .any(|child| child.starts_with(&format!("{target}.")))
    });
    // d2cli/main.go stages a multiboard render and publishes it as one
    // transaction (board_output.go) whenever the rendered (sub)tree has child
    // boards; a watch preview of a leaf board is written in place.
    let workspace = if renders_child_boards && !extensionless {
        let selected = selected_target.unwrap_or("");
        let outputs = postorder_targets
            .iter()
            .filter(|target| selected.is_empty() || target.starts_with(&format!("{selected}.")))
            .cloned()
            .chain(
                (if selected.is_empty() {
                    root_renderable
                } else {
                    renderable_targets.contains(selected)
                })
                .then(|| selected.to_owned()),
            )
            .map(|target| {
                let path = output_dir.join(board_subtree_output_path(
                    &target,
                    selected,
                    output_format,
                    hash_targets,
                ));
                let board = if target.is_empty() {
                    "root".to_owned()
                } else {
                    format!("root.{target}")
                };
                (board, path)
            })
            .collect::<Vec<_>>();
        board_output::validate_board_output_paths(&outputs)?;
        Some(board_output::BoardOutputWorkspace::new(&output_dir)?)
    } else {
        None
    };
    let write_dir = workspace.as_ref().map_or(output_dir.clone(), |workspace| {
        workspace.stage_root().to_path_buf()
    });
    if output_format != ExportFormat::Ascii && workspace.is_none() {
        std::fs::create_dir_all(&output_dir)
            .map_err(|error| format!("could not create {}: {error}", output_dir.display()))?;
    }
    let mut targets = Vec::with_capacity(board_targets.len() + usize::from(root_renderable));
    targets.extend(
        postorder_targets
            .iter()
            .filter(|target| {
                selected_target.is_none_or(|selected| {
                    selected.is_empty() || target.starts_with(&format!("{selected}."))
                })
            })
            .cloned(),
    );
    // Upstream renderWithPNGEncoder visits child boards before the root.
    let selected = selected_target.unwrap_or("");
    if if selected.is_empty() {
        root_renderable
    } else {
        renderable_targets.contains(selected)
    } {
        targets.push(selected.to_owned());
    }
    let preview_board = if targets.iter().any(|target| target == selected) {
        Some(selected.to_owned())
    } else {
        board_targets
            .iter()
            .find(|target| targets.contains(target))
            .cloned()
    };
    let mut preview = Vec::new();
    for target in targets {
        let render_started = std::time::Instant::now();
        let render_args = board_render_args(&target);
        let (rendered, asset_failures) = if output_format == ExportFormat::Svg {
            render_source_partial(input, &render_args)?
        } else {
            (
                render_source_format(input, &render_args, output_format)?,
                Vec::new(),
            )
        };
        let rendered = if output_format == ExportFormat::Svg {
            finalize_svg_output(rendered, no_xml_tag, omit_version)
        } else {
            rendered
        };
        let mut relative_output =
            board_subtree_output_path(&target, selected, output_format, hash_targets);
        if extensionless {
            relative_output.set_extension("");
        }
        let output = write_dir.join(&relative_output);
        let display_output = output_dir.join(&relative_output);
        if preview_target.is_some() && preview_board.as_ref() == Some(&target) {
            preview = rendered.clone();
        }
        if output_format == ExportFormat::Ascii {
            write_ascii_file(&output, &rendered)?;
        } else {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            write_document_file(&output, &rendered, output_format)
                .map_err(|error| format!("could not write {}: {error}", output.display()))?;
        }
        *output_written = workspace.is_none();
        if !asset_failures.is_empty() {
            return Err(report_asset_failures(&asset_failures));
        }
        log_compile_success(
            Path::new(input_path),
            &display_output,
            render_started.elapsed(),
        );
    }
    if let Some(workspace) = workspace {
        *output_written = workspace.publish()?;
    }
    Ok(preview)
}

fn run_document(args: &[String]) -> Result<Vec<u8>, String> {
    run_document_with_stdout(args, None)
}

fn run_document_with_stdout(
    args: &[String],
    mut stdout_writer: Option<&mut dyn FnMut(&[u8]) -> Result<(), String>>,
) -> Result<Vec<u8>, String> {
    let mut input_path = None;
    let mut output_path = None;
    let mut isometric = std::env::var("D2_ISOMETRIC")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| parse_bool_flag(&s, "isometric"))
        .transpose()?;
    let mut raw_scale = std::env::var("SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(-1.0);
    let mut raw_interval = std::env::var("D2_ANIMATE_INTERVAL")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    // D2 v0.9.0's document command defaults to Dagre. The source-oriented
    // `layout-source` command keeps its explicit TALA default for diagnostics.
    let environment_layout = std::env::var("D2_LAYOUT").ok();
    let mut layout = environment_layout
        .clone()
        .unwrap_or_else(|| "dagre".to_owned());
    let mut layout_explicit = environment_layout.is_some();
    let mut stdout_format = None;
    // D2's ASCII renderer treats any value other than `standard` as its
    // extended Unicode charset. Preserve that permissive fallback.
    let mut ascii_mode = std::env::var("D2_ASCII_MODE").unwrap_or_else(|_| "extended".to_owned());
    let mut no_xml_tag = std::env::var("D2_NO_XML_TAG")
        .ok()
        .map(|value| parse_bool_flag(&value, "no-xml-tag"))
        .transpose()?
        .unwrap_or(false);
    let mut omit_version = std::env::var("OMIT_VERSION")
        .ok()
        .map(|value| parse_bool_flag(&value, "omit-version"))
        .transpose()?
        .unwrap_or(false);
    let mut bundle = std::env::var("D2_BUNDLE")
        .ok()
        .map(|value| parse_bool_flag(&value, "D2_BUNDLE"))
        .transpose()?
        .unwrap_or(true);
    let mut force_appendix = std::env::var("D2_FORCE_APPENDIX")
        .ok()
        .map(|value| parse_bool_flag(&value, "D2_FORCE_APPENDIX"))
        .transpose()?
        .unwrap_or(false);
    let mut pad = std::env::var("D2_PAD")
        .ok()
        .map(|value| {
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid D2_PAD value {value:?}: {error}"))?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err("D2_PAD must be a finite non-negative number".to_owned());
            }
            Ok(parsed)
        })
        .transpose()?;
    // D2 calls this environment variable SCALE (rather than D2_SCALE). Its
    // default -1 means that no explicit scale was requested.
    let mut scale = std::env::var("SCALE")
        .ok()
        .map(|value| {
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid SCALE value {value:?}: {error}"))?;
            if parsed.is_finite() && parsed <= 0.0 {
                return Ok(None);
            }
            if !parsed.is_finite() || parsed <= 0.0 {
                return Err("SCALE must be -1 or a finite positive number".to_owned());
            }
            Ok(Some(parsed))
        })
        .transpose()?
        .flatten();
    let mut salt = None;
    let mut theme_id = std::env::var("D2_THEME")
        .ok()
        .and_then(|value| value.parse::<i64>().ok());
    let mut dark_theme_id = std::env::var("D2_DARK_THEME")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0);
    let mut center = std::env::var("D2_CENTER")
        .ok()
        .map(|value| parse_bool_flag(&value, "center"))
        .transpose()?;
    let mut sketch = std::env::var("D2_SKETCH")
        .ok()
        .map(|value| parse_bool_flag(&value, "sketch"))
        .transpose()?;
    let mut animate_interval = std::env::var("D2_ANIMATE_INTERVAL")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| format!("invalid animate-interval value {value:?}: {error}"))
        })
        .transpose()?;
    if animate_interval == Some(0) {
        return Err("animate-interval must be positive".into());
    }
    // The v0.9.0 CLI gives every document invocation a 120-second deadline;
    // an explicit D2_TIMEOUT environment value replaces it.
    let mut timeout = Some(
        std::env::var("D2_TIMEOUT")
            .ok()
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|error| format!("invalid D2_TIMEOUT value {value:?}: {error}"))
            })
            .transpose()?
            .unwrap_or(120),
    );
    let mut target = None;
    let mut watch_preview = false;
    let mut seeds = std::env::var("D2_TALA_SEEDS")
        .ok()
        .or_else(|| std::env::var("D2_TALA_RS_SEEDS").ok());
    let mut source_adapter_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            for positional in &args[index + 1..] {
                if input_path.is_none() {
                    input_path = Some(positional.clone());
                } else if output_path.is_none() {
                    output_path = Some(positional.clone());
                } else {
                    return Err(command_line::usage("too many arguments passed"));
                }
            }
            break;
        }
        if let Some((name, value)) = command_line::engine_option(args, &mut index)? {
            source_adapter_args.push(format!("--{name}={value}"));
            index += 1;
            continue;
        }
        if argument == "--help"
            || argument == "-h"
                && args
                    .get(index + 1)
                    .is_none_or(|value| value.starts_with('-'))
        {
            return command_line::help();
        }
        if argument == "-v" || argument == "--version" {
            return Ok(command_line::VERSION.as_bytes().to_vec());
        }
        if argument == "--watch" || argument == "-w" {
            // `dispatch` diverts enabled watch invocations to the preview loop.
        } else if argument == "--watch-preview" {
            watch_preview = true;
        } else if let Some(value) = argument.strip_prefix("--watch=") {
            parse_bool_flag(value, "watch")?;
        } else if argument == "--host" || argument == "-h" {
            index += 1;
            if args.get(index).is_none() {
                return Err("--host requires a value".into());
            }
        } else if argument.starts_with("--host=") {
            // Accepted for compatibility; host is used only when watch is enabled.
        } else if argument == "--port" || argument == "-p" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--port requires a value".to_owned())?;
            value
                .parse::<u16>()
                .map_err(|error| format!("invalid port value {value:?}: {error}"))?;
        } else if let Some(value) = argument.strip_prefix("--port=") {
            value
                .parse::<u16>()
                .map_err(|error| format!("invalid port value {value:?}: {error}"))?;
        } else if argument == "--browser" {
            index += 1;
            if args.get(index).is_none() {
                return Err("--browser requires a value".into());
            }
        } else if argument.starts_with("--browser=")
            || argument == "--img-cache"
            || argument.starts_with("--img-cache=")
        {
            // These options only affect the watch preview.
        } else if let Some(value) = argument.strip_prefix("--layout=") {
            layout = value.to_owned();
            layout_explicit = true;
        } else if argument == "--layout" {
            index += 1;
            layout = args
                .get(index)
                .ok_or_else(|| "--layout requires a value".to_owned())?
                .clone();
            layout_explicit = true;
        } else if let Some(value) = argument.strip_prefix("--stdout-format=") {
            stdout_format = Some(value);
        } else if argument == "--stdout-format" {
            index += 1;
            stdout_format = Some(
                args.get(index)
                    .ok_or_else(|| "--stdout-format requires a value".to_owned())?,
            );
        } else if let Some(value) = argument.strip_prefix("--ascii-mode=") {
            ascii_mode = value.to_owned();
        } else if argument == "--ascii-mode" {
            index += 1;
            ascii_mode = args
                .get(index)
                .ok_or_else(|| "--ascii-mode requires a value".to_owned())?
                .clone();
        } else if let Some(value) = argument.strip_prefix("--no-xml-tag=") {
            no_xml_tag = parse_bool_flag(value, "no-xml-tag")?;
        } else if argument == "--no-xml-tag" {
            no_xml_tag = true;
        } else if let Some(value) = argument.strip_prefix("--omit-version=") {
            omit_version = parse_bool_flag(value, "omit-version")?;
        } else if argument == "--omit-version" {
            omit_version = true;
        } else if let Some(value) = argument.strip_prefix("--force-appendix=") {
            force_appendix = parse_bool_flag(value, "force-appendix")?;
        } else if argument == "--force-appendix" {
            force_appendix = true;
        } else if let Some(value) = argument.strip_prefix("--bundle=") {
            bundle = parse_bool_flag(value, "bundle")?;
        } else if argument == "--bundle" || argument == "-b" {
            bundle = true;
        } else if argument == "--no-bundle" {
            bundle = false;
        } else if let Some(value) = argument.strip_prefix("--pad=") {
            // An Int64 flag; d2cli passes negative padding through.
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid pad value {value:?}: {error}"))?;
            if !parsed.is_finite() {
                return Err("pad must be a finite number".into());
            }
            pad = Some(parsed);
        } else if argument == "--pad" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--pad requires a value".to_owned())?;
            // An Int64 flag; d2cli passes negative padding through.
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid pad value {value:?}: {error}"))?;
            if !parsed.is_finite() {
                return Err("pad must be a finite number".into());
            }
            pad = Some(parsed);
        } else if let Some(value) = argument.strip_prefix("--scale=") {
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid scale value {value:?}: {error}"))?;
            raw_scale = parsed;
            scale = (parsed > 0.0).then_some(parsed);
        } else if argument == "--scale" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--scale requires a value".to_owned())?;
            let parsed = value
                .parse::<f64>()
                .map_err(|error| format!("invalid scale value {value:?}: {error}"))?;
            raw_scale = parsed;
            scale = (parsed > 0.0).then_some(parsed);
        } else if let Some(value) = argument.strip_prefix("--salt=") {
            if value.is_empty() {
                return Err("salt must not be empty".into());
            }
            salt = Some(value.to_owned());
        } else if argument == "--salt" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--salt requires a value".to_owned())?;
            if value.is_empty() {
                return Err("salt must not be empty".into());
            }
            salt = Some(value.clone());
        } else if let Some(value) = argument
            .strip_prefix("--theme=")
            .or_else(|| argument.strip_prefix("--theme-id="))
        {
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
            if theme::find(parsed).is_none() {
                return Err(theme_not_found("-t[heme]", parsed));
            }
            theme_id = Some(parsed);
        } else if matches!(argument.as_str(), "--theme" | "--theme-id") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--theme requires a value".to_owned())?;
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
            if theme::find(parsed).is_none() {
                return Err(theme_not_found("-t[heme]", parsed));
            }
            theme_id = Some(parsed);
        } else if let Some(value) = argument
            .strip_prefix("--dark-theme=")
            .or_else(|| argument.strip_prefix("--dark-theme-id="))
        {
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid dark theme value {value:?}: {error}"))?;
            if theme::find(parsed).is_none() {
                return Err(theme_not_found("--dark-theme", parsed));
            }
            dark_theme_id = Some(parsed);
        } else if matches!(argument.as_str(), "--dark-theme" | "--dark-theme-id") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--dark-theme requires a value".to_owned())?;
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid dark theme value {value:?}: {error}"))?;
            if theme::find(parsed).is_none() {
                return Err(theme_not_found("--dark-theme", parsed));
            }
            dark_theme_id = Some(parsed);
        } else if let Some(value) = argument.strip_prefix("--center=") {
            center = Some(parse_bool_flag(value, "center")?);
        } else if argument == "--center" || argument == "-c" {
            center = Some(true);
        } else if argument == "--no-center" {
            center = Some(false);
        } else if argument == "--isometric" {
            isometric = Some(true);
        } else if let Some(value) = argument.strip_prefix("--isometric=") {
            isometric = Some(parse_bool_flag(value, "isometric")?);
        } else if argument == "--sketch" {
            sketch = Some(true);
        } else if let Some(value) = argument.strip_prefix("--sketch=") {
            sketch = Some(parse_bool_flag(value, "sketch")?);
        } else if let Some(value) = argument.strip_prefix("--animate-interval=") {
            // An Int64 flag; d2cli animates only when it is positive.
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid animate-interval value {value:?}: {error}"))?;
            raw_interval = parsed;
            animate_interval = u64::try_from(parsed).ok().filter(|interval| *interval > 0);
        } else if argument == "--animate-interval" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--animate-interval requires a value".to_owned())?;
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid animate-interval value {value:?}: {error}"))?;
            raw_interval = parsed;
            animate_interval = u64::try_from(parsed).ok().filter(|interval| *interval > 0);
        } else if let Some(value) = argument.strip_prefix("--timeout=") {
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid timeout value {value:?}: {error}"))?;
            timeout = Some(parsed);
        } else if argument == "--timeout" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--timeout requires a value".to_owned())?;
            let parsed = value
                .parse::<i64>()
                .map_err(|error| format!("invalid timeout value {value:?}: {error}"))?;
            timeout = Some(parsed);
        } else if let Some(value) = argument.strip_prefix("--target=") {
            target = Some(value.to_owned());
        } else if argument == "--target" {
            index += 1;
            target = Some(
                args.get(index)
                    .ok_or_else(|| "--target requires a value".to_owned())?
                    .clone(),
            );
        } else if let Some(value) = argument
            .strip_prefix("--tala-seeds=")
            .or_else(|| argument.strip_prefix("--tala-rs-seeds="))
        {
            if value.is_empty() {
                return Err("tala-seeds must contain at least one seed".into());
            }
            seeds = Some(value.to_owned());
        } else if matches!(argument.as_str(), "--tala-seeds" | "--tala-rs-seeds") {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--tala-seeds requires a value".to_owned())?;
            if value.is_empty() {
                return Err("tala-seeds must contain at least one seed".into());
            }
            seeds = Some(value.clone());
        } else if argument == "--font"
            || matches!(
                argument.as_str(),
                "--font-regular"
                    | "--font-italic"
                    | "--font-bold"
                    | "--font-semibold"
                    | "--font-mono"
                    | "--font-mono-bold"
                    | "--font-mono-italic"
                    | "--font-mono-semibold"
            )
        {
            let flag = argument.clone();
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.is_empty() {
                return Err(format!("{flag} must not be empty"));
            }
            source_adapter_args.push(flag);
            source_adapter_args.push(value.clone());
        } else if argument == "--font"
            || argument.starts_with("--font=")
            || font_argument(argument).is_some()
        {
            source_adapter_args.push(argument.clone());
        } else if argument.starts_with('-') && argument != "-" {
            return Err(format!("unrecognized option {argument}"));
        } else if input_path.is_none() {
            input_path = Some(argument.clone());
        } else if output_path.is_none() {
            output_path = Some(argument.clone());
        } else {
            return Err(command_line::usage("too many arguments passed"));
        }
        index += 1;
    }

    let mut input_path = input_path.ok_or_else(|| "expected an input .d2 file or -".to_owned())?;
    // d2cli.Run derives the default output from the argument as given, then
    // resolves both paths with ms.AbsPath (diagnostics use the absolute
    // source path; success lines print HumanPath).
    if let Some(path) = output_path.as_mut().filter(|path| path.as_str() != "-") {
        *path = go_abs_path(path);
    }
    if input_path != "-" {
        input_path = go_abs_path(&input_path);
    }
    // d2cli/main.go rejects the old PowerPoint extension before selecting
    // the output format or reading the source, even with --stdout-format.
    if output_path
        .as_deref()
        .and_then(|path| Path::new(path).extension())
        .is_some_and(|extension| extension == "ppt")
    {
        return Err(command_line::usage(
            "D2 does not support ppt exports, did you mean \"pptx\"?",
        ));
    }
    // getOutputFormat treats an explicit empty flag just like an unset one.
    let stdout_format = stdout_format.filter(|value| !value.is_empty());
    let output_format = if let Some(stdout_format) = stdout_format {
        cli_stdout_format(stdout_format)?
    } else {
        cli_export_extension(output_path.as_deref().unwrap_or(""))
    };
    if isometric == Some(true) {
        isometric_cli::validate(
            output_format,
            output_path.as_deref(),
            stdout_format.is_some(),
            force_appendix,
            raw_scale,
            raw_interval,
        )
        .map_err(command_line::usage)?;
    }
    // d2cli/main.go:252–258 checks the destination before reading the source.
    // The stdout destination deliberately bypasses this file-only restriction.
    let animation_destination = output_path
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| default_output_path(&input_path, output_format));
    if isometric != Some(true)
        && animate_interval.is_some_and(|interval| interval > 0)
        && !matches!(output_format, ExportFormat::Svg | ExportFormat::Gif)
        && let Some(destination) = animation_destination
        && destination.as_os_str() != "-"
    {
        let extension = destination
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        return Err(command_line::usage(format!(
            "--animate-interval can only be used when exporting to SVG or GIF.\nYou provided: {extension}"
        )));
    }
    if dark_theme_id.is_some() && output_format != ExportFormat::Svg {
        eprintln!(
            "warn: --dark-theme cannot be used while exporting to another format other than .svg"
        );
        dark_theme_id = None;
    }
    // d2cli/main.go:231–244 chooses the output from the original argument
    // before resolving a directory to its index.d2 source. Keep the resolved
    // source path for imports and diagnostics, and the original output stem.
    if input_path != "-" && Path::new(&input_path).is_dir() {
        if output_path.is_none() {
            output_path = default_output_path(&input_path, output_format)
                .map(|path| path.to_string_lossy().into_owned());
        }
        input_path = Path::new(&input_path)
            .join("index.d2")
            .to_string_lossy()
            .into_owned();
    }
    let _deadline_scope = deadline::Scope::document(timeout.and_then(deadline::duration));
    let compile_started = std::time::Instant::now();
    let input = if input_path == "-" {
        read_stdin()?
    } else {
        std::fs::read(&input_path).map_err(|error| {
            #[cfg(windows)]
            let absolute = PathBuf::from(&input_path);
            #[cfg(not(windows))]
            let absolute =
                std::path::absolute(&input_path).unwrap_or_else(|_| PathBuf::from(&input_path));
            document_compile_error(
                Path::new(&input_path),
                local_asset_read_error(&absolute, error),
            )
        })?
    };
    let _source_config = SourceConfigScope::enter(&input);
    // Go starts this ticker after reading the source and constructing its
    // compile options, immediately before d2lib.Compile. Keep it live across
    // every Rust compile/layout return path, including limit errors.
    let compile_progress = CompileProgress::start();
    let display_input_path = PathBuf::from(&input_path);
    let mut partial_written = false;
    let result = (|| -> Result<Vec<u8>, String> {
        let output_path = output_path.as_deref();
        let mut compiled_source = None;
        let mut _compiled_source_scope = None;
        // d2cli.compile substitutes ELK for the Dagre flag before source
        // config is applied, including an explicitly requested Dagre flag.
        if output_format == ExportFormat::Ascii && layout == "dagre" {
            layout = "elk".to_owned();
            // Go passes this substituted non-nil Layout pointer into Compile,
            // so source configuration cannot override the ASCII default.
            layout_explicit = true;
        }
        if !layout_explicit
            || seeds.is_none()
            || sketch.is_none()
            || pad.is_none()
            || theme_id.is_none()
            || dark_theme_id.is_none()
            || center.is_none()
            || isometric.is_none()
        {
            // d2lib.compileInput receives graph and config from the same
            // d2compiler.Compile call. Keep this IR for the board plan rather
            // than parsing/compiling it again after applying source options.
            let (compiler, root) = compile_board_source(&input, &input_path)?;
            let compiled = CompiledSource::new(&input, &input_path, compiler, root);
            let config = source_config_from_compiled(&compiled.compiler, compiled.root);
            remember_source_config(&input, &input_path, &config);
            _compiled_source_scope = Some(CompiledSourceScope::enter(compiled.clone()));
            compiled_source = Some(compiled);
            if let Some(config) = config? {
                if !layout_explicit {
                    if let Some(layout_engine) = config.layout_engine.as_deref() {
                        layout = layout_engine.to_owned();
                    } else if output_format == ExportFormat::Ascii {
                        layout = "elk".to_owned();
                    }
                }
                if pad.is_none()
                    && let Some(configured_pad) = config.pad
                {
                    pad = Some(configured_pad as f64);
                }
                if theme_id.is_none()
                    && let Some(configured_theme) = config.theme_id
                {
                    if theme::find(configured_theme).is_none() {
                        return Err(format!("unknown theme ID {configured_theme}"));
                    }
                    theme_id = Some(configured_theme);
                }
                if dark_theme_id.is_none()
                    && let Some(configured_theme) = config.dark_theme_id
                {
                    if theme::find(configured_theme).is_none() {
                        return Err(format!("unknown dark theme ID {configured_theme}"));
                    }
                    dark_theme_id = Some(configured_theme);
                }
                if isometric.is_none() {
                    isometric = config.isometric;
                }
                if center.is_none() {
                    center = config.center;
                }
                if sketch.is_none() {
                    sketch = config.sketch;
                }
            } else if !layout_explicit && output_format == ExportFormat::Ascii {
                layout = "elk".to_owned();
            }
        }
        // d2plugin.FindPlugin matches the bundled names with EqualFold.
        let selected_engine = match layout.to_lowercase().as_str() {
            "dagre" => EngineKind::Dagre,
            "elk" => EngineKind::Elk,
            "tala" => EngineKind::Tala,
            _ => return Err(layout_not_found(&layout)),
        };
        if isometric == Some(true) {
            drop(compiled_source);
            isometric_cli::validate_source_format(output_format)?;
            isometric_cli::validate(
                output_format,
                output_path,
                stdout_format.is_some(),
                force_appendix,
                raw_scale,
                raw_interval,
            )?;
            if sketch == Some(true) {
                return Err("sketch cannot be combined with isometric rendering".into());
            }
            return isometric_cli::run(
                &input,
                &input_path,
                output_path,
                output_format,
                raw_interval,
                target.as_deref(),
                &selected_engine.to_string(),
                seeds.as_deref(),
                pad,
                scale,
                salt.as_deref(),
                theme_id,
                dark_theme_id,
                center,
                &source_adapter_args,
                no_xml_tag,
                omit_version,
                watch_preview,
                compile_started,
            );
        }
        if !raw_scale.is_finite() {
            return Err("scale must be finite".into());
        }
        let preview_boards =
            watch_preview && output_format == ExportFormat::Svg && animate_interval.is_none();
        let package_all_boards =
            target.is_none() || target.as_deref() == Some("*") || preview_boards;
        let animation_subtree = if (output_format == ExportFormat::Svg
            && animate_interval.is_some())
            || output_format == ExportFormat::Gif
        {
            target.as_deref().and_then(|value| value.strip_suffix(".*"))
        } else {
            None
        };
        let paged_target = if matches!(output_format, ExportFormat::Pdf | ExportFormat::Pptx) {
            target.as_deref().filter(|value| *value != "*")
        } else {
            None
        };
        // d2cli/main.go:368–385 retains a named board's descendants for
        // `board.*`; renderWithPNGEncoder:859–947 starts the output tree at
        // that board's name. Use the same traversal as watch navigation,
        // without returning a preview on ordinary file writes.
        let recursive_subtree = if !preview_boards
            && animate_interval.is_none()
            && matches!(output_format, ExportFormat::Svg | ExportFormat::Png)
        {
            target.as_deref().and_then(|value| value.strip_suffix(".*"))
        } else {
            None
        };
        let mut hash_board_targets = None;
        if package_all_boards
            || animation_subtree.is_some()
            || recursive_subtree.is_some()
            || paged_target.is_some()
        {
            let plan_target = if preview_boards {
                target.as_deref().filter(|t| *t != "*").unwrap_or("")
            } else {
                recursive_subtree
                    .or(animation_subtree)
                    .or(paged_target.map(|value| value.strip_suffix(".*").unwrap_or(value)))
                    .unwrap_or("")
            };
            let (full_plan, preview_target) = match compiled_source.take() {
                Some(compiled) => {
                    source_board_plan_from_compiled(&compiled.compiler, compiled.root, plan_target)
                }
                None => source_board_plan_for_target(&input, &input_path, plan_target),
            }?;
            let (animation_root, mut board_plan) = if animation_subtree.is_some()
                || paged_target.is_some_and(|value| !value.is_empty())
            {
                // Resolve shorthand/quoted names once through the compiler,
                // then retain exactly the selected board's descendants.
                animation_subtree_plan(full_plan, &preview_target)?
            } else {
                (String::new(), full_plan)
            };
            if paged_target.is_some_and(|value| !value.ends_with(".*")) {
                board_plan.targets.clear();
                board_plan.all_targets.clear();
                board_plan.postorder_targets.clear();
                board_plan
                    .renderable_targets
                    .retain(|value| value == &animation_root);
            }
            if output_format == ExportFormat::Svg && animate_interval.is_some() {
                validate_animation_destination(
                    &input_path,
                    output_path,
                    &animation_root,
                    &board_plan.all_targets,
                )?;
            }
            // D2 emits no files for a document with no renderable boards.
            // Root attributes alone do not turn an empty diagram into a
            // blank canvas, including an empty sequence-diagram declaration.
            if !board_plan.root_renderable && board_plan.targets.is_empty() {
                if paged_target.is_some() {
                    // paged.go:newPagedRenderer rejects a selected folder
                    // whose children were removed by a non-wildcard target.
                    return Err("paged export requires at least one renderable board".into());
                }
                // Documents and GIFs skip d2cli/main.go's folder-only render
                // walk: paged.go:newPagedRenderer and raster.go
                // renderGIFWithSession count the renderable boards themselves.
                match output_format {
                    ExportFormat::Pdf | ExportFormat::Pptx => {
                        return Err("paged export requires at least one renderable board".into());
                    }
                    ExportFormat::Gif => {
                        return Err("GIF animation requires at least one renderable board".into());
                    }
                    _ => {}
                }
                // d2cli/main.go still stages and publishes a document with
                // child boards: the output tree holds only its manifest.
                if !board_plan.all_targets.is_empty()
                    && animate_interval.is_none()
                    && output_path != Some("-")
                    && !matches!(
                        output_format,
                        ExportFormat::Pdf | ExportFormat::Pptx | ExportFormat::Gif
                    )
                    && let Some(requested_output) = output_path
                        .map(PathBuf::from)
                        .or_else(|| default_output_path(&input_path, output_format))
                {
                    board_output::BoardOutputWorkspace::new(&requested_output.with_extension(""))?
                        .publish()?;
                }
                return Ok(Vec::new());
            }
            // planBoardOutput uses every child board, including folder-only
            // steps that precede the root's first drawable object.
            if !board_plan.all_targets.is_empty()
                || recursive_subtree.is_some()
                || matches!(output_format, ExportFormat::Pdf | ExportFormat::Pptx)
                || (output_format == ExportFormat::Gif && animation_subtree.is_some())
                || (output_format == ExportFormat::Svg
                    && animate_interval.is_some()
                    && (!board_plan.all_targets.is_empty() || animation_subtree.is_some()))
            {
                return run_multiboard_document(
                    &input_path,
                    &input,
                    output_format,
                    &selected_engine.to_string(),
                    seeds.as_deref(),
                    pad,
                    scale,
                    theme_id,
                    dark_theme_id,
                    center,
                    sketch,
                    bundle,
                    force_appendix,
                    salt.as_deref(),
                    animate_interval.or((output_format == ExportFormat::Gif).then_some(1000)),
                    timeout,
                    &source_adapter_args,
                    output_path,
                    &board_plan.targets,
                    &board_plan.all_targets,
                    &animation_root,
                    &board_plan.postorder_targets,
                    board_plan.root_renderable,
                    &board_plan.renderable_targets,
                    &ascii_mode,
                    no_xml_tag,
                    omit_version,
                    preview_boards.then_some(preview_target.as_str()),
                    recursive_subtree.map(|_| preview_target.as_str()),
                    &mut partial_written,
                );
            }
            hash_board_targets = Some(board_plan.all_targets);
        }
        drop(compiled_source);
        let mut render_args = source_path_args(&input_path);
        render_args.push(format!("--ascii-mode={ascii_mode}"));
        render_args.push(format!("--engine={selected_engine}"));
        if let Some(seeds) = seeds {
            render_args.push(format!("--seeds={seeds}"));
        }
        if let Some(pad) = pad {
            render_args.push(format!("--pad={pad}"));
        }
        if let Some(scale) = scale {
            render_args.push(format!("--scale={scale}"));
        }
        if let Some(salt) = salt.as_deref() {
            render_args.push(format!("--salt={salt}"));
        }
        render_args.push(format!("--bundle={bundle}"));
        render_args.push(format!("--force-appendix={force_appendix}"));
        if let Some(theme_id) = theme_id {
            render_args.push(format!("--theme={theme_id}"));
        }
        if let Some(dark_theme_id) = dark_theme_id {
            render_args.push(format!("--dark-theme={dark_theme_id}"));
        }
        if let Some(center) = center {
            render_args.push(format!("--center={center}"));
        }
        if sketch == Some(true) {
            render_args.push("--sketch".into());
        }
        if let Some(timeout) = timeout {
            render_args.push(format!("--timeout={timeout}"));
        }
        if let Some(target) = target.as_deref() {
            render_args.push(format!("--target={target}"));
        }
        render_args.extend(source_adapter_args);
        if output_format == ExportFormat::Svg && animate_interval.is_none() {
            render_args.push("--document-link-format=svg".into());
            if output_path == Some("-") || output_path.is_none() && input_path == "-" {
                render_args.push("--document-link-extensionless".into());
            }
        }
        // d2lib.compileBoard lays out every board before the CLI selects a
        // named target. Preserve errors from earlier boards for leaf exports.
        let _board_layouts = target
            .as_deref()
            .filter(|value| *value != "*")
            .map(|_| BoardLayoutScope::enter());
        if _board_layouts.is_some() {
            let (plan, _) = source_board_plan_for_target(&input, &input_path, "")?;
            let mut board_args = render_args.clone();
            board_args.retain(|arg| !arg.starts_with("--target="));
            for board in std::iter::once("")
                .filter(|_| plan.root_renderable)
                .chain(plan.targets.iter().map(String::as_str))
            {
                let mut args = board_args.clone();
                args.push(if board.is_empty() {
                    "--target=*".to_owned()
                } else {
                    format!("--target={board}.*")
                });
                let (source_args, options, _, sketch_explicit) = extract_render_options(&args)?;
                let theme = hydrate_render_theme(&input, &source_args, options.theme_id)?;
                let sketch = hydrate_render_sketch(&input, &source_args, sketch_explicit)?;
                source_render_layout_with_theme(&input, &source_args, theme, sketch)?
                    .target_metadata?;
            }
        }
        let animation_context = if output_format == ExportFormat::Svg && animate_interval.is_some()
        {
            Some(render_source_target_animation(&input, &render_args)?)
        } else {
            None
        };
        let pptx_properties = (output_format == ExportFormat::Pptx)
            .then(|| pptx_document_properties(&input_path, output_path));
        let mut asset_failures = Vec::new();
        let (rendered, preview) = if watch_preview {
            let gif_interval =
                (output_format == ExportFormat::Gif).then_some(animate_interval.unwrap_or(1000));
            let (mut rendered, mut preview) = render_source_format_with_pptx_properties(
                input.as_slice(),
                &render_args,
                output_format,
                None,
                gif_interval,
                true,
                matches!(
                    output_format,
                    ExportFormat::Png | ExportFormat::Gif | ExportFormat::Pdf | ExportFormat::Pptx
                ),
                pptx_properties.as_ref(),
            )?;
            if output_format == ExportFormat::Svg
                && let Some(interval) = animate_interval
            {
                rendered = animation_context
                    .as_ref()
                    .expect("animated SVG context")
                    .package_svg(rendered, interval, salt.as_deref(), &render_args)?;
                preview = Some(rendered.clone());
            }
            if output_format == ExportFormat::Ascii {
                preview = Some(rendered.clone());
            } else if output_format == ExportFormat::Svg {
                rendered = finalize_svg_output(rendered, no_xml_tag, omit_version);
                preview = Some(rendered.clone());
            }
            (rendered, preview)
        } else {
            let rendered = if output_format == ExportFormat::Svg && animate_interval.is_some() {
                let (svg, failures) = render_source_partial_with_board_targets(
                    input.as_slice(),
                    &render_args,
                    hash_board_targets.as_deref(),
                )?;
                if !failures.is_empty() {
                    return Err(report_asset_failures(&failures));
                }
                animation_context
                    .as_ref()
                    .expect("animated SVG context")
                    .package_svg(
                        svg,
                        animate_interval.unwrap_or(1000),
                        salt.as_deref(),
                        &render_args,
                    )?
            } else if output_format == ExportFormat::Gif {
                render_source_animated_gif(
                    input.as_slice(),
                    &render_args,
                    animate_interval.unwrap_or(1000),
                )?
            } else if output_format == ExportFormat::Svg {
                let (svg, failures) = render_source_partial_with_board_targets(
                    input.as_slice(),
                    &render_args,
                    hash_board_targets.as_deref(),
                )?;
                asset_failures = failures;
                svg
            } else if let Some(properties) = &pptx_properties {
                render_source_format_with_pptx_properties(
                    input.as_slice(),
                    &render_args,
                    output_format,
                    None,
                    None,
                    false,
                    false,
                    Some(properties),
                )?
                .0
            } else {
                render_source_format(input.as_slice(), &render_args, output_format)?
            };
            let rendered = if output_format == ExportFormat::Svg {
                finalize_svg_output(rendered, no_xml_tag, omit_version)
            } else {
                rendered
            };
            (rendered, None)
        };
        let output_path = output_path
            .map(PathBuf::from)
            .or_else(|| default_output_path(&input_path, output_format));
        if !asset_failures.is_empty()
            && output_path
                .as_ref()
                .is_none_or(|path| path.as_os_str() == "-")
        {
            write_stdout(&rendered)?;
            partial_written = true;
            return Err(report_asset_failures(&asset_failures));
        }
        match output_path {
            None => {
                let output = preview.unwrap_or(rendered);
                if let Some(writer) = stdout_writer.as_mut() {
                    writer(&output)?;
                    log_compile_success(
                        &display_input_path,
                        Path::new("-"),
                        compile_started.elapsed(),
                    );
                    Ok(Vec::new())
                } else {
                    Ok(output)
                }
            }
            Some(path) if path.as_os_str() == "-" => {
                let output = preview.unwrap_or(rendered);
                if let Some(writer) = stdout_writer.as_mut() {
                    writer(&output)?;
                    log_compile_success(&display_input_path, &path, compile_started.elapsed());
                    Ok(Vec::new())
                } else {
                    Ok(output)
                }
            }
            Some(path) => {
                if output_format == ExportFormat::Ascii {
                    write_ascii_file(&path, &rendered)?;
                } else {
                    // d2cli/main.go:1054–1057 writes a final newline while
                    // returning the original SVG bytes to the watch preview.
                    let mut file_bytes = rendered;
                    if output_format == ExportFormat::Svg
                        && animate_interval.is_none()
                        && !file_bytes.is_empty()
                        && !file_bytes.ends_with(b"\n")
                    {
                        file_bytes.push(b'\n');
                    }
                    write_document_file(&path, &file_bytes, output_format)
                        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
                }
                if !asset_failures.is_empty() {
                    partial_written = true;
                    return Err(report_asset_failures(&asset_failures));
                }
                log_compile_success(&display_input_path, &path, compile_started.elapsed());
                Ok(preview.unwrap_or_default())
            }
        }
    })();
    drop(compile_progress);
    result.map_err(|error| {
        if partial_written {
            format!(
                "err: failed to fully compile (partial render written) {}: {error}",
                human_path(&display_input_path)
            )
        } else {
            document_compile_error(&display_input_path, error)
        }
    })
}

fn log_compile_success(input: &Path, output: &Path, elapsed: std::time::Duration) {
    eprintln!(
        "success: successfully compiled {} to {} in {elapsed:?}",
        human_path(input),
        human_path(output)
    );
}

const COMPILE_PROGRESS_MESSAGE: &str = "info: compiling & running layout algorithms...";

/// d2cli.compile reports a long-running d2lib.Compile every five seconds.
/// The sender wakes the thread on every return path, so a quick compile never
/// waits for the ticker or prints after its caller has moved on to another
/// document.
struct CompileProgress {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl CompileProgress {
    fn start() -> Self {
        Self::start_with(std::time::Duration::from_secs(5), || {
            eprintln!("{COMPILE_PROGRESS_MESSAGE}")
        })
    }

    fn start_with(
        interval: std::time::Duration,
        mut report: impl FnMut() + Send + 'static,
    ) -> Self {
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while matches!(
                receive.recv_timeout(interval),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                report();
            }
        });
        Self {
            stop: Some(send),
            worker: Some(worker),
        }
    }
}

impl Drop for CompileProgress {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// imgbundler.BundleWithResolver's returned error: the worker context's
/// cause is the first failure (or the caller's cancellation).
fn report_asset_failures(failures: &[(String, String)]) -> String {
    let cause = watch::active_cancellation_token()
        .and_then(|token| token.error())
        .map(|error| error.to_string())
        .or_else(|| failures.first().map(|(_, error)| error.clone()))
        .unwrap_or_default();
    format!("failed to bundle images: {cause}")
}

/// xmain.State.AbsPath: join a relative path to the working directory and
/// clean it lexically.
fn go_abs_path(path: &str) -> String {
    // xmain.State.AbsPath returns an absolute argument verbatim. On Windows
    // its forward slashes matter to d2ir's slash-only path.Dir import base.
    #[cfg(windows)]
    if Path::new(path).is_absolute() {
        return path.to_owned();
    }
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    clean_asset_path(&joined).display().to_string()
}

/// xmain.State.ReadPath: `-` reads stdin, anything else os.ReadFile.
fn go_read_path(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        return read_stdin();
    }
    std::fs::read(path).map_err(|error| local_asset_read_error(Path::new(path), error))
}

/// The `--check` value forwarded by the flag boundary (last one wins).
fn check_flag(args: &[String]) -> bool {
    args.iter()
        .filter_map(|argument| {
            if argument == "--check" {
                Some("true")
            } else {
                argument.strip_prefix("--check=")
            }
        })
        .next_back()
        .is_some_and(|value| value == "true")
}

// d2cli/fmt.go fmtCmd. Every positional after `fmt` is a path; flags were
// parsed and validated by the d2cli.Run boundary.
fn run_fmt(args: &[String]) -> Result<Vec<u8>, CommandError> {
    run_fmt_paths(args).map_err(|error| match error {
        // xmain.mainFatal extracts ExitError.Message through the deferred
        // fmtCmd wrapper instead of printing the wrapper's Error() string.
        FmtError::Unformatted(count) => CommandError::Exit {
            context: "failed to fmt",
            code: 1,
            message: format!(
                "found {count} unformatted {}. Run d2 fmt to fix.",
                if count == 1 { "file" } else { "files" }
            ),
        },
        FmtError::Failure(error) => CommandError::Failure(format!("failed to fmt: {error}")),
    })
}

enum FmtError {
    Failure(String),
    Unformatted(usize),
}

impl From<String> for FmtError {
    fn from(error: String) -> Self {
        Self::Failure(error)
    }
}

fn run_fmt_paths(args: &[String]) -> Result<Vec<u8>, FmtError> {
    let check = check_flag(args);
    let paths = command_line::positional_arguments(args);
    if paths.is_empty() {
        return Err(
            command_line::usage("fmt must be passed at least one file to be formatted").into(),
        );
    }
    let mut stdout = Vec::new();
    let mut unformatted_count = 0;
    for path in paths {
        let mut input_path = path.clone();
        if input_path != "-" {
            input_path = go_abs_path(&input_path);
            if Path::new(&input_path).is_dir() {
                input_path = Path::new(&input_path)
                    .join("index.d2")
                    .display()
                    .to_string();
            }
        }
        let input = go_read_path(&input_path)?;
        let formatted = format_source(&input, &source_path_args(&input_path))?;
        if formatted != input {
            if check {
                unformatted_count += 1;
                cli_log::warn(&input_path);
            } else if input_path == "-" {
                stdout.extend_from_slice(&formatted);
            } else {
                std::fs::write(&input_path, formatted)
                    .map_err(|error| local_asset_read_error(Path::new(&input_path), error))?;
            }
        }
    }
    if unformatted_count > 0 {
        return Err(FmtError::Unformatted(unformatted_count));
    }
    Ok(stdout)
}

// d2cli/validate.go validateCmd validates only its first path argument.
// xdefer.Errorf(&err, "") prefixes the defining function's name.
fn run_validate(args: &[String]) -> Result<Vec<u8>, String> {
    run_validate_path(args)
        .map_err(|error| format!("github.com/d2lang/d2/d2cli.validateCmd: {error}"))
}

fn run_validate_path(args: &[String]) -> Result<Vec<u8>, String> {
    let paths = command_line::positional_arguments(args);
    let Some(path) = paths.first() else {
        return Err(command_line::usage("input argument required"));
    };
    let input_path = if path == "-" {
        path.clone()
    } else {
        go_abs_path(path)
    };
    let input = go_read_path(&input_path)?;
    // validateCmd calls d2lib.Parse(ctx, input, nil): no source path.
    run(&["validate".to_owned(), "--path=".to_owned()], &input)?;
    let display_path = if input_path == "-" {
        "Input".to_owned()
    } else {
        input_path
    };
    Ok(validation_success_message(&display_path).into_bytes())
}

fn validation_success_message(path: &str) -> String {
    format!("Success! [{path}] is valid D2.\n")
}

// d2cli/help.go layoutCmd, shortLayoutHelp, longLayoutHelp and
// layoutNotFound over the bundled plugins in ListPluginInfos order, with each
// plugin's ShortHelp/LongHelp from d2plugin/plugin_{dagre,elk,tala}.go.
const BUNDLED_LAYOUTS: &[(&str, &str)] = &[
    ("dagre", "The directed graph layout library Dagre"),
    (
        "elk",
        "Eclipse Layout Kernel (ELK) with the Layered algorithm.",
    ),
    (
        "tala",
        "TALA is D2's native layout and edge-routing engine.",
    ),
];

fn layout_not_found(layout: &str) -> String {
    let names = BUNDLED_LAYOUTS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    command_line::usage(format!(
        "D2_LAYOUT \"{layout}\" is not a supported built-in layout engine.\nThe available options are: {names}. For details on each option, run \"d2 layout\".\nExternal d2plugin executables are no longer supported."
    ))
}

fn run_layout_command(args: &[String]) -> Result<Vec<u8>, String> {
    if args
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        return command_line::help();
    }
    let Some(layout) = args.first() else {
        let lines = BUNDLED_LAYOUTS
            .iter()
            .map(|(name, short)| format!("{name} (built-in) - {short}"))
            .collect::<Vec<_>>()
            .join("\n");
        // ms.Name is the program path exactly as invoked.
        let name = std::env::args().next().unwrap_or_else(|| "d2".into());
        return Ok(format!(
            "Available layout engines:\n\n{lines}\n\nUsage:\n  To use a particular layout engine, set the environment variable D2_LAYOUT=[name] or flag --layout=[name].\n\nExample:\n  D2_LAYOUT=dagre d2 in.d2 out.svg\n\nSubcommands:\n  {name} layout [layout name] - Display long help for a particular layout engine, including its configuration options\n\nSee more docs at https://d2lang.com/tour/layouts\n"
        )
        .into_bytes());
    };
    let long_help = match layout.as_str() {
        "dagre" => format!(
            "dagre is a directed graph layout algorithm implemented natively in Go by Dagro.\nSee https://d2lang.com/tour/dagre for more.\n\nDagro implements the Dagre 3.1.1 layout surface used by D2: https://github.com/d2lang/dagro.\n\nFlags correspond to ones found at https://github.com/dagrejs/dagre/wiki.\n\nFlags:\n{}\n",
            command_line::layout_flags("dagre")
        ),
        "elk" => format!(
            "ELK is a layout engine offered by Eclipse.\nOriginally written in Java, D2's ELK.js 0.12.0 layout profile is bundled through the native Go elk-go port. Layered remains the default.\nSee https://d2lang.com/tour/elk for more.\n\nFlags correspond to ones found at https://www.eclipse.org/elk/reference.html.\n\nFlags:\n{}\n",
            command_line::layout_flags("elk")
        ),
        "tala" => format!(
            "TALA is D2's native layout and edge-routing engine for software architecture diagrams.\n\nDiagram data under tala-seeds takes precedence over the command-line flag.\n\nFlags:\n{}\n",
            command_line::layout_flags("tala")
        ),
        _ => return Err(layout_not_found(layout)),
    };
    Ok(format!("{layout} (built-in):\n\n{long_help}").into_bytes())
}

/// d2cli/export.go getOutputFormat over STDOUT_FORMAT_MAP: an exact
/// lower-cased name, else a usage error naming SUPPORTED_STDOUT_FORMATS.
fn cli_stdout_format(value: &str) -> Result<ExportFormat, String> {
    match value.to_lowercase().as_str() {
        "png" => Ok(ExportFormat::Png),
        "svg" => Ok(ExportFormat::Svg),
        "ascii" | "txt" => Ok(ExportFormat::Ascii),
        "pdf" => Ok(ExportFormat::Pdf),
        "pptx" => Ok(ExportFormat::Pptx),
        "gif" => Ok(ExportFormat::Gif),
        _ => Err(command_line::usage(format!(
            "{value} is not a supported format. Supported formats are: [png svg ascii txt pdf pptx gif]"
        ))),
    }
}

/// d2cli/export.go getExportExtension: exact filename extensions, with SVG
/// as the fallback. The library parser also accepts aliases and upper case.
fn cli_export_extension(path: &str) -> ExportFormat {
    let extension = path.rsplit('/').next().unwrap_or("").rsplit_once('.');
    match extension.map(|(_, extension)| extension) {
        Some("png") => ExportFormat::Png,
        Some("gif") => ExportFormat::Gif,
        Some("pdf") => ExportFormat::Pdf,
        Some("pptx") => ExportFormat::Pptx,
        Some("txt") => ExportFormat::Ascii,
        _ => ExportFormat::Svg,
    }
}

/// d2cli.Run's catalog lookup failure for -t[heme] and --dark-theme.
fn theme_not_found(flag: &str, id: i64) -> String {
    let catalog = d2_rs_core::theme::catalog_text();
    let catalog = catalog
        .strip_prefix("Available themes:\n")
        .unwrap_or(&catalog);
    command_line::usage(format!(
        "{flag} could not be found. The available options are:\n{catalog}\nYou provided: {id}"
    ))
}

// d2cli themesCmd ignores any further positionals.
fn run_themes(_args: &[String]) -> Result<Vec<u8>, String> {
    Ok(d2_rs_core::theme::catalog_text().into_bytes())
}

/// Encodes a D2 script exactly as the v0.9.0 playground URL helper does.
/// D2 uses a raw DEFLATE stream at best compression followed by padded URL
/// safe base64.
fn encode_play_script(source: &[u8]) -> Result<String, String> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(source)
        .map_err(|error| format!("could not compress playground input: {error}"))?;
    let compressed = encoder
        .finish()
        .map_err(|error| format!("could not finish playground input: {error}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE.encode(compressed))
}

fn open_playground(url: &str) -> Result<(), String> {
    let browser = std::env::var("BROWSER").ok();
    if browser.as_deref() == Some("0") || std::env::var_os("D2_RS_NO_BROWSER").is_some() {
        return Ok(());
    }
    if let Some(browser) = browser.filter(|browser| !browser.is_empty()) {
        return std::process::Command::new(browser)
            .arg(url)
            .status()
            .map_err(|error| format!("could not open playground in a browser: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("browser exited with status {status}"))
                }
            });
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    command
        .status()
        .map_err(|error| format!("could not open playground in a browser: {error}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("browser exited with status {status}"))
            }
        })
}

fn run_play(args: &[String]) -> Result<Vec<u8>, String> {
    let mut source_path = None;
    let mut theme_id = 0_i64;
    let mut sketch = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--sketch" || argument == "-s" {
            sketch = true;
        } else if let Some(value) = argument.strip_prefix("--theme=") {
            theme_id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
        } else if argument == "--theme" || argument == "-t" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "--theme requires a value".to_owned())?;
            theme_id = value
                .parse::<i64>()
                .map_err(|error| format!("invalid theme value {value:?}: {error}"))?;
        } else if argument.starts_with('-') && argument != "-" {
            return Err(format!("unrecognized play option {argument}"));
        } else if source_path.replace(argument.clone()).is_some() {
            return Err(command_line::usage(
                "play must be passed one argument: either a filepath or '-' for stdin",
            ));
        }
        index += 1;
    }
    let source_path = source_path.ok_or_else(|| {
        command_line::usage("play must be passed one argument: either a filepath or '-' for stdin")
    })?;
    let source = read_path(&source_path)?;
    let encoded = encode_play_script(&source)?;
    let url = format!(
        "https://play.d2lang.com/?script={encoded}&sketch={}&theme={theme_id}&",
        i32::from(sketch)
    );
    eprintln!("opening playground: {url}");
    open_playground(&url)?;
    Ok(Vec::new())
}

#[derive(Debug)]
enum CommandError {
    Failure(String),
    Exit {
        context: &'static str,
        code: i32,
        message: String,
    },
}

impl From<String> for CommandError {
    fn from(error: String) -> Self {
        Self::Failure(error)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failure(message) => formatter.write_str(message),
            Self::Exit {
                context,
                code,
                message,
            } => write!(formatter, "{context}: exiting with code {code}: {message}"),
        }
    }
}

impl CommandError {
    fn terminal_message(&self) -> &str {
        match self {
            Self::Failure(message) | Self::Exit { message, .. } => message,
        }
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Failure(_) => 1,
            Self::Exit { code, .. } => *code,
        }
    }

    /// Optional embedding/test transport: retain the API error independently
    /// of xmain.mainFatal's terminal projection. No expected text is injected.
    fn write_record(&self) -> std::io::Result<()> {
        let Some(path) = std::env::var_os("D2_RS_CLI_ERROR_JSON") else {
            return Ok(());
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({
                "error": self.to_string(), "message": self.terminal_message(), "code": self.exit_code(),
                "kind": if matches!(self, Self::Exit { .. }) { "exit" } else { "failure" }
            }),
        )?;
        file.write_all(b"\n")
    }
}

fn dispatch(args: &[String]) -> Result<Vec<u8>, CommandError> {
    let Some(command) = args.first().map(String::as_str) else {
        return command_line::help().map_err(Into::into);
    };
    if command == "--help" {
        return command_line::help().map_err(Into::into);
    }
    // d2cli.Run parses the complete pinned flag set (and loads supplied
    // fonts) before any subcommand dispatch. The d2-rs internal protocol and
    // source commands keep their own option parsers.
    let internal = SOURCE_COMMANDS.contains(&command) && !matches!(command, "validate" | "version");
    if !internal {
        return match command_line::version_boundary(args)? {
            command_line::Boundary::Output(output) => Ok(output),
            command_line::Boundary::Document(document) => {
                dispatch_without_version(&apply_network_policy(document))
            }
        };
    }
    dispatch_without_version(args)
}

/// d2cli.Run: `ctx = netpolicy.WithPolicy(ctx, Policy{AllowPrivateNetworks:
/// *allowPrivateNetworkFlag})`. The boundary validated the environment default
/// and normalized explicit flags to `--allow-private-network=<bool>`; the
/// policy is process state, so the option is consumed here.
fn apply_network_policy(args: Vec<String>) -> Vec<String> {
    let mut allow = matches!(
        std::env::var("D2_ALLOW_PRIVATE_NETWORK").as_deref(),
        Ok("1" | "true")
    );
    let mut remaining = Vec::with_capacity(args.len());
    let mut ended = false;
    for argument in args {
        ended |= argument == "--";
        if !ended {
            if let Some(value) = argument.strip_prefix("--allow-private-network=") {
                allow = value == "true";
                continue;
            }
        }
        remaining.push(argument);
    }
    http_asset::set_allow_private_network(allow);
    remaining
}

fn dispatch_without_version(args: &[String]) -> Result<Vec<u8>, CommandError> {
    let args = &command_line::expand_shorthands(args);
    let Some(command) = args.first().map(String::as_str) else {
        return command_line::help().map_err(Into::into);
    };
    // `version` is a leaf command. It must not enter the protocol/source
    // stdin branch, otherwise an interactive `d2-rs version` waits forever
    // for input even though the command has no payload.
    if command == "version" {
        return run_inner(args, &[]).map_err(Into::into);
    }
    let result = match command {
        "fmt" => return run_fmt(&args[1..]),
        "validate" => run_validate(&args[1..]),
        "themes" => run_themes(&args[1..]),
        "play" => run_play(&args[1..]),
        "layout" if args.len() > 1 && !args[1].starts_with('-') => {
            // d2cli layoutCmd counts the parsed positionals, `layout` included.
            if command_line::positional_arguments(&args[1..]).len() > 1 {
                return Err(
                    command_line::usage("layout subcommand accepts at most one argument").into(),
                );
            }
            run_layout_command(&args[1..])
        }
        "layout" if args.len() == 1 => run_layout_command(&[]),
        command if SOURCE_COMMANDS.contains(&command) => {
            let input = read_stdin()?;
            run(args, &input)
        }
        _ if watch::requested(args)? => run_watch(args),
        _ => run_document_with_stdout(args, Some(&mut write_stdout)),
    };
    result.map_err(Into::into)
}

fn run_watch(args: &[String]) -> Result<Vec<u8>, String> {
    watch::run(args, |render_args| {
        let dependencies = Arc::new(Mutex::new(BTreeSet::new()));
        let _tracking = watch::track_dependencies(Arc::clone(&dependencies));
        let input_path = render_args.first().map(String::as_str);
        let rendered =
            run_document(render_args).map_err(|error| watch_error_detail(error, input_path));
        let opened = dependencies
            .lock()
            .map(|dependencies| dependencies.clone())
            .unwrap_or_default();
        (rendered, opened)
    })
}

fn watch_error_detail(error: String, input_path: Option<&str>) -> String {
    if let Some(input_path) = input_path {
        let prefix = format!(
            "err: failed to compile {}: ",
            human_path(Path::new(input_path))
        );
        if let Some(detail) = error.strip_prefix(&prefix) {
            return detail.to_owned();
        }
    }
    error
}

fn run_inner(args: &[String], input: &[u8]) -> Result<Vec<u8>, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(
            "expected a subcommand: parse, format, compile, graph, layout-source, render-svg, validate, or version"
                .into(),
        );
    };
    match command {
        "parse" => {
            serde_json::to_vec(&parse_source(input, &args[1..])?).map_err(|error| error.to_string())
        }
        "format" => format_source(input, &args[1..]),
        "compile" => compile_source(input, &args[1..]),
        "graph" => graph_source(input, &args[1..]),
        "layout-source" => layout_source(input, &args[1..]),
        "render-svg" => render_source(input, &args[1..]),
        "validate" => {
            validate_source(input, &args[1..])?;
            Ok(Vec::new())
        }
        "version" if args.len() == 1 => Ok(command_line::VERSION.as_bytes().to_vec()),
        "version" => Err(command_line::usage(
            "version subcommand accepts no arguments",
        )),
        other => Err(format!("unrecognized command: {other}")),
    }
}

fn run(args: &[String], input: &[u8]) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_inner(args, input)))
        .unwrap_or_else(|_| Err("layout engine panicked while handling the request".into()))
}

fn write_cli_error(writer: &mut impl Write, error: &str) -> std::io::Result<()> {
    // D2's xmain.mainFatal sends the complete error through cmdlog.prefixWriter.
    // Keep that line decoration at stderr, outside the error consumed by watch.
    let message = error.strip_prefix("err: ").unwrap_or(error);
    for line in message.split_terminator('\n') {
        if line.is_empty() {
            writeln!(writer, "err:")?;
        } else {
            writeln!(writer, "err: {line}")?;
        }
    }
    Ok(())
}

// Header-checked adapters to the allocator linked by libmimalloc-sys.
unsafe extern "C" {
    fn d2_rs_allocator_collect();
    fn d2_rs_allocator_set_purge_delay();
}

/// A finished TALA routing pass frees its OVG and search state on the seed
/// thread. Purge that thread's freed pages now rather than after mimalloc's
/// purge delay, so the next seed's routing does not stack on top of them.
fn release_routing_memory() {
    // SAFETY: mi_collect only walks and purges the calling thread's heap.
    unsafe { d2_rs_allocator_collect() };
}

fn main() {
    // Freed pages linger for mimalloc's default 10 ms purge delay; 2 ms keeps
    // TALA tpmjs peak RSS at 116 MB instead of 127 MB (Go: 121 MB) at no
    // measurable wall cost, while 0 ms costs about 5%.
    // SAFETY: mi_option_set only stores an option value.
    unsafe { d2_rs_allocator_set_purge_delay() };
    d2_rs_tala::set_routing_release_hook(release_routing_memory);
    // With transparent huge pages set to "always", the kernel backs
    // mimalloc's arenas with 2 MiB pages that layouts touch only sparsely,
    // inflating TALA peak RSS by up to 2x (tpmjs 498 -> 357 MB, spyre
    // 73 -> 33 MB) for a 2-4% wall-time gain. Prefer the smaller footprint.
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_THP_DISABLE only sets a per-process memory flag.
    unsafe {
        libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0);
    }
    // The external protocol requires failures to be returned as stderr text,
    // not a Rust panic report mixed into machine-readable output.
    std::panic::set_hook(Box::new(|panic| {
        if std::env::var_os("D2_RS_PANIC").is_some() {
            eprintln!("{panic}");
            if std::env::var_os("RUST_BACKTRACE").is_some() {
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
        }
    }));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = dispatch(&args);
    if watch::was_interrupted() {
        std::process::exit(1);
    }
    match result {
        Ok(output) => {
            if let Err(error) = write_stdout(&output) {
                let _ = write_cli_error(&mut std::io::stderr().lock(), &error);
                std::process::exit(1);
            }
        }
        Err(error) => {
            if let Err(record_error) = error.write_record() {
                let _ = write_cli_error(
                    &mut std::io::stderr().lock(),
                    &format!("could not write CLI error record: {record_error}"),
                );
            }
            let _ = write_cli_error(&mut std::io::stderr().lock(), error.terminal_message());
            std::process::exit(error.exit_code());
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod braket_raster_contract;

#[cfg(test)]
mod source_profile;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_compile_progress_repeats_and_stops_with_go_message() {
        let (send, receive) = std::sync::mpsc::channel();
        let progress =
            CompileProgress::start_with(std::time::Duration::from_millis(5), move || {
                let _ = send.send(COMPILE_PROGRESS_MESSAGE);
            });
        assert_eq!(
            receive.recv_timeout(std::time::Duration::from_secs(1)),
            Ok("info: compiling & running layout algorithms...")
        );
        assert_eq!(
            receive.recv_timeout(std::time::Duration::from_secs(1)),
            Ok("info: compiling & running layout algorithms...")
        );
        drop(progress);
        assert!(
            receive
                .recv_timeout(std::time::Duration::from_millis(20))
                .is_err()
        );
    }

    fn source_test_path(path: &Path) -> String {
        // d2ir uses Go path, whose separators are slashes on every host.
        #[cfg(windows)]
        return path.to_string_lossy().replace('\\', "/");
        #[cfg(not(windows))]
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn animation_file_format_is_checked_before_source_read() {
        for extension in ["png", "pdf", "pptx", "txt"] {
            let error = run_document(&[
                "--animate-interval=2".into(),
                "missing-animation-input.d2".into(),
                format!("out.{extension}"),
            ])
            .unwrap_err();
            assert_eq!(
                error,
                format!(
                    "bad usage: --animate-interval can only be used when exporting to SVG or GIF.\nYou provided: .{extension}\nRun with --help to see usage."
                )
            );
        }
    }

    #[test]
    fn fmt_check_counts_every_file_without_writing() {
        let directory =
            std::env::temp_dir().join(format!("d2-rs-fmt-count-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let inputs = [
            ("one.d2", "a ---> b"),
            ("two.d2", "x ---> y"),
            ("ok.d2", "a -> z\n"),
        ];
        let mut args = vec!["--check".into()];
        for (name, input) in inputs {
            let path = directory.join(name);
            std::fs::write(&path, input).unwrap();
            args.push(path.to_string_lossy().into_owned());
        }
        assert_eq!(
            run_fmt(&args).unwrap_err().to_string(),
            "failed to fmt: exiting with code 1: found 2 unformatted files. Run d2 fmt to fix."
        );
        assert_eq!(
            run_fmt(&args[..2]).unwrap_err().to_string(),
            "failed to fmt: exiting with code 1: found 1 unformatted file. Run d2 fmt to fix."
        );
        for (name, input) in inputs {
            assert_eq!(
                std::fs::read_to_string(directory.join(name)).unwrap(),
                input
            );
        }
        std::fs::write(directory.join("ok.d2"), "invalid: {\n").unwrap();
        assert!(
            !run_fmt(&args)
                .unwrap_err()
                .to_string()
                .contains("unformatted")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn default_svg_output_is_byte_identical_to_upstream_golden() {
        // Upstream D2 v0.9.0 e2etests/testdata/sanity/basic/dagre/sketch.exp.svg
        // (MPL-2.0), with the generator version normalized to v0.9.0.
        let golden = d2_rs_testdata::oracle_fixture!("tests/fixtures/sanity-basic-dagre-v090.svg");
        let output = run(
            &[
                "render-svg".into(),
                "--engine=dagre".into(),
                "--pad=0".into(),
                "--bundle=false".into(),
            ],
            b"a -> b\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let output = output
            .strip_prefix(r#"<?xml version="1.0" encoding="utf-8"?>"#)
            .unwrap_or(&output);
        let golden = golden
            .strip_prefix(r#"<?xml version="1.0" encoding="utf-8"?>"#)
            .unwrap();
        assert_eq!(output.trim_end(), golden.trim_end());
    }
    use d2_rs_core::LayoutError;

    #[test]
    fn nested_layout_witnesses_match_pinned_go_targets() {
        // Reduced from upstream stable/direction and real_world/jupyter_aws_eks.
        // Hashes captured with original Go D2 v0.9.0, --pad=0 --bundle=false
        // --tala-seeds=1. The target hash covers every shape and connection.
        let route =
            include_bytes!("../../../tests/regressions/e2e/dagre-nested-container-route.d2");
        let label = include_bytes!("../../../tests/regressions/e2e/nested-grid-edge-label.d2");
        let elk_container =
            include_bytes!("../../../tests/regressions/e2e/elk-nested-edge-container.d2");
        let dagre_lanes =
            include_bytes!("../../../tests/regressions/e2e/dagre-single-container-dummy-lanes.d2");
        let grid_crossing =
            include_bytes!("../../../tests/regressions/e2e/nested-grid-crossing-label.d2");
        let object_grid_crossing =
            include_bytes!("../../../tests/regressions/e2e/object-grid-crossing-label.d2");
        let compound_shape =
            include_bytes!("../../../tests/regressions/e2e/dagre-compound-shape-trace.d2");
        let nested_grid_order =
            include_bytes!("../../../tests/regressions/e2e/tala-nested-grid-routing-order.d2");
        let compound_position =
            include_bytes!("../../../tests/regressions/e2e/tala-compound-live-position.d2");
        let modifier_label =
            include_bytes!("../../../tests/regressions/e2e/dagre-modifier-label-trace.d2");
        let empty_grid_label =
            include_bytes!("../../../tests/regressions/e2e/grid-empty-label-icon.d2");
        let sql_icon_port = include_bytes!("../../../tests/regressions/e2e/elk-sql-icon-port.d2");
        let self_loop_center =
            include_bytes!("../../../tests/regressions/e2e/dagre-self-loop-published-center.d2");
        let first_constraint =
            include_bytes!("../../../tests/regressions/e2e/elk-first-constraint-order.d2");
        let opposing_loops =
            include_bytes!("../../../tests/regressions/e2e/elk-opposing-loop-labels.d2");
        let external_fanin =
            include_bytes!("../../../tests/regressions/e2e/elk-external-fanin-order.d2");
        let label_overflow =
            include_bytes!("../../../tests/regressions/e2e/elk-label-overflow-spacing.d2");
        let isolated_tree_label =
            include_bytes!("../../../tests/regressions/e2e/tala-isolated-tree-sibling-label.d2");
        let compound_label_score =
            include_bytes!("../../../tests/regressions/e2e/tala-compound-live-label-score.d2");
        let herd_after_swap =
            include_bytes!("../../../tests/regressions/e2e/tala-herd-fence-after-swap.d2");
        let horizontal_hierarchy =
            include_bytes!("../../../tests/regressions/e2e/tala-horizontal-hierarchy-transpose.d2");
        let fitted_origin =
            include_bytes!("../../../tests/regressions/e2e/dagre-fitted-origin-truncation.d2");
        let mixed_external =
            include_bytes!("../../../tests/regressions/e2e/dagre-mixed-external-anchor-order.d2");
        for (source, engine, hash) in [
            (route.as_slice(), "dagre", "147160669"),
            (label.as_slice(), "dagre", "4076045687"),
            (label.as_slice(), "elk", "3808600835"),
            (label.as_slice(), "tala", "4076045687"),
            (elk_container.as_slice(), "elk", "1902065081"),
            (dagre_lanes.as_slice(), "dagre", "2405017235"),
            (grid_crossing.as_slice(), "dagre", "358427907"),
            (grid_crossing.as_slice(), "elk", "1856303271"),
            (grid_crossing.as_slice(), "tala", "2400284554"),
            (object_grid_crossing.as_slice(), "elk", "635402051"),
            (compound_shape.as_slice(), "dagre", "2428849042"),
            (nested_grid_order.as_slice(), "tala", "1710228742"),
            (compound_position.as_slice(), "tala", "2101581790"),
            (modifier_label.as_slice(), "dagre", "2721557747"),
            (empty_grid_label.as_slice(), "dagre", "2599487348"),
            (sql_icon_port.as_slice(), "elk", "1787285706"),
            (self_loop_center.as_slice(), "dagre", "1485139401"),
            (first_constraint.as_slice(), "elk", "1486478533"),
            (opposing_loops.as_slice(), "elk", "3585752355"),
            (external_fanin.as_slice(), "elk", "2589457099"),
            (label_overflow.as_slice(), "elk", "882734882"),
            (isolated_tree_label.as_slice(), "tala", "1738344229"),
            (compound_label_score.as_slice(), "tala", "4236667171"),
            (herd_after_swap.as_slice(), "tala", "3938423341"),
            (horizontal_hierarchy.as_slice(), "tala", "1608485685"),
            (fitted_origin.as_slice(), "dagre", "4180049586"),
            (mixed_external.as_slice(), "dagre", "4222133879"),
        ] {
            let output = run(
                &[
                    "render-svg".into(),
                    format!("--engine={engine}"),
                    "--pad=0".into(),
                    "--bundle=false".into(),
                    "--seeds=1".into(),
                ],
                source,
            )
            .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(&format!("class=\"d2-{hash} d2-svg\"")),
                "{engine}: {hash}"
            );
        }
    }

    fn normalize_pdf_timestamps(pdf: &[u8]) -> Vec<u8> {
        let mut normalized = pdf.to_vec();
        for marker in [b"/CreationDate (D:".as_slice(), b"/ModDate (D:"] {
            let Some(start) = normalized
                .windows(marker.len())
                .position(|window| window == marker)
                .map(|offset| offset + marker.len())
            else {
                continue;
            };
            let Some(length) = normalized[start..].iter().position(|byte| *byte == b')') else {
                continue;
            };
            normalized[start..start + length].fill(b'0');
        }
        normalized
    }

    #[test]
    fn scene_errors_keep_source_format_and_board_context() {
        let error = "scene: shape \"asset\" SVG asset: d2svgimport: asset.svg: element <text> has unsupported attribute \"x\"";
        for board in [0, 1, 3] {
            for format in [
                ExportFormat::Png,
                ExportFormat::Svg,
                ExportFormat::Gif,
                ExportFormat::Pdf,
                ExportFormat::Pptx,
            ] {
                let actual = super::format_scene_error(format, board, error.into());
                let expected = match format {
                    ExportFormat::Gif => format!("GIF board {board} scene: {error}"),
                    ExportFormat::Pdf | ExportFormat::Pptx => {
                        format!("paged board {board} scene: {error}")
                    }
                    _ => error.into(),
                };
                assert_eq!(actual, expected);
                assert_eq!(
                    super::format_scene_error(
                        format,
                        board,
                        "could not encode PNG: invalid canvas".into()
                    ),
                    "could not encode PNG: invalid canvas"
                );
                let frame_error = "d2raster: frame width 34000 exceeds limit 32768";
                assert_eq!(
                    super::format_scene_error(format, board, frame_error.into()),
                    if matches!(format, ExportFormat::Pdf | ExportFormat::Pptx) {
                        format!("paged board {board} frame: {frame_error}")
                    } else {
                        frame_error.into()
                    }
                );
                assert_eq!(
                    super::format_scene_error(format, board, actual.clone()),
                    actual
                );
            }
        }
    }

    #[test]
    fn playground_encoding_matches_v090_urlenc() {
        let encoded = encode_play_script(b"a -> b\n").unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE
            .decode(encoded)
            .unwrap();
        let mut decoder = flate2::read::DeflateDecoder::new(bytes.as_slice());
        let mut decoded = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut decoded).unwrap();
        assert_eq!(decoded, "a -> b\n");
    }

    #[test]
    fn play_requires_one_source_argument() {
        let error = dispatch(&["play".into()]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("play must be passed one argument")
        );
    }

    #[test]
    fn version_reports_the_pinned_d2_version() {
        assert_eq!(run(&["version".into()], &[]).unwrap(), b"v0.9.0\n");
    }

    #[test]
    fn dispatch_version_does_not_require_stdin() {
        assert_eq!(dispatch(&["version".into()]).unwrap(), b"v0.9.0\n");
    }

    #[test]
    fn dispatch_layout_without_input_lists_bundled_engines() {
        let output = String::from_utf8(dispatch(&["layout".into()]).unwrap()).unwrap();
        assert!(output.contains("dagre (built-in)"));
        assert!(output.contains("elk (built-in)"));
        assert!(output.contains("tala (built-in)"));
    }

    #[test]
    fn font_flags_preserve_per_face_selection() {
        let (source_args, paths) = extract_font_option(&[
            "--font-regular=regular.ttf".into(),
            "--font-italic".into(),
            "italic.ttf".into(),
            "--path=diagram.d2".into(),
        ])
        .unwrap();
        assert_eq!(source_args, ["--path=diagram.d2"]);
        assert_eq!(paths.regular.as_deref(), Some("regular.ttf"));
        assert_eq!(paths.italic.as_deref(), Some("italic.ttf"));
        assert_eq!(paths.bold, None);

        let (_, all_paths) = extract_font_option(&["--font=one.ttf".into()]).unwrap();
        assert_eq!(all_paths.regular.as_deref(), Some("one.ttf"));
        assert_eq!(all_paths.mono_italic.as_deref(), Some("one.ttf"));
    }

    #[test]
    fn svg_embeds_custom_faces_without_leaking_into_later_renders() {
        use d2_rs_text::d2fonts::{FontFamily, FontStyle};
        let custom = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../d2-rs-text/assets/SourceCodePro-Regular.ttf");
        let expected = FontFamily::SourceCodePro
            .font(FontStyle::Regular)
            .get_encoded_subset("custom")
            .unwrap();
        for (flag, style) in [
            ("font-regular", "style.bold: false"),
            ("font-bold", "style.bold: true"),
            ("font-italic", "style.bold: false; style.italic: true"),
            ("font-mono", "style.bold: false; style.font: mono"),
            ("font-mono-bold", "style.bold: true; style.font: mono"),
            (
                "font-mono-italic",
                "style.bold: false; style.italic: true; style.font: mono",
            ),
        ] {
            let source = format!("a: custom {{ {style} }}\n");
            let svg = render_source(
                source.as_bytes(),
                &[format!("--{flag}={}", custom.display())],
            )
            .unwrap();
            assert!(
                String::from_utf8(svg).unwrap().contains(&expected),
                "{flag}"
            );
        }
        let ordinary = render_source(b"a: custom\n", &[]).unwrap();
        assert!(!String::from_utf8(ordinary).unwrap().contains(&expected));
    }

    #[test]
    fn hash_prefixed_markdown_reaches_unsupported_html_diagnostic() {
        // Goldmark parser/atx_heading.go rejects hash-prefixed words as ATX
        // headings. D2 textmeasure.LayoutMarkdown then rejects the HTML node.
        // The unchecked fallback ruler must also consume hash-prefixed words.
        let measurer = load_graph_measurer_with_mode(&FontPaths::default(), false)
            .unwrap()
            .unwrap();
        let sources = ["##` a", "#word", "######x", "#", "#######"]
            .map(|line| format!("x: |md\n# title\n{line}\n<r />\n|"));
        for source in sources.into_iter().chain(["y:|md #`<r/>|".to_owned()]) {
            let (_, markdown) = source.split_once("md").unwrap();
            let markdown = markdown.strip_suffix('|').unwrap();
            let dimensions = measurer.markdown_dimensions(markdown, 16, false, false, false);
            assert!(dimensions.width.is_finite() && dimensions.height.is_finite());
            for engine in ["dagre", "elk", "tala"] {
                let error =
                    render_source(source.as_bytes(), &[format!("--engine={engine}")]).unwrap_err();
                assert_eq!(
                    error, "native Markdown SVG does not support HTML element <r>",
                    "{engine} {source:?}: {error}"
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn markdown_font_family_dimensions_match_all_original_go_layout_controls() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-render"),
            "tests/fixtures/markdown-font-layout-source-v090.json",
        ))
        .unwrap();
        for row in controls.as_array().unwrap() {
            let measurer =
                load_graph_measurer_with_mode(&FontPaths::default(), row["family"] == "HandDrawn")
                    .unwrap()
                    .unwrap();
            let source = row["source"].as_str().unwrap();
            let size = row["size"].as_u64().unwrap() as u32;
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let actual = if row["family"] == "SourceCodePro" {
                    compiler::GraphMeasurer::measure_mono_styled_markdown_label_at_size(
                        &measurer, source, size, bold, italic,
                    )
                } else {
                    compiler::GraphMeasurer::measure_styled_markdown_label_at_size(
                        &measurer, source, size, bold, italic,
                    )
                };
                assert_eq!(actual.width, row["width"].as_f64().unwrap(), "width {row}");
                assert_eq!(
                    actual.height,
                    row["height"].as_f64().unwrap(),
                    "height {row}"
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn markdown_heading_family_dimensions_match_all_original_go_layout_controls() {
        let controls: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-render"),
            "tests/fixtures/markdown-heading-layout-source-v090.json",
        ))
        .unwrap();
        for row in controls.as_array().unwrap() {
            let measurer =
                load_graph_measurer_with_mode(&FontPaths::default(), row["family"] == "HandDrawn")
                    .unwrap()
                    .unwrap();
            let source = row["source"].as_str().unwrap();
            let size = row["size"].as_u64().unwrap() as u32;
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let actual = if row["family"] == "SourceCodePro" {
                    compiler::GraphMeasurer::measure_mono_styled_markdown_label_at_size(
                        &measurer, source, size, bold, italic,
                    )
                } else {
                    compiler::GraphMeasurer::measure_styled_markdown_label_at_size(
                        &measurer, source, size, bold, italic,
                    )
                };
                assert_eq!(actual.width, row["width"].as_f64().unwrap(), "width {row}");
                assert_eq!(
                    actual.height,
                    row["height"].as_f64().unwrap(),
                    "height {row}"
                );
            }
        }
    }

    #[test]
    fn markdown_table_dimensions_match_v090_native_box() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "| Status | Count |\n| ------ | ----- |\n| Done | 42 |",
            16,
        );
        assert_eq!(dimensions.width, 142.0);
        assert_eq!(dimensions.height, 76.0);
    }

    #[test]
    fn markdown_strikethrough_dimensions_exclude_delimiters() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "before ~~strikethrough~~ after",
            16,
        );
        assert_eq!(dimensions.width, 174.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn markdown_uri_autolink_dimensions_exclude_angle_brackets() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "visit <https://example.com> now",
            16,
        );
        assert_eq!(dimensions.width, 205.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn markdown_email_autolink_dimensions_exclude_angle_brackets() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "email <test@example.com> now",
            16,
        );
        assert_eq!(dimensions.width, 203.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn markdown_nested_emphasis_keeps_bold_italic_widths() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "***bold italic*** and ~~**both**~~",
            16,
        );
        assert_eq!(dimensions.width, 137.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn markdown_raw_inline_tags_match_native_text_width() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "<b>bold</b> <em>italic</em> <s>gone</s> <strike>old</strike>",
            16,
        );
        assert_eq!(dimensions.width, 129.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn markdown_images_do_not_enter_native_text_width() {
        let measurer = load_graph_measurer_with_paths(&FontPaths::default())
            .expect("font loading should not fail")
            .expect("bundled fonts should be available");
        let dimensions = compiler::GraphMeasurer::measure_markdown_label_at_size(
            &measurer,
            "before ![build passing](https://example.invalid/status.svg) after",
            16,
        );
        assert_eq!(dimensions.width, 83.0);
        assert_eq!(dimensions.height, 24.0);
    }

    #[test]
    fn themes_match_pinned_catalog_order_and_ids() {
        let output = String::from_utf8(dispatch(&["themes".into()]).unwrap()).unwrap();
        assert!(output.starts_with("Available themes:\nLight:\n- Neutral Default: 0\n"));
        assert!(output.contains("- C4: 303\nDark:\n- Dark Mauve: 200\n"));
        assert!(output.ends_with("- Dark Flagship Terrastruct: 201\n"));
    }

    #[test]
    fn themes_ignores_further_arguments_like_d2() {
        assert_eq!(
            run_themes(&["extra".into()]).unwrap(),
            run_themes(&[]).unwrap()
        );
    }

    #[test]
    fn parse_returns_a_machine_readable_ast() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["parse".into()], b"a: b\n").unwrap()).unwrap();
        assert_eq!(output["range"], "-,0:0:0-1:0:5");
        assert_eq!(
            output["nodes"][0]["map_key"]["value"]["unquoted_string"]["value"][0]["string"],
            "b"
        );
    }

    #[test]
    fn format_accepts_source_path_and_utf16_options() {
        let output = run(
            &[
                "format".into(),
                "--path=diagram.d2".into(),
                "--utf16".into(),
            ],
            b"a: b\n",
        )
        .unwrap();
        assert_eq!(output, b"a: b\n");
    }

    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn source_compilation_keeps_all_ir_diagnostics() {
        let fixtures: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-language"),
            "tests/fixtures/public-compiler-diagnostics-v090.json",
        ))
        .unwrap();
        let mut compared = 0;
        for (request, source) in fixtures["requests"]
            .as_array()
            .unwrap()
            .iter()
            .zip(fixtures["source"].as_array().unwrap())
        {
            let hex = request["hex"].as_str().unwrap();
            let input = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            // Imported inputs use a VFS in this fixture. Its exact errors are
            // checked at the language and WASM boundaries; CLI uses OS files.
            if input.contains(&b'@') {
                continue;
            }
            for command in ["compile", "graph"] {
                let mut args = vec![command.to_owned(), "--path=case.d2".to_owned()];
                if request["utf16"].as_bool().unwrap() {
                    args.push("--utf16".into());
                }
                let expected = source["error"].as_str().unwrap();
                // compile exposes IR; graph additionally validates attributes.
                if command == "compile"
                    && std::str::from_utf8(&input).unwrap().starts_with("a.shape:")
                {
                    continue;
                }
                assert_eq!(
                    run(&args, &input).unwrap_err(),
                    expected,
                    "{request} {command}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 26);
    }

    #[test]
    fn compile_returns_ordered_ir_summary() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["compile".into()], b"a -> b\n").unwrap()).unwrap();
        assert!(output["errors"].as_array().unwrap().is_empty());
        let nodes = output["nodes"].as_array().unwrap();
        assert!(nodes.iter().any(|node| node["kind"] == "edge"));
        assert!(nodes.iter().any(|node| node["path"] == "root.a"));
    }

    #[test]
    fn compile_loads_relative_filesystem_imports() {
        let path = format!(
            "{}/tests/fixtures/import_main.d2",
            env!("CARGO_MANIFEST_DIR")
        );
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["compile".into(), format!("--path={path}")],
                b"...@import_lib\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            output["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| { node["path"] == "root.imported" })
        );
    }

    #[test]
    fn compile_resolves_markdown_substitutions() {
        let output = run(
            &["compile".into()],
            b"vars: {x: value}; a: |md hello ${x} |",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("value"));
        assert!(!output.contains("${x}"));
    }

    #[test]
    fn graph_lowers_fields_and_edges_to_the_measured_model() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"a: hello; a -> b\n").unwrap())
                .unwrap();
        assert_eq!(output["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(output["edges"].as_array().unwrap().len(), 1);
        assert_eq!(output["edges"][0]["source"], 0);
        assert_eq!(output["edges"][0]["target"], 1);
    }

    #[test]
    fn graph_qualifies_nested_edges_and_expands_globs() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"a: { b; c; b -> c }\n").unwrap())
                .unwrap();
        assert_eq!(output["edges"][0]["source"], 1);
        assert_eq!(output["edges"][0]["target"], 2);
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"a: { x; y }; a.* -> c\n").unwrap())
                .unwrap();
        assert_eq!(output["edges"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn graph_keeps_board_children_out_of_the_base_graph() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"layers: {one: {a -> b}}\n").unwrap())
                .unwrap();
        let nodes = output["nodes"].as_array().unwrap();
        assert!(nodes.is_empty());
        assert!(output["edges"].as_array().unwrap().is_empty());
    }

    #[test]
    fn graph_excludes_variable_scope_edges_and_fields() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"vars: {x -> y; name: world}; a: hello\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(output["edges"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn graph_lowers_repeated_variable_maps_at_their_use_sites() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                br#"vars: {
  template: {
    class: cluster
    child: {class: text}
  }
}
container: {
  first: ${template}
  second: ${template}
}
"#,
            )
            .unwrap(),
        )
        .unwrap();
        let nodes = output["nodes"].as_array().unwrap();
        assert!(
            nodes
                .iter()
                .any(|node| node["external_id"] == "container.first")
        );
        assert!(
            nodes
                .iter()
                .any(|node| node["external_id"] == "container.second")
        );
        assert!(nodes.iter().all(|node| {
            node["external_id"]
                .as_str()
                .is_some_and(|id| !id.starts_with("vars."))
        }));
    }

    #[test]
    fn graph_applies_classes_without_promoting_class_fields_to_nodes() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                br#"classes: {
  diamond: {shape: diamond; style.fill: red}
  wide: {width: 120}
}
first: {class: [diamond; wide]}
second: {class: diamond}
"#,
            )
            .unwrap(),
        )
        .unwrap();
        let nodes = output["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().all(|node| {
            node["external_id"]
                .as_str()
                .is_some_and(|id| !id.ends_with(".class"))
        }));
        assert_eq!(nodes[0]["shape"], "diamond");
        assert_eq!(nodes[0]["size"]["width"], 120.0);
        assert_eq!(nodes[1]["shape"], "diamond");
    }

    #[test]
    fn graph_accepts_a_caller_supplied_measurer() {
        fn fixed_size(_: &str) -> Size {
            Size {
                width: 123.0,
                height: 45.0,
            }
        }
        let parsed = parse("-", b"a: hello\n", None);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph_with_measurer(&compiler, root, &fixed_size).unwrap();
        let node = graph.node(d2_rs_core::NodeId(0)).unwrap();
        assert_eq!(node.size.width, 123.0);
        assert_eq!(node.size.height, 45.0);
    }

    #[test]
    fn graph_passes_source_font_size_to_the_measurer() {
        struct SizeAware;
        impl compiler::GraphMeasurer for SizeAware {
            fn measure(&self, _: &str) -> Size {
                Size {
                    width: 10.0,
                    height: 11.0,
                }
            }

            fn measure_at_size(&self, _: &str, font_size: u32) -> Size {
                Size {
                    width: f64::from(font_size),
                    height: f64::from(font_size) + 1.0,
                }
            }
        }

        let parsed = parse("-", b"a: {style.font-size: 24}\n", None);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph_with_measurer(&compiler, root, &SizeAware).unwrap();
        let node = graph.node(d2_rs_core::NodeId(0)).unwrap();
        assert_eq!(node.size.width, 24.0);
        assert_eq!(node.size.height, 25.0);
    }

    #[test]
    fn graph_retains_declared_dimensions_and_content_shape_labels() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"text: {shape: text; width: 80; height: 30}\nclass_node: {shape: class}\n",
            )
            .unwrap(),
        )
        .unwrap();
        // `class` is a reserved attribute, so use an actual object name.
        // Pristine d2lib.Compile emits two objects and these header metrics.
        assert_eq!(output["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(output["nodes"][1]["external_id"], "class_node");
        assert_eq!(output["nodes"][0]["declared_size"]["width"], 80.0);
        assert!(output["nodes"][0]["label_size"].is_object());
        // Class headers participate in the same measured graph contract as
        // SQL-table headers; their 24 px heading metrics are retained for
        // downstream layout/render stages.
        assert!(output["nodes"][1]["label_size"].is_object());
        assert_eq!(output["nodes"][1]["label_size"]["width"], 142.0);
        assert_eq!(output["nodes"][1]["label_size"]["height"], 31.0);
        assert_eq!(output["nodes"][1]["font_size"], 24);
    }

    #[test]
    fn graph_class_dimensions_match_v090_empty_and_member_cases() {
        let cases = [
            (b"foo: {shape: class}\n".as_slice(), 145.0, 92.0),
            (b"foo: {shape: class; name}\n".as_slice(), 146.0, 138.0),
            (
                b"foo: {shape: class; +name: string; -id: int}\n".as_slice(),
                219.0,
                184.0,
            ),
            (
                b"foo: {shape: class; style.font-size: 30; +name: string}\n".as_slice(),
                277.0,
                174.0,
            ),
        ];
        for (source, width, height) in cases {
            let output: serde_json::Value =
                serde_json::from_slice(&run(&["graph".into()], source).unwrap()).unwrap();
            assert_eq!(output["nodes"][0]["size"]["width"], width);
            assert_eq!(output["nodes"][0]["size"]["height"], height);
        }
        let explicit: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"foo: {shape: class; style.font-size: 30; +name: string}\n",
            )
            .unwrap(),
        )
        .unwrap();
        // Source graph JSON preserves the explicit base font size. The
        // structured class header adds four pixels only at render time.
        assert_eq!(explicit["nodes"][0]["font_size"], 30);
    }

    #[test]
    fn graph_translates_d2_port_spread_rules() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"labelled: hello\nimage: {shape: image; icon: https://example.com/image.svg}\ncontainer: {child}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"][0]["port_spread"], 12.0);
        assert_eq!(output["nodes"][1]["port_spread"], 5.0);
        assert_eq!(output["nodes"][2]["port_spread"], 0.0);
    }

    #[test]
    fn graph_translates_shape_and_icon_container_insets() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"database: {shape: cylinder; icon: https://example.com/icon.svg; child}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"][0]["content_insets"]["top"], 108.0);
        // Object.SpacingOpt reserves no inside lane for an unresolved icon.
        // Dagre selects outside-top-left; TALA applies its own 74px floor.
        assert_eq!(output["nodes"][0]["content_insets"]["right"], 60.0);
        assert_eq!(output["nodes"][0]["content_insets"]["bottom"], 84.0);
        assert_eq!(output["nodes"][0]["content_insets"]["left"], 60.0);
        assert_eq!(output["nodes"][0]["has_icon"], true);
        assert!(output["nodes"][0]["icon_position"].is_null());
    }

    #[test]
    fn graph_keeps_sql_table_columns_out_of_layout_hierarchy() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"users: {shape: sql_table; id: int; name: string}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(output["table_column_counts"][0], 2);
    }

    #[test]
    fn graph_translates_root_hierarchy_shape() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"shape: hierarchy\na\nb\n").unwrap())
                .unwrap();
        assert_eq!(output["root_hierarchy"], true);
    }

    #[test]
    fn graph_marks_synthetic_sequence_root_for_in_process_dispatch() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"shape: sequence_diagram\na; b\na -> b\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["root_sequence_diagram"], true);
    }

    #[test]
    fn graph_passes_edge_font_size_to_the_measurer() {
        struct SizeAware;
        impl compiler::GraphMeasurer for SizeAware {
            fn measure(&self, _: &str) -> Size {
                Size {
                    width: 10.0,
                    height: 11.0,
                }
            }

            fn measure_at_size(&self, _: &str, font_size: u32) -> Size {
                Size {
                    width: f64::from(font_size),
                    height: f64::from(font_size) + 1.0,
                }
            }
        }

        let parsed = parse("-", b"a -> b: edge {style.font-size: 24}\n", None);
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph_with_measurer(&compiler, root, &SizeAware).unwrap();
        assert_eq!(
            graph.edge_label(d2_rs_core::EdgeId(0)).unwrap().size.width,
            24.0
        );
        assert_eq!(
            graph.edge_label(d2_rs_core::EdgeId(0)).unwrap().size.height,
            25.0
        );
    }

    #[test]
    fn graph_translates_source_constraints_and_metadata() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"direction: right\na: { label: A; shape: circle; width: 80; height: 80; top: 20; left: 30; near: b; label-position: OUTSIDE_TOP_LEFT; icon-position: INSIDE_BOTTOM_RIGHT }\nb: B\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["direction"], "right");
        assert_eq!(output["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(output["nodes"][0]["external_id"], "a");
        assert_eq!(output["nodes"][0]["shape"], "circle");
        assert_eq!(output["nodes"][0]["size"]["width"], 80.0);
        assert_eq!(output["nodes"][0]["locked_position"]["x"], 30.0);
        assert_eq!(output["nodes"][0]["near"], 1);
        assert_eq!(output["nodes"][0]["label_position"], "OUTSIDE_TOP_LEFT");
        // SetDimensions materializes a position only when an icon exists.
        assert!(output["nodes"][0]["icon_position"].is_null());
        assert_eq!(
            output["nodes"][0]["d2_authored_geometry"]["attributes"]["icon_position"],
            serde_json::json!(b"INSIDE_BOTTOM_RIGHT".to_vec())
        );
        assert_eq!(output["nodes"][0]["content_alignment"], "Center");
    }

    #[test]
    fn graph_translates_nested_d2_label_and_icon_positions() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"a: {label: {near: outside-top-left}; icon: https://example.com/icon.svg {near: bottom-right}}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"][0]["label_position"], "OUTSIDE_TOP_LEFT");
        assert_eq!(output["nodes"][0]["icon_position"], "INSIDE_BOTTOM_RIGHT");
    }

    #[test]
    fn graph_reserves_explicit_external_label_space() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"a: {label: hello; label-position: OUTSIDE_TOP_RIGHT}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"][0]["external_label"]["side"], "Top");
        assert_eq!(output["nodes"][0]["external_label"]["alignment"], "End");
        assert_eq!(output["nodes"][0]["external_label"]["reserve_space"], true);
    }

    #[test]
    fn graph_uses_d2_automatic_external_label_preferences() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"image: {shape: image; icon: https://example.com/image.svg}\nperson: {shape: person}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["nodes"][0]["external_label"]["side"], "Bottom");
        assert_eq!(output["nodes"][0]["external_label"]["automatic"], true);
        assert_eq!(output["nodes"][1]["external_label"]["side"], "Bottom");
        assert_eq!(output["nodes"][1]["external_label"]["automatic"], true);
    }

    #[test]
    fn graph_translates_constant_near_position() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"top_left: {near: top-left}\ntop_center: {near: top-center}\ntop_right: {near: top-right}\ncenter_left: {near: center-left}\ncenter_right: {near: center-right}\nbottom_left: {near: bottom-left}\nbottom_center: {near: bottom-center}\nbottom_right: {near: bottom-right}\n",
            )
            .unwrap(),
        )
        .unwrap();
        let positions = output["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["canvas_position"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                "top_left",
                "top_center",
                "top_right",
                "center_left",
                "center_right",
                "bottom_left",
                "bottom_center",
                "bottom_right",
            ]
        );
    }

    #[test]
    fn graph_marks_transparent_source_nodes_invisible() {
        let parsed = parse(
            "-",
            b"a: {style.fill: transparent; style.stroke: transparent; label: \"\"}\n",
            None,
        );
        let (compiler, root) = compiler::compile_ir(Shared::new(parsed.map)).unwrap();
        let graph = compiler::lower_graph(&compiler, root).unwrap();
        assert!(graph.node_is_invisible(d2_rs_core::NodeId(0)));
    }

    #[test]
    fn graph_carries_edge_label_text_and_dimensions() {
        let output: serde_json::Value =
            serde_json::from_slice(&run(&["graph".into()], b"a -> b: hello\n").unwrap()).unwrap();
        assert_eq!(output["edge_labels"][0]["text"], "hello");
        assert_eq!(output["edge_labels"][0]["percentage"], 0.0);
    }

    #[test]
    fn edge_label_dimensions_use_d2s_default_italic_face() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(&["graph".into()], b"a -> b: what did you have for lunch?\n").unwrap(),
        )
        .unwrap();
        // D2's Edge.Text marks labels italic even without an explicit style;
        // Source Sans Pro Italic measures this v0.9.0 witness at 187px.
        assert_eq!(output["edge_labels"][0]["size"]["width"], 187.0);
        assert_eq!(output["edge_labels"][0]["size"]["height"], 21.0);
    }

    #[test]
    fn graph_carries_source_edge_arrowheads_and_styles() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"a -> b: {source-arrowhead.shape: diamond; target-arrowhead.label: target; style.stroke: red; style.border-radius: 5}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["edge_arrowheads"][0]["source"], "diamond");
        assert_eq!(
            output["edge_arrowhead_labels"][0]["target"]["text"],
            "target"
        );
        assert_eq!(output["edge_styles"][0]["stroke"], "red");
        assert_eq!(output["edge_styles"][0]["border_radius"], "5");
    }

    #[test]
    fn graph_inherits_edge_styles_from_classes_with_explicit_values_winning() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["graph".into()],
                b"classes.green.style: {stroke: green; stroke-width: 5; border-radius: 8}\na -> b: {class: green; style.stroke-width: 3}\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["edge_styles"][0]["stroke"], "green");
        assert_eq!(output["edge_styles"][0]["stroke_width"], "3");
        assert_eq!(output["edge_styles"][0]["border_radius"], "8");
    }

    #[test]
    fn graph_rejects_unsupported_edge_label_position_and_percentage_keys() {
        let error = run(
            &["graph".into()],
            b"a -> b: {label: link; label-position: outside-top-right; label-percentage: 0.25}\n",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error
                .matches("edge map keys must be reserved keywords")
                .count(),
            2
        );
        // Complete ranges and order for this exact input are independently
        // checked against pristine Go in the language diagnostic fixture.
    }

    #[test]
    fn layout_source_runs_tala_in_process_with_requested_seeds() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(&["layout-source".into(), "--seeds=7".into()], b"a -> b\n").unwrap(),
        )
        .unwrap();
        assert_eq!(output["boxes"].as_object().unwrap().len(), 2);
        assert_eq!(output["routes"].as_object().unwrap().len(), 1);
        assert_eq!(output["report"]["selected_seed"], 7);
    }

    #[test]
    fn tala_step_sequences_export_only_surviving_edges() {
        // Minimal stable/investigate + regression/nested_steps witness.
        // Go patch.go:215–239 removes the defining edge, retaining the next
        // edge's identity, label, arrowheads and style.
        for source in [
            "a.shape: step\nb.shape: step\na -> b\nb -> c: remains\n",
            "grid-columns: 1\ncell: {a.shape: step; b.shape: step; a -> b; b -> c: remains}\n",
        ] {
            let (graph, result, labels, styles) = source_layout(
                source.as_bytes(),
                &["--engine=tala".into(), "--seeds=1".into()],
            )
            .unwrap();
            assert_eq!(result.removed_edges, [d2_rs_core::EdgeId(0)].into());
            assert!(!result.routes.contains_key(&d2_rs_core::EdgeId(0)));
            assert!(result.routes[&d2_rs_core::EdgeId(1)].len() >= 2);
            let json = d2_rs_render::target_connections_metadata(&graph, &result, &styles).unwrap();
            let connections = json.as_array().unwrap();
            assert_eq!(connections.len(), 1);
            assert_eq!(connections[0]["label"], "remains");
            assert!(
                connections[0]["id"]
                    .as_str()
                    .unwrap()
                    .ends_with("(b -> c)[0]")
            );
            d2_rs_render::render_d2svg(
                &graph,
                &result,
                &labels,
                &styles,
                &d2_rs_render::RenderOptions::default(),
                false,
            )
            .unwrap();
            let wire = serde_json::to_value(&result).unwrap();
            let restored: d2_rs_core::LayoutResult = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(restored.removed_edges, result.removed_edges);
            let mut legacy = wire;
            legacy.as_object_mut().unwrap().remove("removed_edges");
            let restored: d2_rs_core::LayoutResult = serde_json::from_value(legacy).unwrap();
            assert!(restored.removed_edges.is_empty());
        }
    }

    #[test]
    fn layout_source_can_select_dagre_without_a_plugin_process() {
        let output: serde_json::Value = serde_json::from_slice(
            &run(
                &["layout-source".into(), "--engine=dagre".into()],
                b"a -> b\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(output["boxes"].as_object().unwrap().len(), 2);
        assert_eq!(output["routes"].as_object().unwrap().len(), 1);
        assert_eq!(output["report"]["selected_seed"], 1);
    }

    #[test]
    fn dagre_restores_sequence_projection_routes_and_actor_boxes() {
        let (graph, result, _, _) = source_layout(
            b"sequence: {\n  shape: sequence_diagram\n  first -> second: message\n}\n",
            &["--engine=dagre".into()],
        )
        .unwrap();
        let sequence = graph
            .nodes()
            .find_map(|(id, node)| (node.external_id == "sequence").then_some(id))
            .expect("sequence root");
        let first = graph
            .nodes()
            .find_map(|(id, node)| (node.external_id == "sequence.first").then_some(id))
            .expect("first sequence actor");
        let second = graph
            .nodes()
            .find_map(|(id, node)| (node.external_id == "sequence.second").then_some(id))
            .expect("second sequence actor");
        let (edge_id, _) = graph
            .edges()
            .find(|(_, edge)| edge.source == first && edge.target == second)
            .expect("sequence message edge");
        assert!(result.boxes.contains_key(&first));
        assert!(result.boxes.contains_key(&second));
        assert!(result.boxes[&sequence].size.width > 200.0);
        assert!(result.boxes[&sequence].size.height > 200.0);
        assert_eq!(
            result.node_labels[&sequence].position,
            d2_rs_core::LabelPosition::InsideTopCenter
        );
        assert!(
            result.routes.contains_key(&edge_id),
            "Dagre's parent projection must restore the locally laid out sequence message"
        );
    }

    #[test]
    fn elk_rejects_object_near_with_upstream_error() {
        let error = source_layout(b"a\nb: {near: a}\n", &["--engine=elk".into()]).unwrap_err();
        assert!(
            error.contains(
                "Object \"b\" has \"near\" set to another object, but layout engine \"elk\""
            ),
            "{error}"
        );
    }

    #[test]
    fn elk_rejects_locked_positions_with_upstream_error() {
        let error =
            source_layout(b"a: {left: 120; top: 110}\nb\n", &["--engine=elk".into()]).unwrap_err();
        assert!(
            error.contains(
                "Object \"a\" has attribute \"top\" and/or \"left\" set, but layout engine \"elk\""
            ),
            "{error}"
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_tala_seed_validation_follows_actual_core_invocations() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-wasm"),
            "tests/fixtures/tala-deferred-seeds-source-v090.json",
        ))
        .unwrap();
        // The CLI rejects an empty --seeds flag in its own argument parser;
        // the empty caller-list cases exercise the WASM/library API instead.
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| !case["input"]["seeds"].as_array().unwrap().is_empty())
            .collect();
        assert_eq!(cases.len(), 50);
        for case in cases {
            let input = &case["input"];
            let result = source_layout(
                input["source"].as_str().unwrap().as_bytes(),
                &["--engine=tala".into(), "--seeds=1".into()],
            );
            assert_eq!(
                result.err().unwrap_or_default(),
                case["error"].as_str().unwrap(),
                "{}",
                input["name"]
            );
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn source_tala_seed_config_matches_original_engine_boundary() {
        let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-wasm"),
            "tests/fixtures/tala-seed-config-source-v090.json",
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 72);
        for case in cases {
            let input = &case["input"];
            let mut args = vec![format!("--engine={}", input["engine"].as_str().unwrap())];
            if let Some(seeds) = input["seeds"].as_array() {
                args.push(format!(
                    "--seeds={}",
                    seeds
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            let result = source_layout(input["source"].as_str().unwrap().as_bytes(), &args);
            let error = case["error"].as_str().unwrap();
            if !error.is_empty() {
                assert_eq!(result.unwrap_err(), error, "{}", input["name"]);
            } else {
                let (_, layout, _, _) = result.unwrap();
                if !case["seeds"].is_null() {
                    assert_eq!(
                        serde_json::json!(
                            layout
                                .report
                                .candidates
                                .iter()
                                .map(|candidate| candidate.seed)
                                .collect::<Vec<_>>()
                        ),
                        case["seeds"],
                        "{}",
                        input["name"]
                    );
                }
            }
        }
    }

    #[test]
    fn layout_source_hydrates_layout_and_seed_from_d2_config() {
        let input = b"vars: {d2-config: {layout-engine: tala; data: {tala-seeds: [7]}}}\na -> b\n";
        let from_config = source_layout(input, &[]).unwrap();
        let explicit =
            source_layout(b"a -> b\n", &["--engine=tala".into(), "--seeds=7".into()]).unwrap();
        assert_eq!(
            serde_json::to_value(&from_config.1).unwrap(),
            serde_json::to_value(&explicit.1).unwrap()
        );
        assert_eq!(from_config.1.report.selected_seed, 7);
    }

    #[test]
    fn source_config_rejects_invalid_tala_seed_data() {
        let input = b"vars: {d2-config: {data: {tala-seeds: [nope]}}}\na\n";
        let error = source_layout(input, &[]).unwrap_err();
        assert!(error.contains("invalid tala seed at index 0"), "{error}");
    }

    #[test]
    fn source_config_rejects_invalid_theme_override_after_ir_compilation() {
        let input = b"vars: {d2-config: {theme-overrides: {B1: potato}}}\na\n";
        let error = source_config(input, "colors.d2").unwrap_err();
        assert_eq!(
            error,
            "colors.d2:1:38: expected \"B1\" to be a valid named color (\"orange\") or a hex code (\"#f0ff3a\")"
        );
    }

    #[test]
    fn compiled_target_metadata_preserves_board_labels_and_target_forms() {
        let input = br#"label: document
a -> b
layers: {
  bare: {a -> b}
  titled: {label: title; a -> b}
  empty: {label: ""; a -> b}
  "odd.name": {a -> b}
}
"#;
        let (compiler, root) = compile_board_source(input, "boards.d2").unwrap();
        for (target, label, config) in [
            ("", "document", true),
            ("*", "document", true),
            ("bare", "bare", false),
            ("layers.bare", "bare", false),
            ("bare.*", "bare", false),
            ("titled", "title", false),
            ("empty", "", false),
            ("\"odd.name\"", "odd.name", false),
        ] {
            for args in [
                vec![format!("--target={target}")],
                vec!["--target".into(), target.into()],
            ] {
                assert_eq!(
                    source_target_hash_metadata_from_compiled(&compiler, root, &args).unwrap(),
                    (Some(label.into()), config),
                    "{args:?}"
                );
            }
        }
        assert_eq!(
            source_target_hash_metadata_from_compiled(&compiler, root, &[]).unwrap(),
            (Some("document".into()), true)
        );
        let (compiler, root) = compile_board_source(b"a -> b", "boards.d2").unwrap();
        assert_eq!(
            source_target_hash_metadata_from_compiled(&compiler, root, &[]).unwrap(),
            (None, true)
        );
    }

    #[test]
    fn render_layout_carries_imported_metadata_and_defers_metadata_errors() {
        let base = std::env::temp_dir().join(format!(
            "d2-rs-layout-metadata-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let imported = base.join("labels.d2");
        let input = b"...@labels\nlayers: {one: {a -> b}}\n";
        let args = vec![
            format!("--path={}", source_test_path(&base.join("index.d2"))),
            "--engine=dagre".into(),
        ];
        std::fs::write(&imported, "label: snapshot\na -> b\n").unwrap();
        let compiled = source_render_layout_with_theme(input, &args, None, false).unwrap();
        // The layout and its target metadata share one import snapshot. No
        // reader should reopen the import after layout to recover the label.
        std::fs::write(&imported, "label: updated\na -> b\n").unwrap();
        assert_eq!(
            compiled.target_metadata.unwrap(),
            (Some("snapshot".into()), true)
        );
        let next = source_render_layout_with_theme(input, &args, None, false).unwrap();
        assert_eq!(
            next.target_metadata.unwrap(),
            (Some("updated".into()), true)
        );
        std::fs::remove_dir_all(base).unwrap();

        // Existing source helpers take the last layout target but the first
        // metadata target. Preserve the post-layout metadata error boundary,
        // including layout-only callers that never request render metadata.
        let args = vec![
            "--engine=dagre".into(),
            "--target=missing".into(),
            "--target=one".into(),
        ];
        let input = b"layers: {one: {a -> b}}";
        let compiled = source_render_layout_with_theme(input, &args, None, false).unwrap();
        assert_eq!(
            compiled.target_metadata.unwrap_err(),
            "render target \"missing\" not found"
        );
        assert!(source_layout(input, &args).is_ok());
        assert_eq!(
            render_source(input, &args).unwrap_err(),
            "render target \"missing\" not found"
        );
    }

    #[test]
    fn document_config_and_plan_share_import_snapshot_with_nested_renders() {
        let base = std::env::temp_dir().join(format!(
            "d2-rs-config-plan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let imported = base.join("settings.d2");
        let path = source_test_path(&base.join("index.d2"));
        let input = b"...@settings\nlayers: {one: {a -> b}}\n";
        std::fs::write(&imported, "vars: {d2-config: {theme-id: 300}}\n").unwrap();
        {
            let _document = SourceConfigScope::enter(input);
            let (compiler, root) = compile_board_source(input, &path).unwrap();
            let config = source_config_from_compiled(&compiler, root);
            assert_eq!(
                config.as_ref().unwrap().as_ref().unwrap().theme_id,
                Some(300)
            );
            remember_source_config(input, &path, &config);
            let expected = source_board_plan_for_target(input, &path, "one").unwrap();
            // A document keeps its compiled view. A later watch compilation
            // must read the changed import, after the document scope ends.
            std::fs::write(&imported, "vars: {d2-config: {theme-id: 0}}\n").unwrap();
            let plan = source_board_plan_from_compiled(&compiler, root, "one").unwrap();
            assert_eq!(format!("{plan:?}"), format!("{expected:?}"));
            {
                let _render = SourceConfigScope::enter(input);
                assert_eq!(source_config(input, &path), config);
                let other_path = source_test_path(&base.join("other.d2"));
                assert_eq!(
                    source_config(input, &other_path).unwrap().unwrap().theme_id,
                    Some(0)
                );
            }
            assert_eq!(source_config(input, &path), config);
            {
                let other = b"vars: {d2-config: {theme-id: 4}}\n";
                let _other_document = SourceConfigScope::enter(other);
                assert_eq!(
                    source_config(other, &path).unwrap().unwrap().theme_id,
                    Some(4)
                );
            }
            assert_eq!(source_config(input, &path), config);
        }
        let _next_document = SourceConfigScope::enter(input);
        assert_eq!(
            source_config(input, &path).unwrap().unwrap().theme_id,
            Some(0)
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn nested_config_scopes_preserve_absence_and_complete_errors() {
        for input in [
            b"a -> b\n".as_slice(),
            b"vars: {d2-config: {theme-overrides: {B1: potato; B2: nope}}}\na\n",
            b"vars: {d2-config: {theme-id: nope; pad: nope}}\na\n",
            b"a: {\n",
        ] {
            let expected = compute_source_config(input, "config.d2");
            let _document = SourceConfigScope::enter(input);
            assert_eq!(source_config(input, "config.d2"), expected);
            {
                let _render = SourceConfigScope::enter(input);
                assert_eq!(source_config(input, "config.d2"), expected);
                // A different diagnostic path cannot reuse cached errors.
                assert_eq!(
                    source_config(input, "other.d2"),
                    compute_source_config(input, "other.d2")
                );
            }
            assert_eq!(source_config(input, "config.d2"), expected);
        }
    }

    #[test]
    fn layout_source_rejects_invalid_timeout_without_dispatching() {
        let error = source_layout(b"a -> b\n", &["--timeout=soon".into()]).unwrap_err();
        assert!(error.contains("invalid timeout value"), "{error}");
    }

    #[test]
    fn layout_listing_uses_canonical_engine_names() {
        let dagre =
            String::from_utf8(dispatch(&["layout".into(), "dagre".into()]).unwrap()).unwrap();
        assert!(dagre.starts_with("dagre (built-in):"));
        let tala = String::from_utf8(dispatch(&["layout".into(), "tala".into()]).unwrap()).unwrap();
        assert!(tala.starts_with("tala (built-in):"));
    }

    #[test]
    fn board_output_paths_match_d2_collection_layout() {
        let targets = vec![
            "layers.one".to_owned(),
            "layers.one.scenarios.deployed".to_owned(),
            "scenarios.review".to_owned(),
        ];
        assert_eq!(
            board_output_path(None, ExportFormat::Svg, &targets),
            PathBuf::from("index.svg")
        );
        assert_eq!(
            board_output_path(Some("layers.one"), ExportFormat::Svg, &targets),
            PathBuf::from("layers/one/index.svg")
        );
        assert_eq!(
            board_output_path(
                Some("layers.one.scenarios.deployed"),
                ExportFormat::Svg,
                &targets
            ),
            PathBuf::from("layers/one/deployed.svg")
        );
        assert_eq!(
            board_output_path(Some("scenarios.review"), ExportFormat::Svg, &targets),
            PathBuf::from("scenarios/review.svg")
        );
    }

    #[test]
    fn render_svg_connects_source_layout_and_labels() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(&["render-svg".into(), "--seeds=7".into()], b"a -> b\n").unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(output.contains("<path d=\"M "));
        assert!(output.contains(">a</text>"));
        assert!(output.contains(">b</text>"));
    }

    #[test]
    fn dagre_sequence_self_edges_keep_rounded_orthogonal_routes() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let source = b"sequence: { shape: sequence_diagram; worker -> worker: retry }\ncomplete: Complete\nsequence -> complete\n";
        let output = run(&["render-svg".into(), "--engine=dagre".into()], source).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(
                "M 64 224 L 132 224 S 142 224 142 234 L 142 259 S 142 269 132 269 L 66 269"
            )
        );
        assert!(output.contains("M 85 352 C 85 391 85 411 85 447"));
        assert!(!output.contains("M 64 224 C 142 224 142 269 66 269"));
        assert!(output.contains("stroke-dasharray=\"12,11.838767\" stroke-linecap=\"round\""));
    }

    #[test]
    fn render_svg_accepts_d2_padding_option() {
        let default_output = run(&["render-svg".into()], b"a\n").unwrap();
        let zero_output = run(&["render-svg".into(), "--pad=0".into()], b"a\n").unwrap();
        let view_box_width = |output: &[u8]| {
            let text = String::from_utf8_lossy(output);
            let value = text
                .split("viewBox=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                .expect("viewBox");
            value
                .split_whitespace()
                .nth(2)
                .expect("viewBox width")
                .parse::<f64>()
                .expect("numeric width")
        };
        assert_eq!(
            view_box_width(&default_output) - view_box_width(&zero_output),
            200.0
        );
    }

    #[test]
    fn render_svg_accepts_d2_scale_option() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let default_output = run(&["render-svg".into()], b"a\n").unwrap();
        let scaled_output = run(&["render-svg".into(), "--scale=2".into()], b"a\n").unwrap();
        let root_attribute = |output: &[u8], name: &str| {
            let text = String::from_utf8_lossy(output);
            text.split('>')
                .next()
                .expect("root SVG")
                .split(&format!("{name}=\""))
                .nth(1)
                .and_then(|value| value.split('"').next())
                .expect("root attribute")
                .to_owned()
        };
        // Pinned d2svg scales physical output size while keeping its logical
        // viewBox. Original and official CLI controls independently prove it.
        assert_eq!(
            root_attribute(&scaled_output, "viewBox"),
            root_attribute(&default_output, "viewBox")
        );
        let default_root = String::from_utf8_lossy(&default_output)
            .split('>')
            .next()
            .unwrap()
            .to_owned();
        assert!(!default_root.contains("width=\"") && !default_root.contains("height=\""));
        let explicit = run(&["render-svg".into(), "--scale=1".into()], b"a\n").unwrap();
        assert!(!root_attribute(&explicit, "width").is_empty());
        for dimension in ["width", "height"] {
            assert_eq!(
                root_attribute(&scaled_output, dimension)
                    .parse::<f64>()
                    .unwrap(),
                root_attribute(&default_output, "viewBox")
                    .split_whitespace()
                    .nth(if dimension == "width" { 2 } else { 3 })
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
                    * 2.0
            );
        }
    }

    #[test]
    fn render_svg_applies_pinned_theme_palette() {
        let output = String::from_utf8(
            run(&["render-svg".into(), "--theme=1".into()], b"a -> b\n").unwrap(),
        )
        .unwrap();
        assert!(output.contains("stroke=\"#0A0F25\""));
        assert!(output.contains("fill=\"#EEF1F8\""));
        assert!(output.contains("fill=\"#FFFFFF\""));
    }

    #[test]
    fn render_svg_hydrates_theme_id_from_source_config() {
        let input = b"vars: {d2-config: {theme-id: 1}}\na -> b\n";
        let output = String::from_utf8(run(&["render-svg".into()], input).unwrap()).unwrap();
        assert!(output.contains("stroke=\"#0A0F25\""));
        assert!(output.contains("fill=\"#EEF1F8\""));
    }

    #[test]
    fn render_svg_applies_terminal_caps_lock_before_layout() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = String::from_utf8(
            run(
                &["render-svg".into(), "--theme=300".into()],
                b"a: hello world\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(output.contains(">HELLO WORLD</text>"), "{output}");
        assert!(output.contains("Source Code Pro, monospace"), "{output}");
    }

    #[test]
    fn render_svg_rejects_unknown_theme_id() {
        let error = run(&["render-svg".into(), "--theme=999".into()], b"a\n").unwrap_err();
        assert!(error.contains("unknown theme ID 999"), "{error}");
    }

    #[test]
    fn render_svg_accepts_center_flag() {
        let output =
            String::from_utf8(run(&["render-svg".into(), "--center".into()], b"a\n").unwrap())
                .unwrap();
        assert!(output.contains("preserveAspectRatio=\"xMidYMid meet\""));
    }

    #[test]
    fn render_svg_hydrates_center_from_source_config() {
        let input = b"vars: {d2-config: {center: true}}\na\n";
        let output = String::from_utf8(run(&["render-svg".into()], input).unwrap()).unwrap();
        assert!(output.contains("preserveAspectRatio=\"xMidYMid meet\""));
    }

    #[test]
    fn render_svg_explicit_center_false_overrides_source_config() {
        let input = b"vars: {d2-config: {center: true}}\na\n";
        let output =
            String::from_utf8(run(&["render-svg".into(), "--center=false".into()], input).unwrap())
                .unwrap();
        assert!(output.contains("preserveAspectRatio=\"xMinYMin meet\""));
    }

    #[test]
    fn render_svg_applies_source_theme_overrides() {
        let input = b"vars: {d2-config: {theme-overrides: {B1: '#123456'; B6: '#abcdef'; N1: '#010203'}}}\na -> b\n";
        let output = String::from_utf8(run(&["render-svg".into()], input).unwrap()).unwrap();
        assert!(output.contains("stroke=\"#123456\""));
        assert!(output.contains("fill=\"#abcdef\""));
        assert!(output.contains("fill=\"#010203\""));
    }

    #[test]
    fn render_svg_applies_dark_theme_media_palette() {
        let output = String::from_utf8(
            run(&["render-svg".into(), "--dark-theme=200".into()], b"a\n").unwrap(),
        )
        .unwrap();
        assert!(output.contains("prefers-color-scheme:dark"));
        assert!(output.contains("#CDD6F4"));
    }

    #[test]
    fn render_svg_hydrates_dark_theme_from_source_config() {
        let input = b"vars: {d2-config: {dark-theme-id: 200}}\na\n";
        let output = String::from_utf8(run(&["render-svg".into()], input).unwrap()).unwrap();
        assert!(output.contains("prefers-color-scheme:dark"));
    }

    #[test]
    fn document_command_supports_deterministic_ascii() {
        let output = render_source_format(b"a -> b\n", &[], ExportFormat::Ascii).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("│ a  │"));
        assert!(text.contains("│ b  │"));
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_go_ascii_file_caller_contracts() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::load_str(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../d2-rs-export"),
            "tests/fixtures/ascii-callers-source-v090.json",
        ))
        .unwrap();
        assert_eq!(rows.len(), 192);
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.reference")
            .join(format!("ascii-callers-unit-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        for (index, row) in rows.into_iter().enumerate() {
            let directory = base.join(index.to_string());
            std::fs::create_dir_all(&directory).unwrap();
            let input = directory.join("input.d2");
            let output = directory.join("output.txt");
            std::fs::write(&input, row["source"].as_str().unwrap()).unwrap();
            let mut args = vec![format!("--ascii-mode={}", row["mode"].as_str().unwrap())];
            if let Some(engine) = row["engine"].as_str() {
                args.push(format!("--layout={engine}"));
            }
            if let Some(scale) = row["scale"].as_f64() {
                args.push(format!("--scale={scale}"));
            }
            args.push(input.to_string_lossy().into_owned());
            args.push(output.to_string_lossy().into_owned());
            let actual = run_document(&args);
            if let Some(files) = row["cli_files"].as_object() {
                // Multiboard ASCII: the CLI stages the board tree
                // (d2cli/board_output.go) instead of writing output.txt.
                assert!(actual.unwrap().is_empty(), "{}", row["name"]);
                assert!(!output.exists(), "{}", row["name"]);
                for (path, text) in files {
                    assert_eq!(
                        std::fs::read_to_string(directory.join(path)).unwrap(),
                        text.as_str().unwrap(),
                        "{} {path}",
                        row["name"]
                    );
                }
                continue;
            }
            match row["cli_kind"].as_str().unwrap() {
                "text" => {
                    assert!(actual.unwrap().is_empty(), "{}", row["name"]);
                    assert_eq!(
                        std::fs::read_to_string(&output).unwrap(),
                        row["output"].as_str().unwrap(),
                        "{}",
                        row["name"]
                    );
                }
                "no-output" => {
                    assert!(actual.unwrap().is_empty());
                    assert!(!output.exists());
                }
                "error" => {
                    let expected = row["cli_error_template"]
                        .as_str()
                        .unwrap()
                        .replace("{input}", &human_path(&input))
                        .replace("{directory}", &go_abs_path(&directory.to_string_lossy()));
                    assert_eq!(
                        format!("{}\n", actual.unwrap_err()),
                        expected,
                        "{}",
                        row["name"]
                    );
                    assert!(!output.exists());
                }
                other => panic!("unknown source outcome {other}"),
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn document_stdout_writer_receives_output_once_and_propagates_failure() {
        let path =
            std::env::temp_dir().join(format!("d2-rs-stdout-write-{}.d2", std::process::id()));
        std::fs::write(&path, b"a -> b\n").unwrap();
        let args = vec![
            "--layout=dagre".into(),
            path.to_string_lossy().into_owned(),
            "-".into(),
        ];
        let expected = run_document(&args).unwrap();
        let mut writes = Vec::new();
        let mut writer = |bytes: &[u8]| {
            writes.push(bytes.to_vec());
            Ok(())
        };
        assert!(
            run_document_with_stdout(&args, Some(&mut writer))
                .unwrap()
                .is_empty()
        );
        assert_eq!(writes, vec![expected]);
        let mut failing_writer = |_: &[u8]| Err("could not write stdout: broken pipe".into());
        let error = run_document_with_stdout(&args, Some(&mut failing_writer)).unwrap_err();
        assert!(error.contains("could not write stdout: broken pipe"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn document_command_supports_standard_ascii_charset() {
        let path =
            std::env::temp_dir().join(format!("d2-rs-ascii-standard-{}.d2", std::process::id()));
        std::fs::write(&path, b"a -> b\n").unwrap();
        let output = run_document(&[
            "--stdout-format=ascii".into(),
            "--ascii-mode=standard".into(),
            path.to_string_lossy().into_owned(),
            "-".into(),
        ])
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "+----+\n| a  |\n|    |\n+----+\n   |  \n   v  \n+----+\n| b  |\n|    |\n+----+\n"
        );
    }

    #[test]
    fn document_command_rejects_legacy_ppt_before_reading_source() {
        for format_args in [vec![], vec!["--stdout-format=svg".to_owned()]] {
            let mut args = format_args;
            args.extend(["missing-input.d2".to_owned(), "out.ppt".to_owned()]);
            assert_eq!(
                run_document(&args).unwrap_err(),
                "bad usage: D2 does not support ppt exports, did you mean \"pptx\"?\nRun with --help to see usage.",
            );
        }
    }

    #[test]
    fn document_command_infers_format_from_output_extension() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-format-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-format-{suffix}.png"));
        std::fs::write(&input_path, b"a -> b\n").unwrap();
        let result = run_document(&[
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert!(result.is_empty());
        assert!(
            std::fs::read(&output_path)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn document_svg_metadata_flags_match_d2_boundary() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let path =
            std::env::temp_dir().join(format!("d2-rs-svg-metadata-{}.d2", std::process::id()));
        std::fs::write(&path, b"a\n").unwrap();
        let base_args = || vec![path.to_string_lossy().into_owned(), "-".into()];
        let default = String::from_utf8(run_document(&base_args()).unwrap()).unwrap();
        assert!(default.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(default.contains("d2Version=\"v0.9.0\""));

        let mut no_tag = vec!["--no-xml-tag".into()];
        no_tag.extend(base_args());
        let no_tag = String::from_utf8(run_document(&no_tag).unwrap()).unwrap();
        assert!(no_tag.starts_with("<svg"));
        assert!(no_tag.contains("d2Version=\"v0.9.0\""));

        let mut no_version = vec!["--omit-version".into()];
        no_version.extend(base_args());
        let no_version = String::from_utf8(run_document(&no_version).unwrap()).unwrap();
        assert!(no_version.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(!no_version.contains("d2Version="));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn document_salt_is_forwarded_to_svg_target() {
        let path = std::env::temp_dir().join(format!("d2-rs-svg-salt-{}.d2", std::process::id()));
        std::fs::write(&path, b"a -> b\n").unwrap();
        let output = run_document(&[
            "--salt=embedded-copy".into(),
            path.to_string_lossy().into_owned(),
            "-".into(),
        ])
        .unwrap();
        std::fs::remove_file(path).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("id=\"mk-d2-361933264-3488378134\""));
        assert!(output.contains("marker-end=\"url(#mk-d2-361933264-3488378134)\""));
    }

    #[test]
    fn ascii_document_defaults_to_elk_when_engine_is_not_explicit() {
        let path = std::env::temp_dir().join(format!("d2-rs-ascii-{}.d2", std::process::id()));
        std::fs::write(&path, b"a -> b\n").unwrap();
        let args = vec![
            "--stdout-format=ascii".into(),
            path.to_string_lossy().into_owned(),
            "-".into(),
        ];
        let output = run_document(&args).unwrap();
        std::fs::remove_file(path).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("│ a  │"));
        assert!(text.contains("│ b  │"));
    }

    #[test]
    fn empty_document_writes_no_output_and_preserves_existing_files() {
        let directory =
            std::env::temp_dir().join(format!("d2-rs-empty-document-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("input.d2");
        for input in [b"".as_slice(), b"shape: sequence_diagram\n".as_slice()] {
            std::fs::write(&source, input).unwrap();
            for engine in ["tala", "dagre", "elk"] {
                for extension in ["svg", "png", "pdf", "pptx", "gif"] {
                    let output = directory.join(format!("{engine}.{extension}"));
                    let args = vec![
                        format!("--layout={engine}"),
                        source.to_string_lossy().into_owned(),
                        output.to_string_lossy().into_owned(),
                    ];
                    // Pinned Go: paged.go and raster.go count renderable
                    // boards themselves and reject an empty document.
                    let expected_error = match extension {
                        "pdf" | "pptx" => {
                            Some("paged export requires at least one renderable board")
                        }
                        "gif" => Some("GIF animation requires at least one renderable board"),
                        _ => None,
                    };
                    if let Some(expected_error) = expected_error {
                        std::fs::write(&output, b"existing data").unwrap();
                        let error = run_document(&args).unwrap_err();
                        assert!(error.ends_with(expected_error), "{error}");
                        assert_eq!(std::fs::read(&output).unwrap(), b"existing data");
                        std::fs::remove_file(output).unwrap();
                        continue;
                    }
                    assert!(run_document(&args).unwrap().is_empty());
                    assert!(!output.exists());
                    std::fs::write(&output, b"existing data").unwrap();
                    assert!(run_document(&args).unwrap().is_empty());
                    assert_eq!(std::fs::read(&output).unwrap(), b"existing data");
                    std::fs::remove_file(output).unwrap();
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn document_directory_resolves_index_before_imports_and_keeps_output_stem() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let directory = std::env::temp_dir().join(format!("d2-rs-index-{suffix}"));
        let source_dir = directory.join("life");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("index.d2"), b"...@nodes\n").unwrap();
        std::fs::write(source_dir.join("nodes.d2"), b"a: imported\n").unwrap();
        let input = source_dir.to_string_lossy().into_owned();
        let default_output = source_dir.with_extension("svg");
        #[cfg(windows)]
        {
            // filepath.Join uses backslashes for directory/index.d2. Pinned
            // Go's slash-only path.Dir then resolves this import from the CWD.
            let error = run_document(std::slice::from_ref(&input)).unwrap_err();
            assert!(error.contains("failed to import \"nodes.d2\""), "{error}");
            assert!(!default_output.exists());
            std::fs::write(source_dir.join("index.d2"), b"a: imported\n").unwrap();
        }
        run_document(std::slice::from_ref(&input)).unwrap();
        assert!(
            std::fs::read_to_string(&default_output)
                .unwrap()
                .contains("imported")
        );
        assert!(!source_dir.join("index.svg").exists());

        let explicit_output = directory.join("explicit.svg");
        run_document(&[
            input.clone(),
            explicit_output.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert_eq!(
            std::fs::read(&default_output).unwrap(),
            std::fs::read(&explicit_output).unwrap()
        );

        // The default multiboard folder is life/, rather than life/index/.
        std::fs::write(
            source_dir.join("index.d2"),
            b"a\nlayers: { child: { b } }\n",
        )
        .unwrap();
        run_document(std::slice::from_ref(&input)).unwrap();
        assert!(source_dir.join("index.svg").is_file());
        assert!(source_dir.join("child.svg").is_file());
        assert!(!source_dir.join("index").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn folder_only_steps_still_select_multiboard_output_paths() {
        let directory = std::env::temp_dir().join(format!(
            "d2-rs-empty-step-paths-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("input.d2");
        // Pinned Go d2cli/board_output.go planBoardOutput selects index
        // whenever a child exists, even when none of the children render.
        for source in [
            "steps: { first: {} }\nnode\n",
            "steps: { first: { steps: { inner: {} } } }\nnode\n",
        ] {
            std::fs::write(&input, source).unwrap();
            for engine in ["dagre", "elk", "tala"] {
                for extension in ["svg", "png"] {
                    let output = directory.join(format!("{engine}.{extension}"));
                    let args = vec![
                        format!("--layout={engine}"),
                        input.to_string_lossy().into_owned(),
                        output.to_string_lossy().into_owned(),
                    ];
                    assert!(run_document(&args).unwrap().is_empty());
                    assert!(!output.exists());
                    let tree = output.with_extension("");
                    assert!(tree.join(format!("index.{extension}")).is_file());
                    assert!(
                        tree.join(board_output::BOARD_OUTPUT_MANIFEST_NAME)
                            .is_file()
                    );
                    assert!(!tree.join(format!("first.{extension}")).exists());
                    std::fs::remove_dir_all(tree).unwrap();
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn document_command_packages_all_boards_when_target_is_omitted() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-boards-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-boards-{suffix}.svg"));
        let output_dir = output_path.with_extension("");
        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::write(output_dir.join("stale.svg"), b"stale").unwrap();
        std::fs::write(
            &input_path,
            b"a: root\nlayers: {\n  one: { b: first }\n  two: { c: second }\n}\n",
        )
        .unwrap();
        let args = vec![
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let index = std::fs::read_to_string(output_dir.join("index.svg")).unwrap();
        let first = std::fs::read_to_string(output_dir.join("one.svg")).unwrap();
        let second = std::fs::read_to_string(output_dir.join("two.svg")).unwrap();
        assert!(index.contains(">root</text>"));
        assert!(first.contains(">first</text>"));
        assert!(second.contains(">second</text>"));
        // Master publishes transactionally and preserves files it never
        // generated (d2cli/board_output.go).
        assert_eq!(
            std::fs::read(output_dir.join("stale.svg")).unwrap(),
            b"stale"
        );
        assert!(
            output_dir
                .join(board_output::BOARD_OUTPUT_MANIFEST_NAME)
                .is_file()
        );
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn watch_preview_publishes_selected_subtree_and_preserves_user_files() {
        let input_path =
            std::env::temp_dir().join(format!("d2-rs-watch-tree-{}.d2", std::process::id()));
        let output_path = input_path.with_extension("svg");
        let output_dir = input_path.with_extension("");
        std::fs::write(
            &input_path,
            br#"
a: {link: layers.b}
layers: {
  b: {
    back: {link: _}
    layers: {child: {c}}
  }
  sibling: {s}
}
"#,
        )
        .unwrap();
        let render = |target: &str| {
            run_document(&[
                input_path.to_string_lossy().into_owned(),
                output_path.to_string_lossy().into_owned(),
                "--watch-preview".into(),
                format!("--target={target}"),
            ])
            .unwrap()
        };
        let root = render("");
        assert_eq!(root, std::fs::read(output_dir.join("index.svg")).unwrap());
        assert!(
            String::from_utf8(root.clone())
                .unwrap()
                .contains("href=\"b/index.svg\"")
        );
        assert!(output_dir.join("b/child.svg").is_file());
        assert!(!output_path.exists());
        let sibling = std::fs::read(output_dir.join("sibling.svg")).unwrap();
        std::fs::write(output_dir.join("b/stale.svg"), b"obsolete").unwrap();
        let selected = render("b");
        assert_eq!(
            selected,
            std::fs::read(output_dir.join("b/index.svg")).unwrap()
        );
        assert!(
            String::from_utf8(selected)
                .unwrap()
                .contains("href=\"../index.svg\"")
        );
        // Master's watch compile publishes the selected subtree into the
        // root output tree: generated files absent from the new manifest
        // retire, while files D2 never generated remain.
        assert_eq!(
            std::fs::read(output_dir.join("b/stale.svg")).unwrap(),
            b"obsolete"
        );
        assert!(!output_dir.join("index.svg").exists());
        assert!(!sibling.is_empty());
        assert!(!output_dir.join("sibling.svg").exists());
        assert_eq!(
            render("b.child"),
            std::fs::read(output_dir.join("child.svg")).unwrap()
        );
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn document_command_omits_nested_folder_only_boards() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-nested-folder-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-nested-folder-{suffix}.svg"));
        let output_dir = output_path.with_extension("");
        std::fs::write(
            &input_path,
            b"layers: {\n  group: {\n    layers: {\n      one: { a: child }\n    }\n  }\n}\n",
        )
        .unwrap();
        assert!(
            run_document(&[
                input_path.to_string_lossy().into_owned(),
                output_path.to_string_lossy().into_owned(),
            ])
            .unwrap()
            .is_empty()
        );
        assert!(output_dir.join("group/one.svg").exists());
        assert!(!output_dir.join("group.svg").exists());
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn document_command_packages_all_boards_as_one_pdf_and_supports_stdout() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-pdf-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-pdf-{suffix}.pdf"));
        std::fs::write(
            &input_path,
            b"a: root\nlayers: {\n  one: { b: first }\n  two: { c: second }\n}\n",
        )
        .unwrap();
        let file_args = vec![
            "--stdout-format=pdf".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&file_args).unwrap().is_empty());
        let file_bytes = std::fs::read(&output_path).unwrap();
        assert!(file_bytes.starts_with(b"%PDF-1.4\n"));
        assert!(file_bytes.windows(8).any(|window| window == b"/Count 3"));
        assert!(file_bytes.ends_with(b"%%EOF\n"));

        let stdout_args = vec![
            "--stdout-format=pdf".into(),
            input_path.to_string_lossy().into_owned(),
            "-".into(),
        ];
        let stdout_bytes = run_document(&stdout_args).unwrap();
        assert_eq!(
            normalize_pdf_timestamps(&stdout_bytes),
            normalize_pdf_timestamps(&file_bytes)
        );

        // d2cli/paged.go divides clip work across all eleven boards before
        // rendering board zero. This nested sketch has 23,889,216 work in
        // the pinned Go planner, exceeding 250,000,000 / 11.
        std::fs::remove_file(&output_path).unwrap();
        let steps = (0..10).map(|i| format!("s{i}: {{}}\n")).collect::<String>();
        std::fs::write(&input_path, format!("a: {{b}}\nsteps: {{\n{steps}}}\n")).unwrap();
        for extension in ["pdf", "pptx"] {
            let output = output_path.with_extension(extension);
            let error = run_document(&[
                "--layout=elk".into(),
                "--theme=200".into(),
                "--sketch".into(),
                "--pad=20".into(),
                input_path.to_string_lossy().into_owned(),
                output.to_string_lossy().into_owned(),
            ])
            .unwrap_err();
            // HumanPath abbreviates $HOME, which contains the temp directory on Windows.
            assert_eq!(
                error,
                format!(
                    "err: failed to compile {}: paged board 0 frame: d2raster: even-odd clip work 23889216 exceeds limit 22727272",
                    human_path(&input_path)
                )
            );
            assert!(!output.exists());
        }
        std::fs::remove_file(input_path).unwrap();
    }

    #[test]
    fn document_command_includes_single_board_root_title_in_pdf_navigation() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-pdf-title-{suffix}.d2"));
        std::fs::write(&input_path, b"label: \"D2 PDF\"\na -> b\n").unwrap();
        let output = run_document(&[
            "--stdout-format=pdf".into(),
            input_path.to_string_lossy().into_owned(),
            "-".into(),
        ])
        .unwrap();
        let pdf = String::from_utf8_lossy(&output);
        assert!(pdf.contains("/MediaBox [0 0 576.00 648.00]"));
        assert!(pdf.contains("/Subtype /Type0"));
        assert!(pdf.contains("/FontFile2"));
        // FPDF compresses the page stream and writes literal UTF-16 text.
        let marker = b"\n4 0 obj\n";
        let object = output
            .windows(marker.len())
            .position(|w| w == marker)
            .unwrap()
            + marker.len();
        let stream = b"\nstream\n";
        let start = object
            + output[object..]
                .windows(stream.len())
                .position(|w| w == stream)
                .unwrap()
            + stream.len();
        let header = std::str::from_utf8(&output[object..start]).unwrap();
        let length = header
            .split("/Length ")
            .nth(1)
            .unwrap()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut commands = Vec::new();
        flate2::read::ZlibDecoder::new(&output[start..start + length])
            .read_to_end(&mut commands)
            .unwrap();
        assert!(commands.windows(12).any(|w| w == b"\0D\x002\0 \0P\0D\0F"));
        let producer = b"/Producer (\xfe\xff\0F\0P\0D\0F\0 \x001\0.\x007)";
        assert!(output.windows(producer.len()).any(|w| w == producer));
        std::fs::remove_file(input_path).unwrap();
    }

    #[test]
    fn document_command_preserves_links_across_multiboard_pdf_pages() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-pdf-links-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-pdf-links-{suffix}.pdf"));
        std::fs::write(
            &input_path,
            b"a: {label: root; link: https://example.com/root}\nlayers: {\n  one: { b: {label: first; link: https://example.com/one} }\n  two: { c: second }\n}\n",
        )
        .unwrap();
        let args = vec![
            "--stdout-format=pdf".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let bytes = std::fs::read(&output_path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Count 3"));
        assert_eq!(text.matches("/Subtype /Link").count(), 2);
        assert!(text.contains("https://example.com/root"));
        assert!(text.contains("https://example.com/one"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn static_gif_uses_interval_samples_delays_and_single_frame_loop_policy() {
        // Pinned e2etests-cli/one-layer-gif and d2cli/raster.go apply the
        // same sample schedule whether or not the scene contains animation.
        for (interval, expected) in [(10, vec![1]), (250, vec![3, 3, 3, 3, 3, 3, 3, 4])] {
            let bytes = render_source_animated_gif(b"x", &[], interval).unwrap();
            let loops = bytes.windows(11).any(|window| window == b"NETSCAPE2.0");
            assert_eq!(loops, expected.len() > 1);
            let mut decoder = gif::DecodeOptions::new()
                .read_info(std::io::Cursor::new(bytes))
                .unwrap();
            let mut delays = Vec::new();
            while let Some(frame) = decoder.read_next_frame().unwrap() {
                delays.push(frame.delay);
            }
            assert_eq!(delays, expected);
        }
    }

    #[test]
    fn document_command_packages_all_boards_as_animated_gif() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-gif-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-gif-{suffix}.gif"));
        std::fs::write(
            &input_path,
            b"a: root\nlayers: {\n  one: { b: first }\n  two: { c: second }\n}\n",
        )
        .unwrap();
        let args = vec![
            "--stdout-format=gif".into(),
            "--animate-interval=250".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let bytes = std::fs::read(&output_path).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            frames.push(frame.delay);
        }
        assert_eq!(frames.len(), 24);
        for board_delays in frames.chunks(8) {
            assert_eq!(board_delays, [3, 3, 3, 3, 3, 3, 3, 4]);
        }
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn animation_destination_policy_matches_original_go_io_boundaries() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/animation-io-source-v090.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 6);
        for case in cases {
            let source = case["input"].as_str().unwrap().as_bytes();
            let full = source_board_plan(source, "index.d2").unwrap();
            let (root, plan) = match case["target"]
                .as_str()
                .and_then(|target| target.strip_suffix(".*"))
            {
                Some(target) => animation_subtree_plan(full, target).unwrap(),
                None => (String::new(), full),
            };
            let result = validate_animation_destination(
                if case["stdin"] == true {
                    "-"
                } else {
                    "index.d2"
                },
                Some(if case["stdout"] == true {
                    "-"
                } else {
                    "output.svg"
                }),
                &root,
                &plan.all_targets,
            );
            if case["expected_exit"] == 0 {
                assert!(result.is_ok(), "{}", case["name"]);
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    format!("err: failed to compile -: {error}\n"),
                    case["expected_error"].as_str().unwrap()
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn animated_svg_canvas_targets_match_original_go_cli() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/animation-target-canvas-source-v090.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 46);
        let base =
            std::env::temp_dir().join(format!("d2-rs-source-animation-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let folder = base.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            let input = folder.join("input.d2");
            let output = folder.join("output.svg");
            std::fs::write(&input, case["input"].as_str().unwrap()).unwrap();
            let mut args = vec![
                "--layout=dagre".to_owned(),
                "--animate-interval=500".to_owned(),
            ];
            args.extend(
                case["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned()),
            );
            args.push(input.to_string_lossy().into_owned());
            args.push(output.to_string_lossy().into_owned());
            assert!(run_document(&args).unwrap().is_empty(), "{name}");
            if case["expected"].is_null() {
                assert!(!output.exists(), "{name}: unexpected frame output");
                continue;
            }
            let svg = std::fs::read_to_string(&output).unwrap();
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let root = doc.root_element();
            let inner = root
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "svg")
                .unwrap();
            for (node, key) in [(root, "attributes"), (inner, "inner_attributes")] {
                for (attribute, expected) in case["expected"][key].as_object().unwrap() {
                    assert_eq!(
                        node.attribute(attribute.as_str()),
                        expected.as_str(),
                        "{name} {key} {attribute}"
                    );
                }
            }
            assert_eq!(
                svg.matches("animation: d2Transition-").count(),
                case["expected"]["transitions"].as_u64().unwrap() as usize,
                "{name}"
            );
            assert!(
                svg.contains(&format!(
                    "d2Transition-{}-0",
                    case["expected"]["hash"].as_str().unwrap()
                )),
                "{name}: recursive hash differs"
            );
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn document_command_packages_folder_only_boards_as_animated_svg() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-svg-animation-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-svg-animation-{suffix}.svg"));
        std::fs::write(
            &input_path,
            b"label: Architecture\nlayers: {\n  one: { b: first }\n  two: { c: second }\n}\n",
        )
        .unwrap();
        let args = vec![
            "--stdout-format=svg".into(),
            "--animate-interval=125".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let output = std::fs::read_to_string(&output_path).unwrap();
        assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(output.contains("@keyframes d2Transition-d2-"));
        assert!(output.contains("d2Transition-d2-3591019550-0"));
        assert!(output.contains("animation: d2Transition-d2-"));
        assert_eq!(output.matches(" 250ms infinite").count(), 2);
        assert!(output.contains(">first</text>"));
        assert!(output.contains(">second</text>"));
        assert!(!output.contains(">Architecture</text>"));
        assert!(!output_path.with_extension("").exists());
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn document_command_uses_recursive_v090_animation_hash() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-svg-hash-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-svg-hash-{suffix}.svg"));
        std::fs::write(&input_path, b"x -> y\nlayers: { n: { a -> b } }\n").unwrap();
        let args = vec![
            "--stdout-format=svg".into(),
            "--animate-interval=100".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let output = std::fs::read_to_string(&output_path).unwrap();
        assert!(output.contains("d2Transition-d2-912644653-0"));
        assert!(output.contains("d2-3603291771 d2-svg"));
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn document_command_uses_recursive_tala_v090_animation_hash() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-tala-svg-hash-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-tala-svg-hash-{suffix}.svg"));
        std::fs::write(&input_path, b"x -> y\nlayers: { n: { a -> b } }\n").unwrap();
        let args = vec![
            "--layout=tala".into(),
            "--stdout-format=svg".into(),
            "--animate-interval=100".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let output = std::fs::read_to_string(&output_path).unwrap();
        assert!(output.contains("d2Transition-d2-3671043608-0"));
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn document_command_animates_a_single_svg_board_when_requested() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-single-svg-{suffix}.d2"));
        std::fs::write(&input_path, b"a -> b\n").unwrap();
        let output = run_document(&[
            "--stdout-format=svg".into(),
            "--animate-interval=100".into(),
            input_path.to_string_lossy().into_owned(),
            "-".into(),
        ])
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(output.contains("@keyframes d2Transition-d2-"));
        assert!(output.contains("animation: d2Transition-d2-"));
        assert!(output.contains(" 100ms infinite"));
        std::fs::remove_file(input_path).unwrap();
    }

    #[test]
    fn document_command_samples_animated_edges_on_each_board() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("test")
                .replace("::", "-")
        );
        let input_path = std::env::temp_dir().join(format!("d2-rs-animated-boards-{suffix}.d2"));
        let output_path = std::env::temp_dir().join(format!("d2-rs-animated-boards-{suffix}.gif"));
        std::fs::write(
            &input_path,
            b"a: one\nb: two\na -> b: {style.animated: true}\nlayers: {\n  one: { c: child; d: child2; c -> d: {style.animated: true} }\n}\n",
        )
        .unwrap();
        let args = vec![
            "--stdout-format=gif".into(),
            "--animate-interval=100".into(),
            input_path.to_string_lossy().into_owned(),
            output_path.to_string_lossy().into_owned(),
        ];
        assert!(run_document(&args).unwrap().is_empty());
        let bytes = std::fs::read(&output_path).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).unwrap();
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delays.push(frame.delay);
        }
        assert_eq!(delays.len(), 6);
        assert_eq!(delays, [3, 3, 4, 3, 3, 4]);
        std::fs::remove_file(input_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
    }

    #[test]
    fn document_empty_target_selects_root_without_packaging() {
        let input_path =
            std::env::temp_dir().join(format!("d2-rs-root-target-{}.d2", std::process::id()));
        std::fs::write(&input_path, b"a: root\nlayers: { one: { b: child } }\n").unwrap();
        let args = vec![
            "--target=".into(),
            input_path.to_string_lossy().into_owned(),
            "-".into(),
        ];
        let output = String::from_utf8(run_document(&args).unwrap()).unwrap();
        assert!(output.contains(">root</text>"));
        assert!(!output.contains(">child</text>"));
        std::fs::remove_file(input_path).unwrap();
    }

    #[test]
    fn explicit_board_targets_hash_only_the_selected_board() {
        // Pinned d2cli.main: empty/named targets clear descendants before
        // HashID; '*' and 'board.*' retain them. Go hashes of this witness
        // were captured in run28-witness-before.json.
        let source = b"a\nlayers: {\n  one: {\n    b\n    layers: {two: {c}}\n  }\n}\n";
        let path =
            std::env::temp_dir().join(format!("d2-rs-target-hash-{}.d2", std::process::id()));
        std::fs::write(&path, source).unwrap();
        for (target, hash) in [("", "3672566581"), ("layers.one", "2567905072")] {
            for flags in [
                vec![format!("--target={target}")],
                vec!["--target".into(), target.into()],
            ] {
                let mut args = flags;
                args.extend([path.to_string_lossy().into_owned(), "-".into()]);
                let svg = String::from_utf8(run_document(&args).unwrap()).unwrap();
                assert!(svg.contains(&format!("class=\"d2-{hash} d2-svg\"")));
            }
        }
        for (target, hash) in [("*", "1206689147"), ("layers.one.*", "1755266571")] {
            for mut flags in [
                vec![format!("--target={target}")],
                vec!["--target".into(), target.into()],
            ] {
                flags.push("--engine=dagre".into());
                let svg = String::from_utf8(render_source(source, &flags).unwrap()).unwrap();
                assert!(svg.contains(&format!("class=\"d2-{hash} d2-svg\"")));
            }
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn document_command_applies_source_layout_and_seed_config() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let path = std::env::temp_dir().join(format!("d2-rs-config-{}.d2", std::process::id()));
        std::fs::write(
            &path,
            b"vars: {d2-config: {layout-engine: dagre; pad: 7; data: {tala-seeds: [7]}}}\na -> b\n",
        )
        .unwrap();
        let args = vec![
            "--stdout-format=svg".into(),
            path.to_string_lossy().into_owned(),
            "-".into(),
        ];
        let configured = run_document(&args).unwrap();
        std::fs::remove_file(path).unwrap();
        let explicit = render_source_format(
            b"a -> b\n",
            &[
                "--engine=dagre".into(),
                "--seeds=7".into(),
                "--pad=7".into(),
            ],
            ExportFormat::Svg,
        )
        .map(|output| finalize_svg_output(output, false, false))
        .unwrap();
        // Original Go v0.9.0 retains document Config in Diagram.Bytes:
        // these two inputs have identical geometry but different namespaces.
        // Assert both source hashes before consistently relabeling all SVG
        // definitions/references for the complete geometry/style comparison.
        let configured = String::from_utf8(configured).unwrap();
        let explicit = String::from_utf8(explicit).unwrap();
        assert!(configured.contains("class=\"d2-1694691162 d2-svg\""));
        assert!(explicit.contains("class=\"d2-2044524636 d2-svg\""));
        assert_eq!(
            configured.replace("d2-1694691162", "d2-2044524636"),
            explicit
        );
    }

    #[test]
    fn render_svg_projects_markdown_labels() {
        let output = run(
            &["render-svg".into()],
            b"a: |md **hello** [world](https://example.com)|\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        // Native SVG renders Markdown inline runs as separate text elements,
        // matching D2's role-class/link representation rather than carrying
        // the intermediate HTML `<strong>` node into the scene. Concrete
        // emphasis faces are selected by class; font-weight is reserved for
        // synthetic emphasis axes.
        assert!(output.contains(">hello</text>"));
        assert!(output.contains("class=\"md-text text-bold"));
        assert!(output.contains(">world</text>"));
        assert!(output.contains("href=\"https://example.com\""));
        assert!(!output.contains("**hello**"));
    }

    #[test]
    fn render_svg_rejects_unsupported_markdown_html_like_d2() {
        let error = run(&["render-svg".into()], b"a: |md\n<u>under</u>\n|\n").unwrap_err();
        assert_eq!(
            error,
            "native Markdown SVG does not support HTML element <u>"
        );
    }

    #[test]
    fn render_svg_resolves_markdown_substitutions() {
        let output = run(
            &["render-svg".into()],
            b"vars: {name: world}; a: |md **hello ${name}**|\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(">hello world</text>"));
        assert!(!output.contains("${name}"));
        assert!(!output.contains(">vars</text>"));
    }

    #[test]
    fn render_svg_places_automatic_external_labels_outside_nodes() {
        let output = run(
            &["render-svg".into(), "--bundle=false".into()],
            b"image: {shape: image; icon: https://example.com/image.svg}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(">image</text>"));
        assert!(output.contains("<text"));
    }

    #[test]
    fn render_svg_preserves_source_node_and_edge_styles() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a: {style.fill: red; style.stroke: blue; style.stroke-width: 4; style.border-radius: 12; style.font-color: purple}\nb -> c: {style.stroke: green; style.stroke-dash: 2; style.bold: true}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("fill=\"red\""));
        assert!(output.contains("stroke=\"blue\""));
        assert!(output.contains("stroke-width=\"4\""));
        assert!(output.contains("stroke=\"green\""));
        assert!(output.contains("stroke-dasharray=\"4,3.946256\""));
        assert!(output.contains("font-weight=\"bold\""));
        assert!(output.contains("rx=\"12\""));
        assert!(output.contains("fill=\"purple\""));
    }

    #[test]
    fn render_svg_emits_node_links_and_tooltips() {
        let output = run(
            &["render-svg".into()],
            b"a: {label: hello; link: https://example.com; tooltip: tip}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("<a href=\"https://example.com\" xlink:href=\"https://example.com\">")
        );
        assert!(output.contains("<title>tip</title>"));
        assert!(output.contains("</a>"));
        // v0.9.0 reserves the linked-node badge envelope in both axes.
        // This simple witness therefore has a 363×285 root viewbox.
        assert!(output.contains("viewBox=\"0 0 363 285\""));
    }

    #[test]
    fn render_svg_force_appendix_adds_static_link_and_tooltip_entries() {
        let output = run(
            &["render-svg".into(), "--force-appendix".into()],
            b"a: {label: hello; link: https://example.com; tooltip: tip}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("class=\"appendix\""));
        assert!(output.contains(">tip</text>"));
        assert!(output.contains(">https://example.com</text>"));
        assert!(output.matches("viewBox=").count() >= 2);
    }

    #[test]
    fn render_svg_emits_edge_links_and_tooltips() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a -> b: {label: link; link: https://example.com/edge; tooltip: edge tip}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(
            "<a href=\"https://example.com/edge\" xlink:href=\"https://example.com/edge\">"
        ));
        assert!(output.contains("<title>edge tip</title>"));
        assert!(output.contains(">link</text>"));
        assert!(output.contains("</a>"));
    }

    #[test]
    fn render_svg_emits_node_icons_at_the_requested_position() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into(), "--bundle=false".into()],
            b"a: {label: hello; icon: https://example.com/icon.svg; icon-position: inside-top-left}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        // D2 GetIconSize uses ceil(min(106, 92) / 2) for this inside icon.
        assert!(output.contains("<rect x=\"0\" y=\"0\" width=\"106\" height=\"92\""));
        assert!(output.contains(
            "<image x=\"5\" y=\"5\" width=\"46\" height=\"46\" href=\"https://example.com/icon.svg\""
        ));
    }

    #[test]
    fn render_svg_uses_image_shape_icon_as_full_image_box() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into(), "--bundle=false".into()],
            b"a: {shape: image; icon: https://example.com/image.svg}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(
            "<image x=\"0\" y=\"0\" width=\"128\" height=\"128\" href=\"https://example.com/image.svg\""
        ));
        assert!(
            !output.contains("width=\"24\" height=\"24\" href=\"https://example.com/image.svg\"")
        );
    }

    #[test]
    fn render_svg_bundles_local_image_assets_for_file_sources() {
        let base =
            std::env::temp_dir().join(format!("d2-rs-assets-{}-{}", std::process::id(), "image"));
        std::fs::create_dir_all(&base).unwrap();
        let source_path = base.join("diagram.d2");
        std::fs::write(
            base.join("icon.svg"),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"1\" height=\"1\"/></svg>",
        )
        .unwrap();
        let output = run(
            &[
                "render-svg".into(),
                format!("--path={}", source_path.display()),
            ],
            b"a: {shape: image; icon: icon.svg}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("href=\"data:image/svg+xml;base64,"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn render_svg_bundles_local_edge_icon_assets_for_file_sources() {
        let base = std::env::temp_dir().join(format!(
            "d2-rs-assets-{}-{}",
            std::process::id(),
            "edge-icon"
        ));
        std::fs::create_dir_all(&base).unwrap();
        let source_path = base.join("diagram.d2");
        std::fs::write(
            base.join("edge-icon.svg"),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>",
        )
        .unwrap();
        let output = run(
            &[
                "render-svg".into(),
                format!("--path={}", source_path.display()),
            ],
            b"a -> b: {icon: edge-icon.svg}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("href=\"data:image/svg+xml;base64,"));
        assert!(!output.contains("href=\"edge-icon.svg\""));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn render_svg_bundles_remote_image_assets_for_file_sources() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let body = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M0 0h1v1z\"/></svg>";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
            std::io::Write::write_all(&mut stream, body).unwrap();
        });
        let base =
            std::env::temp_dir().join(format!("d2-rs-assets-{}-{}", std::process::id(), "remote"));
        std::fs::create_dir_all(&base).unwrap();
        let source_path = base.join("diagram.d2");
        let icon_url = format!("http://{address}/icon.svg");
        let source = format!("a: {{shape: image; icon: {icon_url}}}\n");
        let output = run(
            &[
                "render-svg".into(),
                format!("--path={}", source_path.display()),
            ],
            source.as_bytes(),
        )
        .unwrap();
        server.join().unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("href=\"data:image/svg+xml;base64,"));
        assert!(!output.contains(&format!("href=\"{icon_url}\"")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn render_svg_bundles_remote_image_assets_from_stdin() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let body =
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
            std::io::Write::write_all(&mut stream, body).unwrap();
        });
        let icon_url = format!("http://{address}/stdin-icon.svg");
        let source = format!("a: {{shape: image; icon: {icon_url}}}\n");
        let output = run(&["render-svg".into()], source.as_bytes()).unwrap();
        server.join().unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("href=\"data:image/svg+xml;base64,"));
        assert!(!output.contains(&format!("href=\"{icon_url}\"")));
    }

    #[test]
    fn diagnostic_paths_include_sibling_relative_paths() {
        let cwd = std::env::current_dir().unwrap();
        let sibling = cwd.parent().unwrap().join("d2/input.d2");
        let relative = PathBuf::from("..").join("d2").join("input.d2");
        assert_eq!(human_path(&sibling), relative.to_string_lossy());
        assert_eq!(human_path(&cwd), ".");
        assert_eq!(human_path(Path::new("-")), "-");
        assert_eq!(human_path(Path::new("./folder/../input.d2")), "input.d2");
        assert_eq!(human_path(&cwd.join("folder/../input.d2")), "input.d2");
        assert_eq!(
            human_path(Path::new("../d2/./input.d2")),
            relative.to_string_lossy()
        );
    }

    #[test]
    #[cfg(windows)]
    fn absolute_source_paths_preserve_slashes_for_cross_drive_imports() {
        // util-go/xmain.State.AbsPath returns absolute input verbatim; d2ir
        // import.go then uses path.Dir, which recognizes forward slashes only.
        for path in ["C:/diagrams/./main.d2", "D:/diagrams/main.d2"] {
            assert_eq!(go_abs_path(path), path);
        }
        let directory =
            std::env::temp_dir().join(format!("d2-rs-win-import-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("main.d2");
        std::fs::write(&source, b"...@nodes\n").unwrap();
        std::fs::write(directory.join("nodes.d2"), b"a: imported\n").unwrap();
        let output = directory.join("result.svg");
        run_document(&[
            "--layout=dagre".into(),
            source_test_path(&source),
            source_test_path(&output),
        ])
        .unwrap();
        assert!(
            std::fs::read_to_string(&output)
                .unwrap()
                .contains("imported")
        );
        // Backslashes and directory inputs have the same import limitation in
        // pinned Go. Preserve that diagnostic, including Windows' OS message.
        let error = run_document(&[
            "--layout=dagre".into(),
            source.to_string_lossy().into_owned(),
            source_test_path(&output),
        ])
        .unwrap_err();
        assert!(error.contains("failed to import \"nodes.d2\""), "{error}");
        let detail = d2_rs_core::windows_io::error_message(&io::Error::from_raw_os_error(2));
        assert!(error.ends_with(&detail), "{error}");
        let missing = source_test_path(&directory.join("missing.d2"));
        let error = run_document(&[
            "--layout=dagre".into(),
            missing.clone(),
            source_test_path(&output),
        ])
        .unwrap_err();
        assert!(
            error.contains(&format!("open {missing}: {detail}")),
            "{error}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn diagnostic_paths_follow_windows_volume_and_case_rules() {
        // Go 1.27 filepath.Rel via util-go/xmain.State.HumanPath.
        let cwd = Path::new(r"C:\work\diagrams");
        for (path, expected) in [
            (r"c:\WORK\other\input.d2", r"..\other\input.d2"),
            ("D:/diagrams/./input.d2", "D:/diagrams/./input.d2"),
            ("C:/work/diagrams/../input.d2", r"..\input.d2"),
        ] {
            assert_eq!(human_path_from(Path::new(path), cwd, None), expected);
        }
        assert_eq!(
            human_path_from(Path::new(r"C:\ſ\input.d2"), Path::new(r"c:\S"), None),
            "input.d2"
        );
    }

    #[test]
    #[cfg(unix)]
    fn diagnostic_paths_abbreviate_home_before_relativizing() {
        let home = Some(std::ffi::OsStr::new("/Users/example"));
        let cwd = Path::new("/Users/example/code/d2-rs");
        assert_eq!(
            human_path_from(Path::new("/var/tmp/output.svg"), cwd, home),
            "/var/tmp/output.svg"
        );
        assert_eq!(
            human_path_from(Path::new("/Users/example/code/d2/input.d2"), cwd, home),
            "../d2/input.d2"
        );
        assert_eq!(
            human_path_from(
                Path::new("/Users/example/input.d2"),
                Path::new("/var/tmp"),
                home
            ),
            "~/input.d2"
        );
    }

    #[test]
    fn svg_bundling_changes_only_image_references() {
        let svg = b"<svg id=\"fixed\"><a href=\"a&amp;b.svg\">link</a><image x=\"3\" href=\"a&amp;b.svg\" xlink:href=\"a&amp;b.svg\"/></svg>";
        let output = d2_rs_render::bundle_svg_images(
            svg,
            &BTreeMap::from([("a&b.svg".into(), b"<svg/>".to_vec())]),
        );
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.starts_with("<svg id=\"fixed\"><a href=\"a&amp;b.svg\">link</a><image x=\"3\" ")
        );
        assert_eq!(output.matches("data:image/svg+xml;base64,").count(), 2);
        assert_eq!(d2_rs_render::bundle_svg_images(svg, &BTreeMap::new()), svg);
    }

    #[test]
    fn svg_asset_discovery_preserves_first_rendered_occurrence() {
        assert_eq!(svg_image_hrefs(b"<image href=\"z.svg\"/><image href=\"a&amp;b.svg\"/><image href=\"z.svg\"/><image href=\"data:image/svg+xml;base64,AA==\"/>"), vec!["z.svg", "a&b.svg"]);
    }

    #[test]
    fn board_render_postorder_visits_children_and_wrapper_kinds_like_go() {
        let plan = source_board_plan(
            b"scenarios: {s: {x}}\nlayers: {a: {x; layers: {b: {y}}}; c: {z}}\n",
            "postorder.d2",
        )
        .unwrap();
        assert_eq!(
            plan.postorder_targets,
            vec!["layers.a.layers.b", "layers.a", "layers.c", "scenarios.s"]
        );
    }

    #[test]
    fn partial_svg_preserves_failed_local_asset_reference() {
        let source = b"a: {icon: /nonexistent-d2-rs-partial-output.svg}\n";
        let (svg, failures) = render_source_partial(source, &["--engine=dagre".into()]).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "/nonexistent-d2-rs-partial-output.svg");
        assert!(
            String::from_utf8(svg)
                .unwrap()
                .contains("/nonexistent-d2-rs-partial-output.svg")
        );
    }

    #[test]
    fn asset_collection_preserves_successes_and_failures() {
        let loaded =
            collect_asset_results_concurrent(["good", "bad"].map(str::to_owned).to_vec(), |name| {
                if name == "good" {
                    Ok(Some(vec![1, 2, 3].into()))
                } else {
                    Err(format!("missing {name}"))
                }
            });
        // Whether "good" was dispatched before "bad" failed is a worker race,
        // as in Go; a completed success is always retained.
        if let Some(bytes) = loaded.assets.get("good") {
            assert_eq!(bytes, &vec![1, 2, 3]);
        }
        assert_eq!(loaded.failures, vec![("bad".into(), "missing bad".into())]);
    }

    #[test]
    fn asset_workers_are_bounded_and_retain_every_result() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(16);
        let loaded = collect_asset_results_concurrent(
            (0..32).map(|index| index.to_string()).collect(),
            |icon| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                barrier.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = icon;
                Ok(Some(vec![1].into()))
            },
        );
        assert_eq!(peak.load(Ordering::SeqCst), 16);
        assert_eq!(loaded.assets.len(), 32);
        assert!(loaded.failures.is_empty());
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn asset_workers_stop_dispatching_after_the_first_failure() {
        // imgbundler.runResolverWorkersWithLimit cancels the worker context on
        // the first failure; a sequential worker set observes no later job.
        let loaded = collect_asset_results_concurrent(vec!["bad".into()], |_| Err("first".into()));
        assert_eq!(loaded.failures, vec![("bad".into(), "first".into())]);
        assert_eq!(
            report_asset_failures(&loaded.failures),
            "failed to bundle images: first"
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let icons = (0..64).map(|index| index.to_string()).collect::<Vec<_>>();
        let loaded = collect_asset_results_concurrent(icons, |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("failed".into())
        });
        // At most one job per worker was in flight when the first failed.
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) <= 16);
        assert_eq!(
            loaded.failures.len(),
            calls.load(std::sync::atomic::Ordering::SeqCst)
        );
    }

    #[test]
    fn local_asset_join_cleans_parents_without_resolving_symlinks() {
        assert_eq!(
            clean_asset_path(Path::new("./a/../missing.svg")),
            Path::new("missing.svg")
        );
        assert_eq!(
            clean_asset_path(Path::new("../../a/../missing.svg")),
            Path::new("../../missing.svg")
        );
        assert_eq!(clean_asset_path(Path::new("a/..")), Path::new("."));
        #[cfg(unix)]
        {
            assert_eq!(
                clean_asset_path(Path::new("/../a/../missing.svg")),
                Path::new("/missing.svg")
            );
            assert_eq!(
                local_asset_read_error(
                    Path::new("missing.svg"),
                    io::Error::from(io::ErrorKind::NotFound)
                ),
                "open missing.svg: no such file or directory"
            );
        }
    }

    #[test]
    fn remote_asset_context_cancellation_interrupts_headers_and_body() {
        for deadline in [false, true] {
            for send_headers in [false, true] {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
                let address = listener.local_addr().unwrap();
                let cancellation = d2_rs_core::CancellationToken::new();
                let token = cancellation.clone();
                let (started, request_started) = std::sync::mpsc::channel();
                let (stop, stopped) = std::sync::mpsc::channel();
                let server = std::thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = [0_u8; 2048];
                    // Only the arrival of a request matters, not its length.
                    #[allow(clippy::unused_io_amount)]
                    stream.read(&mut request).unwrap();
                    if send_headers {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nx").unwrap();
                    }
                    started.send(()).unwrap();
                    let _ = stopped.recv_timeout(std::time::Duration::from_secs(3));
                });
                let cancel = std::thread::spawn(move || {
                    request_started.recv().unwrap();
                    // Ensure the body case has entered its read before cancellation.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    if deadline {
                        token.expire()
                    } else {
                        token.cancel()
                    }
                });
                let _scope = watch::track_cancellation(cancellation);
                let start = std::time::Instant::now();
                let url = format!("http://{address}/slow.svg");
                let result =
                    fetch_remote_asset_with_timeout(&url, std::time::Duration::from_secs(2));
                let elapsed = start.elapsed();
                let _ = stop.send(());
                server.join().unwrap();
                cancel.join().unwrap();
                let cause = if deadline {
                    "context deadline exceeded"
                } else {
                    "context canceled"
                };
                let expected = if send_headers {
                    cause.to_owned()
                } else {
                    format!("Get {url:?}: {cause}")
                };
                assert_eq!(
                    result.unwrap_err(),
                    expected,
                    "body={send_headers}, deadline={deadline}"
                );
                assert!(
                    elapsed < std::time::Duration::from_millis(750),
                    "cancellation took {elapsed:?}"
                );
            }
        }
    }

    #[test]
    fn remote_asset_timeout_covers_headers_and_body() {
        for send_headers in [false, true] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                // Only the arrival of a request matters, not its length.
                #[allow(clippy::unused_io_amount)]
                stream.read(&mut request).unwrap();
                if send_headers {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            });
            let error = fetch_remote_asset_with_timeout(
                &format!("http://{address}/slow.svg"),
                std::time::Duration::from_millis(30),
            )
            .unwrap_err();
            if send_headers {
                assert_eq!(error, "context deadline exceeded");
            } else {
                assert_eq!(
                    error,
                    format!("Get \"http://{address}/slow.svg\": context deadline exceeded")
                );
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn remote_asset_loader_requires_http_200() {
        for (code, reason) in [
            (201, "Created"),
            (204, "No Content"),
            (206, "Partial Content"),
            (400, "Bad Request"),
            (404, "Not Found"),
            (500, "Internal Server Error"),
        ] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            });
            let error = fetch_remote_asset(&format!("http://{address}/status")).unwrap_err();
            assert_eq!(
                error,
                format!("expected status 200 but got {code} {code} {reason}")
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn remote_asset_loader_decodes_wrapped_and_raw_deflate_responses() {
        for wrapped in [false, true] {
            let body = b"deflated image bytes";
            let compressed = if wrapped {
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
                e.write_all(body).unwrap();
                e.finish().unwrap()
            } else {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::default());
                e.write_all(body).unwrap();
                e.finish().unwrap()
            };
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Encoding: deflate\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", compressed.len()).unwrap();
                stream.write_all(&compressed).unwrap();
            });
            assert_eq!(
                fetch_remote_asset(&format!("http://{address}/deflate")).unwrap(),
                body
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn remote_content_encoding_matches_brotli_and_truncated_stream_boundaries() {
        // Encoded with Python Brotli 1.2.0, independent of the Rust decoder.
        let brotli = vec![
            139, 8, 128, 98, 114, 111, 116, 108, 105, 32, 105, 109, 97, 103, 101, 32, 98, 121, 116,
            101, 115, 3,
        ];
        assert_eq!(
            decode_remote_content_encoding(brotli, "br").unwrap(),
            b"brotli image bytes"
        );
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(b"truncated image bytes").unwrap();
        let mut gzip = gzip.finish().unwrap();
        gzip.truncate(gzip.len() - 4);
        assert_eq!(
            decode_remote_content_encoding(gzip, "gzip").unwrap_err(),
            "unexpected EOF"
        );
        let mut deflate = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
        deflate.write_all(b"truncated image bytes").unwrap();
        let mut deflate = deflate.finish().unwrap();
        deflate.truncate(deflate.len() - 4);
        assert_eq!(
            decode_remote_content_encoding(deflate, "deflate").unwrap_err(),
            "unexpected EOF"
        );
    }

    #[test]
    fn remote_gzip_errors_keep_source_header_and_checksum_causes() {
        assert_eq!(
            decode_remote_content_encoding(b"invalid-gzip".to_vec(), "gzip").unwrap_err(),
            "gzip: invalid header"
        );
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"image bytes").unwrap();
        let mut bytes = encoder.finish().unwrap();
        let checksum = bytes.len() - 8;
        bytes[checksum] ^= 1;
        assert_eq!(
            decode_remote_content_encoding(bytes, "gzip").unwrap_err(),
            "gzip: invalid checksum"
        );
    }

    #[test]
    fn remote_content_encoding_decodes_stacks_aliases_and_rejects_unknown_encodings() {
        let source = b"stacked image bytes";
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(source).unwrap();
        let gzip = gzip.finish().unwrap();
        let mut deflate = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
        deflate.write_all(&gzip).unwrap();
        assert_eq!(
            decode_remote_content_encoding(deflate.finish().unwrap(), " GZIP, identity, Deflate ")
                .unwrap(),
            source
        );
        assert_eq!(
            decode_remote_content_encoding(gzip.clone(), "x-gzip").unwrap(),
            source
        );
        let concatenated = [gzip.clone(), gzip].concat();
        assert_eq!(
            decode_remote_content_encoding(concatenated, "gzip").unwrap(),
            [source.as_slice(), source.as_slice()].concat()
        );
        assert_eq!(
            decode_remote_content_encoding(source.to_vec(), "zstd").unwrap_err(),
            "unsupported content encoding \"zstd\""
        );
        assert!(decode_remote_content_encoding(vec![0; 8], "gzip").is_err());
    }

    #[test]
    fn remote_asset_loader_decodes_gzip_responses() {
        let source_body = b"compressed-image-bytes";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, source_body).unwrap();
        let compressed = encoder.finish().unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let received = std::io::Read::read(&mut stream, &mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..received]).to_ascii_lowercase();
            for header in [
                "chrome/131.0.0.0 safari/537.36",
                "accept-language: en-us,en;q=0.9",
                "accept-encoding: gzip, deflate, br",
                "dnt: 1",
                "upgrade-insecure-requests: 1",
                "sec-fetch-dest: image",
                "sec-fetch-mode: no-cors",
                "sec-fetch-site: cross-site",
            ] {
                assert!(request.contains(header), "missing {header}: {request}");
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                compressed.len()
            );
            std::io::Write::write_all(&mut stream, header.as_bytes()).unwrap();
            std::io::Write::write_all(&mut stream, &compressed).unwrap();
        });
        let url = format!("http://{address}/compressed");
        assert_eq!(fetch_remote_asset(&url).unwrap(), source_body);
        server.join().unwrap();
    }

    #[test]
    fn raster_asset_probe_rejects_malformed_and_oversized_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1_u32.to_be_bytes());
        png.extend_from_slice(&1_u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0, 0, 0, 0]);
        assert!(validate_asset_bytes("tiny.png", &png).is_ok());

        let mut too_wide = png.clone();
        too_wide[16..20].copy_from_slice(&32_769_u32.to_be_bytes());
        let error = validate_asset_bytes("wide.png", &too_wide).unwrap_err();
        assert!(error.contains("32768-pixel limit"));

        let mut too_many_pixels = png;
        too_many_pixels[16..20].copy_from_slice(&8_192_u32.to_be_bytes());
        too_many_pixels[20..24].copy_from_slice(&8_193_u32.to_be_bytes());
        let error = validate_asset_bytes("large.png", &too_many_pixels).unwrap_err();
        assert!(error.contains("decoded pixels"));

        let error = validate_asset_bytes("broken.png", b"not-an-image").unwrap_err();
        assert!(error.contains("not a valid raster image"));
    }

    #[test]
    fn embedded_raster_data_uris_use_the_same_probe_boundary() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1_u32.to_be_bytes());
        png.extend_from_slice(&1_u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0, 0, 0, 0]);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        let uri = format!("data:image/png;base64,{encoded}");
        assert!(validate_data_uri_asset(&uri).is_ok());
        assert!(validate_data_uri_asset("data:image/png;base64,broken").is_err());
        assert!(validate_data_uri_asset("data:image/svg+xml,%3Csvg%3E%3C/svg%3E").is_ok());
        assert!(validate_data_uri_asset("data:image/avif;base64,AA==").is_err());
    }

    #[test]
    fn render_svg_bundles_local_images_even_when_remote_bundling_is_disabled() {
        let base = std::env::temp_dir().join(format!(
            "d2-rs-assets-{}-{}",
            std::process::id(),
            "external"
        ));
        std::fs::create_dir_all(&base).unwrap();
        let source_path = base.join("diagram.d2");
        std::fs::write(
            base.join("icon.svg"),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"1\" height=\"1\"/></svg>",
        )
        .unwrap();
        let output = run(
            &[
                "render-svg".into(),
                "--bundle=false".into(),
                format!("--path={}", source_path.display()),
            ],
            b"a: {shape: image; icon: icon.svg}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("href=\"icon.svg\""));
        assert!(output.contains("data:image/svg+xml;base64,"));
        let (svg, preview) = render_source_format_with_preview(
            b"a: {shape: image; icon: icon.svg}\n",
            &[
                "--bundle=false".into(),
                format!("--path={}", source_path.display()),
            ],
            ExportFormat::Svg,
            None,
            None,
            true,
            false,
        )
        .unwrap();
        assert!(
            String::from_utf8(svg)
                .unwrap()
                .contains("data:image/svg+xml;base64,")
        );
        assert!(
            String::from_utf8(preview.unwrap())
                .unwrap()
                .contains("data:image/svg+xml;base64,")
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn raster_exports_resolve_local_assets_even_when_svg_bundling_is_disabled() {
        let base =
            std::env::temp_dir().join(format!("d2-rs-assets-{}-{}", std::process::id(), "raster"));
        std::fs::create_dir_all(&base).unwrap();
        let source_path = base.join("diagram.d2");
        std::fs::write(
            base.join("icon.svg"),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><rect width=\"1\" height=\"1\" fill=\"red\"/></svg>",
        )
        .unwrap();
        let input = b"a: {shape: image; icon: icon.svg}\n";
        let path_arg = format!("--path={}", source_path.display());
        let bundled =
            render_source_format(input, std::slice::from_ref(&path_arg), ExportFormat::Png)
                .unwrap();
        let external = render_source_format(
            input,
            &["--bundle=false".into(), path_arg],
            ExportFormat::Png,
        )
        .unwrap();
        assert_eq!(bundled, external);
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn unavailable_raster_image_matches_complete_original_frame_and_svg_stays_fatal() {
        // Original Go CLI with built-in Dagre.
        let input = b"a: \"\" {shape: image; icon: absent.png; width: 64; height: 64}\n";
        let args = vec!["--engine=dagre".to_owned()];
        let actual = render_source_format(input, &args, ExportFormat::Png).unwrap();
        fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
            let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            decoder
                .set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().unwrap();
            let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut bytes).unwrap();
            bytes.truncate(info.buffer_size());
            // Both encoders emit RGB for this opaque scene.
            assert_eq!(info.color_type, png::ColorType::Rgb);
            (info.width, info.height, bytes)
        }
        assert_eq!(
            decode(&actual),
            decode(d2_rs_testdata::oracle_fixture_bytes!(
                "tests/fixtures/unavailable-image-v090.png"
            ))
        );
        assert!(render_source(input, &args).is_err());
    }

    #[test]
    fn render_svg_text_shape_is_label_only() {
        let output = run(&["render-svg".into()], b"a: hello {shape: text}\n").unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(">hello</text>"));
        assert!(!output.contains("<rect x=\"100\""));
        assert!(!output.contains("rx=\"4\""));
    }

    #[test]
    fn render_svg_short_markdown_uses_v090_text_minimum() {
        let output = run(&["render-svg".into()], b"a: |md\nh\n|\n").unwrap();
        let output = String::from_utf8(output).unwrap();
        // d2graph.MIN_SHAPE_SIZE is 5px; the nine-pixel glyph bounds are
        // therefore preserved instead of being widened to the 12px table
        // minimum used by unrelated class/SQL envelopes. Compare dimensions
        // numerically because SVG serialization precision is not semantic.
        let markdown_svg = output.split("<svg x=").nth(1).unwrap();
        let attributes = format!("x={}", markdown_svg.split('>').next().unwrap());
        let number = |attribute: &str| {
            attributes
                .split_once(&format!("{attribute}=\""))
                .unwrap()
                .1
                .split('"')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap_or_else(|error| panic!("{attribute} in {attributes:?}: {error}"))
        };
        assert_eq!(
            (number("x"), number("y"), number("width"), number("height")),
            (0.0, 0.0, 9.0, 24.0)
        );
    }

    #[test]
    fn render_svg_class_shape_emits_header_and_member_rows() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"foo: {shape: class; +name: string; -id: int}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("class=\"class_header d2-rs-fill-N1\""));
        assert!(output.contains(">+</text>"));
        assert!(output.contains(">name</text>"));
        assert!(output.contains(">string</text>"));
        assert!(output.contains(">-</text>"));
        assert!(output.contains(">id</text>"));
        assert!(output.contains(">int</text>"));
        // The class target owns its title/member text; the generic label pass
        // must not emit a second centered `foo` text node.
        assert_eq!(output.matches(">foo</text>").count(), 1);

        let empty = run(&["render-svg".into()], b"empty: {shape: class}\n").unwrap();
        let empty = String::from_utf8(empty).unwrap();
        assert!(empty.contains("class=\"class_header d2-rs-fill-N1\""));
        assert!(empty.contains("stroke-width=\"2\""));

        let methods = run(
            &["render-svg".into()],
            b"foo: {shape: class; +name: string; +get(): string; -set(x): void}\n",
        )
        .unwrap();
        let methods = String::from_utf8(methods).unwrap();
        assert!(methods.contains(">get()</text>"));
        assert!(methods.contains(">set(x)</text>"));
        // d2svg.drawClass emits one thin compartment rule; the outer
        // rectangle already supplies the bottom border.
        assert_eq!(methods.matches("<line ").count(), 1);
        assert!(methods.contains("stroke-width=\"1\""));

        let explicit = run(
            &["render-svg".into()],
            b"foo: {shape: class; style.font-size: 30; +name: string}\n",
        )
        .unwrap();
        let explicit = String::from_utf8(explicit).unwrap();
        assert!(explicit.contains("width=\"277\" height=\"174\""));
        assert!(explicit.contains("width=\"277\" height=\"116\""));
        assert!(explicit.contains("font-size=\"34\""));
        assert!(explicit.contains("font-size=\"30\""));
    }

    #[test]
    fn render_svg_sql_table_emits_header_types_and_constraints() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"users: {shape: sql_table; id: int { constraint: primary_key}; name: string}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("class=\"class_header d2-rs-fill-N1\""));
        assert_eq!(output.matches(">users</text>").count(), 1);
        assert!(output.contains(">id</text>"));
        assert!(output.contains(">int</text>"));
        assert!(output.contains(">PK</text>"));
        assert!(output.contains(">name</text>"));
        assert!(output.contains(">string</text>"));
        assert!(output.contains("d2-rs-text-B2"));
        assert!(output.contains("d2-rs-text-N2"));
        assert!(output.contains("stroke-width=\"2\""));
    }

    #[test]
    fn render_svg_emits_reserved_legend_without_layout_nodes() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            br#"vars: {
  d2-legend: "Legend" {
    user: User {shape: person}
    database: Database {shape: cylinder}
    user -> database: Reads {style.stroke: blue; style.stroke-dash: 5}
    hidden: Hidden {style.opacity: 0}
  }
}
user: User
database: Database
user -> database: Uses
"#,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("class=\"d2-rs-legend\""));
        assert!(output.contains(">Legend</text>"));
        assert!(output.contains(">User</text>"));
        assert!(output.contains(">Database</text>"));
        assert!(output.contains(">Reads</text>"));
        assert!(output.contains("stroke=\"blue\""));
        assert!(output.contains("stroke-dasharray="));
        // Legend entries are target metadata and must not become ordinary
        // layout nodes or duplicate the main diagram's labels.
        assert_eq!(output.matches(">Hidden</text>").count(), 0);
        assert_eq!(output.matches(">User</text>").count(), 2);
        assert_eq!(output.matches(">Database</text>").count(), 2);
    }

    #[test]
    fn render_svg_applies_patterns_transforms_underline_and_animated_edges() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a: hello world {style.text-transform: uppercase; style.underline: true; style.fill-pattern: dots; style.shadow: true; style.multiple: true; style.double-border: true}\nb: box {style.3d: true}\na -> b: edge {style.animated: true; style.text-transform: uppercase}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("d2-rs-pattern-dots"));
        assert!(output.contains("HELLO WORLD"));
        assert!(output.contains("text-decoration=\"underline\""));
        assert!(output.contains("animation=\"dashdraw"));
        assert!(output.contains("d2-rs-shadow-filter"));
        assert!(output.matches("fill=\"none\"").count() >= 2);
        assert!(output.contains(">EDGE</text>"));
    }

    #[test]
    fn render_svg_emits_gradient_definitions_for_source_fills() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a: gradient {style.fill: linear-gradient(to right, red, blue 75%)}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("<linearGradient id=\"grad-"));
        assert!(output.contains("x1=\"0%\" y1=\"50%\" x2=\"100%\" y2=\"50%\""));
        assert!(output.contains("stop-color=\"red\""));
        assert!(output.contains("stop-color=\"blue\""));
        assert!(output.contains("fill=\"url(#grad-"));
    }

    #[test]
    fn render_svg_honors_explicit_and_source_sketch_flags() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let explicit = run(
            &["render-svg".into(), "--sketch".into()],
            b"a: box\nb: box\na -> b\n",
        )
        .unwrap();
        let explicit = String::from_utf8(explicit).unwrap();
        // The Source-qualified rectangle painter emits fill and outline,
        // rather than the former approximate overlay's class count.
        assert_eq!(
            explicit.matches("data-d2-rs-sketch-set=\"rough\"").count(),
            4
        );
        assert!(explicit.contains("sketch-streak-overlay"));
        assert!(explicit.contains("class=\"d2-rs-theme d2-rs-sketch\""));

        let configured = run(
            &["render-svg".into()],
            b"vars: {d2-config: {sketch: true}}\na: box\n",
        )
        .unwrap();
        let configured = String::from_utf8(configured).unwrap();
        assert_eq!(
            configured
                .matches("data-d2-rs-sketch-set=\"rough\"")
                .count(),
            2
        );
        assert!(configured.contains("sketch-streak-overlay"));
        assert!(configured.contains("class=\"d2-rs-theme d2-rs-sketch\""));
    }

    #[test]
    fn render_svg_honors_explicit_inside_label_position() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a: {label: hello; label-position: inside-top-left}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        // d2svg writes ordinary node labels from their measured label box and
        // keeps the text anchor centered even for corner positions.
        assert!(output.contains("text-anchor=\"middle\""));
    }

    #[test]
    fn render_svg_renders_valid_edge_label_near_map() {
        // Contract of the legacy native SVG renderer (D2_RS_SVG_RENDERER=legacy).
        let _legacy = super::LegacySvgGuard::new();
        let output = run(
            &["render-svg".into()],
            b"a -> b: {label: link { near: outside-top-right }}\n",
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(">link</text>"));
        // D2 positions route labels through a measured rectangle and centers
        // the SVG text anchor even for an outside corner position.
        assert!(output.contains("text-anchor=\"middle\""));
        assert!(output.contains("font-size=\"16\""));
        assert!(output.contains("font-style=\"italic\""));
    }

    #[test]
    fn render_svg_preserves_named_arrowhead_markers() {
        let output = run(
            &["render-svg".into()],
            br#"a -> b: "link" {
  target-arrowhead: { shape: diamond }
}
"#,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("id=\"mk-d2-"));
        assert!(output.contains("marker-end=\"url(#mk-d2-"));
    }

    #[test]
    fn source_errors_are_reported_without_partial_stdout() {
        let error = run(&["format".into()], b"a: {").unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn document_compile_errors_use_d2_cli_context_and_human_path() {
        let path = std::env::current_dir()
            .unwrap()
            .join("markdown-raw-html-unsupported.d2");
        assert_eq!(
            document_compile_error(&path, "unsupported HTML".into()),
            "err: failed to compile markdown-raw-html-unsupported.d2: unsupported HTML"
        );
    }

    #[test]
    fn cli_error_writer_prefixes_ordered_seed_failures_on_every_line() {
        let path = std::env::current_dir().unwrap().join("oversized.d2");
        let error = LayoutError::AllSeedAttemptsFailed {
            attempts: [17, -17, 0]
                .into_iter()
                .map(|seed| {
                    (
                        seed,
                        LayoutError::StageDimensions {
                            stage: "NodePlacement",
                            width: 30001.0,
                            height: 80.0,
                        },
                    )
                })
                .collect(),
        };
        let wrapped = document_compile_error(&path, error.to_string());
        let mut stderr = Vec::new();
        write_cli_error(&mut stderr, &wrapped).unwrap();
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "err: failed to compile oversized.d2: all TALA seed attempts failed: seed 17: layout invariant violated: Dimensions w:30001, h:80 reached after stage NodePlacement\n\
             err: seed -17: layout invariant violated: Dimensions w:30001, h:80 reached after stage NodePlacement\n\
             err: seed 0: layout invariant violated: Dimensions w:30001, h:80 reached after stage NodePlacement\n"
        );
        assert_eq!(
            watch_error_detail(wrapped, Some(path.to_str().unwrap())),
            error.to_string(),
            "watch receives the underlying error before stderr decoration"
        );
    }

    #[test]
    fn cli_error_writer_preserves_blank_lines_and_existing_final_newline() {
        let mut stderr = Vec::new();
        write_cli_error(&mut stderr, "err: first line\n\nlast line\n").unwrap();
        assert_eq!(stderr, b"err: first line\nerr:\nerr: last line\n");
    }

    #[test]
    fn watch_errors_remove_only_the_document_cli_prefix() {
        let path = std::env::current_dir()
            .unwrap()
            .join("input: with colon.d2");
        let wrapped = document_compile_error(&path, "invalid D2 source".into());
        assert_eq!(
            watch_error_detail(wrapped, Some(path.to_str().unwrap())),
            "invalid D2 source"
        );
        assert_eq!(
            watch_error_detail("unknown option --bad".into(), Some("input.d2")),
            "unknown option --bad"
        );
    }

    #[test]
    fn validate_matches_d2_success_message() {
        assert_eq!(
            validation_success_message("Input"),
            "Success! [Input] is valid D2.\n"
        );
        assert!(run(&["validate".into()], b"a: {").is_err());
    }

    #[test]
    fn validate_matches_d2_parse_only_boundary() {
        assert!(
            run(
                &["validate".into()],
                b"vars: {d2-config: {theme-id: 999}}; a: b\n",
            )
            .is_ok()
        );
        assert!(run(&["validate".into()], b"grid-columns: 1\ncell -> cell\n").is_ok());
    }

    #[test]
    fn image_shapes_require_an_icon_like_d2() {
        let error = run(&["render-svg".into()], b"a: {shape: image}\n").unwrap_err();
        assert!(
            error.contains("image shape must include an \"icon\" field"),
            "{error}"
        );
    }

    #[test]
    fn image_shapes_reject_children_like_d2() {
        let error = run(
            &["render-svg".into()],
            b"a: {shape: image; icon: placeholder.svg; child: value}\n",
        )
        .unwrap_err();
        assert!(
            error.contains("image shapes cannot have children."),
            "{error}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod authored_label_position_source_contract {
    #[test]
    fn authored_label_positions_match_pristine_tala_for_all_shape_positions() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/authored-label-position-source-v090.json"
        ))
        .unwrap();
        let mut differences = Vec::new();
        for row in rows.as_array().unwrap() {
            let (graph, result, _, _) = super::source_layout(
                row["source"].as_str().unwrap().as_bytes(),
                &["--seeds=1".into()],
            )
            .unwrap();
            let actual: std::collections::BTreeMap<_, _> = graph
                .nodes()
                .map(|(id, node)| {
                    (
                        node.external_id.clone(),
                        result
                            .node_labels
                            .get(&id)
                            .map(|state| state.position.d2_name().to_owned()),
                    )
                })
                .collect();
            let expected: std::collections::BTreeMap<_, _> = row["positions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().map(str::to_owned)))
                .collect();
            if actual != expected {
                differences.push((row["name"].clone(), actual, expected));
            }
        }
        assert!(
            differences.is_empty(),
            "pristine authored label-position differences: {differences:#?}"
        );
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod badge_paint_order_source_contract {
    #[test]
    fn badges_match_pristine_paint_order_and_ancestor_contracts() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/badge-paint-order-source-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let svg = super::render_source(
                row["source"].as_str().unwrap().as_bytes(),
                &["--seeds=1".into()],
            )
            .unwrap();
            let document = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
            let elements: Vec<_> = document
                .descendants()
                .filter(|node| node.is_element())
                .collect();
            let badges: Vec<_> = elements
                .iter()
                .enumerate()
                .filter(|(_, node)| {
                    node.attribute("class").is_some_and(|class| {
                        class.split_whitespace().any(|name| name == "appendix-icon")
                    })
                })
                .collect();
            let last_paint = elements
                .iter()
                .enumerate()
                .filter(|(_, node)| {
                    node.attribute("fill").is_some_and(|fill| {
                        ["#ff0000", "#00ff00", "#0000ff", "#ffff00"].contains(&fill)
                    }) || node.attribute("stroke") == Some("#ff00ff")
                })
                .map(|(index, _)| index)
                .max()
                .unwrap();
            let ancestor_count: usize = badges
                .iter()
                .map(|(_, node)| {
                    node.ancestors()
                        .skip(1)
                        .filter(|ancestor| {
                            ancestor.has_tag_name("a")
                                || ancestor.attribute("opacity").is_some()
                                || ancestor
                                    .attribute("style")
                                    .is_some_and(|style| style.contains("opacity:"))
                        })
                        .count()
                })
                .sum();
            let actual = serde_json::json!({"badge_count":badges.len(),"badges_after_all_colored_paint":badges.first().is_some_and(|(index,_)| *index > last_paint),"badge_links_or_opacity_ancestors":ancestor_count});
            assert_eq!(actual, row["expected"], "{}", row["name"]);
        }
    }
}

#[cfg(test)]
mod intrinsic_math_measure_contract {
    use super::*;
    use compiler::GraphMeasurer;
    #[test]
    fn math_measurers_keep_original_intrinsic_sizes_across_font_sizes() {
        let paths = FontPaths::default();
        let bundled = load_graph_measurer_with_paths(&paths).unwrap().unwrap();
        for measurer in [&bundled as &dyn GraphMeasurer, &FallbackGraphMeasurer] {
            for font in [16, 20, 30] {
                assert_eq!(
                    measurer.measure_latex_label_at_size("1 + 1", font),
                    Size {
                        width: 41.0,
                        height: 14.0
                    }
                );
                assert_eq!(
                    measurer.measure_latex_at_size("1 + 1", font),
                    Size {
                        width: 86.0,
                        height: 59.0
                    }
                );
            }
        }
    }
}

#[cfg(test)]
mod elk_wire_contract;
