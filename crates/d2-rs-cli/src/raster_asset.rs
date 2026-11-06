// SPDX-License-Identifier: MPL-2.0
//! Source D2 master 6a7b48378 lib/imageasset (Resolver.Resolve/classifySource/
//! loadLocal/loadHTTP, SourceError), lib/localfile (Unrestricted policy) and
//! d2scenebuild.preflightAssets. The same resolver backs SVG bundling
//! (imgbundler.BundleWithResolver) and native raster scenes; acquisition
//! errors retain their availability kind until the caller's boundary.
use super::*;

enum Acquired {
    Data(AssetData),
    Unavailable,
}

/// One probed imageasset.Resource: its retained bytes and MIME type.
pub(super) struct Resolved {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// Resolution outcome. `Unavailable` carries the full SourceError text of an
/// imageasset.ErrUnavailable failure (missing file, transport failure,
/// non-200 status), which raster preflight tolerates and imgbundler reports.
pub(super) enum Outcome {
    Data(Resolved),
    Unavailable(String),
}

fn context() -> Result<(), String> {
    match watch::active_cancellation_token().and_then(|token| token.error()) {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

fn go_quote(value: &str) -> String {
    d2_rs_core::quote_go_bytes(value.as_bytes())
}

/// imageasset.SourceError.Error.
fn source_error(op: &str, label: &str, detail: impl std::fmt::Display) -> String {
    format!("imageasset: {op} {}: {detail}", go_quote(label))
}

/// imageasset.uriScheme.
pub(super) fn uri_scheme(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if *byte == b':' {
            return Some(&value[..index]);
        }
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')) {
            return None;
        }
    }
    None
}

/// url.URL.String after imageasset.displaySource removes userinfo, query and
/// fragment (and url.Parse lowercases the scheme). Opaque URIs are redacted.
fn display_uri(scheme: &str, value: &str) -> String {
    let rest = &value[scheme.len() + 1..];
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let scheme = scheme.to_ascii_lowercase();
    if let Some(hierarchical) = rest.strip_prefix("//") {
        let (authority, path) = hierarchical
            .find('/')
            .map_or((hierarchical, ""), |slash| hierarchical.split_at(slash));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        format!("{scheme}://{host}{path}")
    } else if rest.is_empty() || rest.starts_with('/') {
        format!("{scheme}:{rest}")
    } else {
        format!("{scheme}:<redacted>")
    }
}

/// imageasset.displaySource for non-data sources.
fn display_source(source: &str) -> String {
    if source.starts_with("//") {
        return "<network-path reference>".into();
    }
    if source.len() > 64 * 1024 {
        return "<oversized source locator>".into();
    }
    match uri_scheme(source) {
        Some(scheme) if !Path::new(source).is_absolute() => {
            if source.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
                "<malformed URI>".into()
            } else {
                display_uri(scheme, source)
            }
        }
        _ => source.to_owned(),
    }
}

/// imageasset's MIME type for a probed resource (probe.go mimeForFormat and
/// svgResource).
fn resource_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "image/svg+xml"
    }
}

fn probe(
    icon: &str,
    label: &str,
    mime_hint: Option<&str>,
    bytes: &[u8],
) -> Result<Resolved, String> {
    let bytes = d2_rs_export::prepare_image_asset_bytes(icon, mime_hint, bytes)
        .map_err(|error| source_error("probe", label, error))?;
    // imageasset/probe.go probeRaster: rasterimage.Config validates the WebP
    // container, including an animation's first frame, before any limit.
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        d2_rs_math::svg_image_config::webp_config(&bytes, None).map_err(|error| {
            source_error("probe", label, format!("malformed WEBP image: {error}"))
        })?;
    }
    validate_asset_bytes("resolved-image", &bytes)
        .map_err(|error| source_error("probe", label, error))?;
    Ok(Resolved {
        mime: resource_mime(&bytes),
        bytes,
    })
}

/// os.PathError text for an open failure, as Go prints it on Unix.
fn path_error_detail(error: &io::Error) -> String {
    #[cfg(unix)]
    return match error.kind() {
        io::ErrorKind::NotFound => "no such file or directory".to_owned(),
        io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        io::ErrorKind::IsADirectory => "is a directory".to_owned(),
        _ => error.to_string(),
    };
    #[cfg(not(unix))]
    error.to_string()
}

/// localfile.Unrestricted().Open: a nonblocking read-only open verified from
/// the opened descriptor, so FIFOs, devices and directories are rejected
/// without blocking.
fn open_regular(path: &Path) -> Result<std::fs::File, (bool, String)> {
    let display = path.to_string_lossy();
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| {
        (
            error.kind() == io::ErrorKind::NotFound,
            format!("open {display}: {}", path_error_detail(&error)),
        )
    })?;
    let info = file.metadata().map_err(|error| {
        (
            false,
            format!(
                "localfile: stat opened file {}: {}",
                go_quote(&display),
                path_error_detail(&error)
            ),
        )
    })?;
    if !info.is_file() {
        return Err((
            false,
            format!("localfile: {} is not a regular file", go_quote(&display)),
        ));
    }
    Ok(file)
}

/// imageasset.Resolver.Resolve with localfile.Unrestricted and the process
/// network policy. `base` is the resolver's BaseDir (the input's directory).
pub(super) fn resolve(icon: &str, base: &Path) -> Result<Outcome, String> {
    context()?;
    if icon.is_empty() {
        return Err(source_error("classify source", "", "empty source"));
    }
    if icon.len() > 64 * 1024 {
        return Err(source_error(
            "classify source",
            &display_source(icon),
            format!("source locator bytes {} exceeds limit 65536", icon.len()),
        ));
    }
    if icon.starts_with("//") {
        return Err(source_error(
            "classify source",
            &display_source(icon),
            "network-path references beginning with // are unsupported; use an explicit http:// or https:// URL",
        ));
    }
    if has_http_asset_scheme(icon) {
        let canonical = icon.split('#').next().unwrap_or(icon);
        let parsed = reqwest::Url::parse(canonical).map_err(|_| {
            source_error("classify source", &display_source(icon), "invalid HTTP URL")
        })?;
        if parsed.host_str().is_none_or(str::is_empty) {
            return Err(source_error(
                "classify source",
                &display_source(icon),
                "HTTP URL has no host",
            ));
        }
        let label = display_source(canonical);
        // Separate resolver namespace from the legacy SVG cache; the network
        // policy is part of the effective namespace (imageasset.New).
        let policy = if http_asset::allow_private_network() {
            "private-network"
        } else {
            "public-only"
        };
        let key = format!("d2cli/imageasset/v1/default-http:{policy}:{canonical}");
        if let Some(asset) = watch::cached_remote_asset_data(&key) {
            return Ok(Outcome::Data(Resolved {
                mime: resource_mime(&asset.bytes),
                bytes: asset.bytes,
            }));
        }
        let fetched = http_asset::fetch_raster(
            canonical,
            std::time::Duration::from_secs(60),
            watch::active_cancellation_token(),
        );
        context()?;
        let (bytes, encoding, content_type) = match fetched {
            Ok(result) => result,
            Err(error) if error.unavailable => {
                return Ok(Outcome::Unavailable(source_error(
                    "load",
                    &label,
                    error.message,
                )));
            }
            Err(error) => return Err(source_error("load", &label, error.message)),
        };
        let bytes = decode_remote_content_encoding(bytes, &encoding)
            .map_err(|error| source_error("load", &label, error))?;
        let resolved = probe(icon, &label, content_type.as_deref(), &bytes)?;
        context()?;
        watch::cache_remote_asset_data(
            &key,
            &AssetData {
                bytes: resolved.bytes.clone(),
                content_type,
            },
        );
        return Ok(Outcome::Data(resolved));
    }
    if let Some(scheme) = uri_scheme(icon).filter(|_| !Path::new(icon).is_absolute()) {
        return Err(source_error(
            "classify source",
            &display_source(icon),
            format!("unsupported URI scheme {}", go_quote(scheme)),
        ));
    }
    let path = Path::new(icon);
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    let resolved = clean_asset_path(&joined);
    watch::record_dependency(&resolved);
    let display = resolved.to_string_lossy().into_owned();
    let load_error = |detail: String| source_error("load", icon, detail);
    let file = match open_regular(&resolved) {
        Ok(file) => file,
        Err((missing, detail)) => {
            context()?;
            let message = load_error(format!("open local file {}: {detail}", go_quote(&display)));
            return if missing {
                Ok(Outcome::Unavailable(message))
            } else {
                Err(message)
            };
        }
    };
    let length = file.metadata().map(|info| info.len()).unwrap_or_default();
    if length > MAX_ASSET_BYTES {
        return Err(load_error(format!(
            "fetched bytes {length} exceeds limit {MAX_ASSET_BYTES}"
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| load_error(error.to_string()))?;
    if bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err(load_error(format!(
            "fetched bytes {} exceeds limit {MAX_ASSET_BYTES}",
            bytes.len()
        )));
    }
    context()?;
    probe(icon, icon, None, &bytes).map(Outcome::Data)
}

/// imageasset.New: BaseDir is filepath.Abs(filepath.Dir(inputPath)), or the
/// working directory for stdin.
pub(super) fn resolver_base(source_args: &[String]) -> PathBuf {
    let source_path = source_args
        .iter()
        .find_map(|arg| arg.strip_prefix("--path=").filter(|path| *path != "-"));
    let base = source_path
        .map(|path| Path::new(path).parent().unwrap_or_else(|| Path::new(".")))
        .unwrap_or_else(|| Path::new("."));
    let absolute = if base.is_absolute() {
        base.to_owned()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(base))
            .unwrap_or_else(|_| base.to_owned())
    };
    clean_asset_path(&absolute)
}

fn acquire(icon: &str, base: &Path) -> Result<Acquired, String> {
    match resolve(icon, base)? {
        Outcome::Data(resolved) => Ok(Acquired::Data(AssetData {
            bytes: resolved.bytes,
            content_type: Some(resolved.mime.to_owned()),
        })),
        Outcome::Unavailable(_) => Ok(Acquired::Unavailable),
    }
}

/// d2cli/raster.go gifSceneAssetOptions: GIF sessions cap each raster
/// resource at gifMaxFramePixels decoded pixels instead of rasterMaxPixels.
pub(super) const GIF_MAX_DECODED_PIXELS: u64 = 2 * 1024 * 1024;

/// imageasset/probe.go validateDecodedLimits for a session-specific
/// MaxDecodedPixels below the resolver's general raster limit.
fn validate_session_decoded_pixels(
    icon: &str,
    bytes: &[u8],
    max_decoded_pixels: Option<u64>,
) -> Result<(), String> {
    let Some(limit) = max_decoded_pixels else {
        return Ok(());
    };
    if resource_mime(bytes) == "image/svg+xml" {
        return Ok(());
    }
    let Ok(size) = imagesize::blob_size(bytes) else {
        return Ok(());
    };
    let pixels = (size.width as u64).saturating_mul(size.height as u64);
    if pixels <= limit {
        return Ok(());
    }
    let label = if has_http_asset_scheme(icon) {
        display_source(icon.split('#').next().unwrap_or(icon))
    } else {
        icon.to_owned()
    };
    Err(source_error(
        "probe",
        &label,
        format!("decoded pixels {pixels} exceeds limit {limit}"),
    ))
}

pub(super) fn load_into_options(
    source_args: &[String],
    graph: &Graph,
    styles: &BTreeMap<d2_rs_core::NodeId, compiler::SourceNodeStyle>,
    options: &mut RenderOptions,
    max_decoded_pixels: Option<u64>,
) -> Result<(), String> {
    let base = resolver_base(source_args);
    let base = base.as_path();
    let mut nodes = graph.nodes().collect::<Vec<_>>();
    nodes.sort_by_key(|(_, node)| {
        let mut parent = node.parent;
        let mut depth = 0;
        while let Some(id) = parent {
            depth += 1;
            parent = graph.node(id).and_then(|node| node.parent);
        }
        depth
    });
    let mut inputs = Vec::new();
    for (id, node) in nodes {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let Some(icon) = style.icon.as_deref() else {
            continue;
        };
        let opacity = style
            .opacity
            .as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.);
        if opacity == 0.
            && !matches!(
                node.shape,
                d2_rs_core::ShapeKind::Image
                    | d2_rs_core::ShapeKind::Class
                    | d2_rs_core::ShapeKind::SqlTable
            )
        {
            continue;
        }
        inputs.push((
            format!(
                "shape {}",
                d2_rs_core::quote_go_bytes(node.external_id.as_bytes())
            ),
            icon,
        ));
    }
    for (id, _) in graph.edges() {
        // Preflight follows target presence, including url.Parse("").
        // The nonempty accessor is only appropriate when painting an image.
        if let Some(icon) = graph
            .edge_icons
            .get(id.0 as usize)
            .and_then(Option::as_deref)
        {
            let name = d2_rs_core::edge_abs_id(graph, id).expect("live authored edge");
            inputs.push((
                format!("connection {}", d2_rs_core::quote_go_bytes(name.as_bytes())),
                icon,
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for (object, icon) in inputs {
        context()?;
        if !seen.insert(icon) {
            continue;
        }
        if icon.starts_with("data:") {
            validate_data_uri_asset(icon)
                .map_err(|e| format!("scene: {object} image asset: {e}"))?;
            continue;
        }
        match acquire(icon, base).map_err(|e| format!("scene: {object} image asset: {e}"))? {
            Acquired::Data(asset) => {
                validate_session_decoded_pixels(icon, &asset.bytes, max_decoded_pixels)
                    .map_err(|e| format!("scene: {object} image asset: {e}"))?;
                options.asset_data.insert(icon.to_owned(), asset.bytes);
            }
            Acquired::Unavailable => {
                options.unavailable_images.insert(icon.to_owned());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_icons_keep_source_target_presence_and_exact_raster_errors() {
        for (source, expected) in [
            (
                b"a -> b: {icon: \"\"}".as_slice(),
                "scene: connection \"(a -> b)[0]\" image asset: imageasset: classify source \"\": empty source",
            ),
            (
                b"a: {icon: \"\"}".as_slice(),
                "scene: shape \"a\" image asset: imageasset: classify source \"\": empty source",
            ),
        ] {
            let error = render_source_format(source, &["--engine=dagre".into()], ExportFormat::Png)
                .unwrap_err();
            assert_eq!(error, expected);
        }
    }

    #[test]
    fn local_unavailability_retries_and_malformed_or_directory_stays_fatal() {
        let root =
            std::env::temp_dir().join(format!("d2-raster-unavailable-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let name = "missing.svg";
        assert!(matches!(
            acquire(name, &root).unwrap(),
            Acquired::Unavailable
        ));
        std::fs::write(
            root.join(name),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"/>",
        )
        .unwrap();
        assert!(matches!(acquire(name, &root).unwrap(), Acquired::Data(_)));
        std::fs::write(root.join("malformed.png"), b"not an image").unwrap();
        assert!(acquire("malformed.png", &root).is_err());
        assert!(acquire(".", &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
