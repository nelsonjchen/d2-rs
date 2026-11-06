// SPDX-License-Identifier: MPL-2.0
//! D2 master 6a7b48378 lib/imgbundler/imgbundler.go: bounded generated-SVG
//! scanning, cumulative replacement budgets and transactional replacement.
//! Acquisition/policy belongs to the caller's image resolver.
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_REFERENCES: usize = 4096;
pub const MAX_REFERENCE_BYTES: usize = 64 << 10;
pub const MAX_BUNDLED_BYTES: i64 = 512 << 20;
pub const MAX_OUTPUT_BYTES: i64 = 512 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageMatch {
    pub start: usize,
    pub end: usize,
    pub href_start: usize,
    pub href_end: usize,
}

#[derive(Default, Clone, Copy)]
pub struct Options {
    pub local: bool,
    pub remote: bool,
}

/// Decode only for classification/resolution; retain raw hrefs as replacement
/// keys so entity spellings and duplicate occurrences survive the scan.
pub fn decode_href(href: &str) -> String {
    String::from_utf8_lossy(&d2_rs_text::markdown::html::unescape_html(href.as_bytes()))
        .into_owned()
}

fn is_remote(href: &str) -> bool {
    // Reuse the shared Go net/url parser, including opaque URIs, authority
    // validation, and the distinct query/path/fragment escape rules.
    d2_rs_language::compiler::import_urls::parse(href.as_bytes(), false)
        .is_ok_and(|url| url.scheme.starts_with(b"http"))
}

fn limit_error(name: &str, actual: i64, limit: i64) -> String {
    format!("{name} exceeds maximum of {limit} bytes: {actual}")
}

/// findImageElementsForOptions. Literal matching mirrors imageRegex, including
/// its nonempty href and exact generated-markup prefix.
pub fn find_image_elements(
    svg: &[u8],
    options: Options,
    context: &impl Fn() -> Result<(), String>,
) -> Result<(Vec<ImageMatch>, Vec<String>), String> {
    const PREFIX: &[u8] = b"<image href=\"";
    let mut matches = Vec::new();
    let mut hrefs = Vec::new();
    let mut unique = BTreeSet::new();
    let mut offset = 0;
    while offset < svg.len() {
        context()?;
        let Some(relative) = svg[offset..]
            .windows(PREFIX.len())
            .position(|s| s == PREFIX)
        else {
            break;
        };
        let start = offset + relative;
        let href_start = start + PREFIX.len();
        let Some(length) = svg[href_start..].iter().position(|b| *b == b'"') else {
            break;
        };
        let href_end = href_start + length;
        offset = href_end + 1;
        if length == 0 {
            continue;
        }
        let href = String::from_utf8_lossy(&svg[href_start..href_end]).into_owned();
        let decoded = decode_href(&href);
        if decoded
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("data:"))
        {
            continue;
        }
        let remote = is_remote(&decoded);
        if (remote && !options.remote) || (!remote && !options.local) {
            continue;
        }
        if length > MAX_REFERENCE_BYTES {
            return Err(limit_error(
                "image reference",
                length as i64,
                MAX_REFERENCE_BYTES as i64,
            ));
        }
        if matches.len() == MAX_REFERENCES {
            return Err(format!(
                "image references exceed maximum of {MAX_REFERENCES}"
            ));
        }
        matches.push(ImageMatch {
            start,
            end: offset,
            href_start,
            href_end,
        });
        if unique.insert(href.clone()) {
            hrefs.push(href);
        }
    }
    Ok((matches, hrefs))
}

/// bundleBudget.reserveBundled: failed reservations never consume bytes and
/// the first error remains sticky for all later reservations.
pub struct Budget {
    pub used: i64,
    pub limit: i64,
    pub exhausted: Option<String>,
}
impl Budget {
    pub fn new(limit: i64) -> Self {
        Self {
            used: 0,
            limit,
            exhausted: None,
        }
    }
    pub fn reserve(&mut self, count: i64) -> Result<(), String> {
        if let Some(error) = &self.exhausted {
            return Err(error.clone());
        }
        if count > self.limit - self.used {
            let error = limit_error(
                "cumulative bundled image bytes",
                self.used + count,
                self.limit,
            );
            self.exhausted = Some(error.clone());
            return Err(error);
        }
        self.used += count;
        Ok(())
    }
}

/// resolveReplacement's serialization/budget boundary, after Resource.DataURI.
pub fn replacement(data_uri: &str, budget: &mut Budget) -> Result<Vec<u8>, String> {
    let count = b"<image href=\"".len() + data_uri.len() + 1;
    budget.reserve(count as i64)?;
    Ok(format!("<image href=\"{data_uri}\"").into_bytes())
}

/// applyReplacements: size/context errors return no partially assembled output.
/// The caller retains `svg` when this returns Err.
pub fn apply_replacements(
    svg: &[u8],
    matches: &[ImageMatch],
    replacements: &BTreeMap<String, Vec<u8>>,
    max_output: i64,
    context: &impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    if replacements.is_empty() {
        return Ok(svg.to_vec());
    }
    let mut output_bytes = svg.len() as i64;
    for m in matches {
        context()?;
        if let Some(replacement) =
            replacements.get(String::from_utf8_lossy(&svg[m.href_start..m.href_end]).as_ref())
        {
            output_bytes += replacement.len() as i64 - (m.end - m.start) as i64;
            if output_bytes > max_output {
                return Err(limit_error("bundled SVG output", output_bytes, max_output));
            }
        }
    }
    if output_bytes > max_output {
        return Err(limit_error("bundled SVG output", output_bytes, max_output));
    }
    let mut out = Vec::with_capacity(output_bytes as usize);
    let mut previous = 0;
    for m in matches {
        context()?;
        if let Some(replacement) =
            replacements.get(String::from_utf8_lossy(&svg[m.href_start..m.href_end]).as_ref())
        {
            out.extend_from_slice(&svg[previous..m.start]);
            out.extend_from_slice(replacement);
            previous = m.end;
        }
    }
    out.extend_from_slice(&svg[previous..]);
    Ok(out)
}

/// Resolve a raw generated-SVG href through the caller's imageasset session.
/// Source: imgbundler.resolveReplacement (368–389). DataURIContext canonicalizes
/// MIME from validated bytes; raw HTTP header strings never enter SVG markup.
pub fn resolve_replacement(
    resolver: &d2_rs_scenebuild::RasterImageResolver,
    href: &str,
    budget: &mut Budget,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Vec<u8>, String> {
    let resource = resolver
        .resolve_with_context(&decode_href(href), cancel)
        .map_err(|e| e.message)?;
    let uri = resource.data_uri(cancel).map_err(|e| e.message)?;
    replacement(
        std::str::from_utf8(&uri).expect("data URI is ASCII"),
        budget,
    )
}

/// Caller-owned resolver session, equivalent to BundleWithResolver. The
/// deprecated wrappers create a fresh resolver for each document invocation.
/// Successful replacements survive a later resolution failure; scanner/output
/// admission errors preserve the complete input. The returned error is separate
/// from the returned (possibly partially bundled) document.
pub fn bundle_with_resolver(
    svg: &[u8],
    options: Options,
    resolver: Option<&d2_rs_scenebuild::RasterImageResolver>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> (Vec<u8>, Option<String>) {
    let context = || context_error(cancel);
    let (matches, hrefs) = match find_image_elements(svg, options, &context) {
        Ok(result) => result,
        Err(error) => {
            return (
                svg.to_vec(),
                Some(format!("failed to bundle images: {error}")),
            );
        }
    };
    if hrefs.is_empty() {
        return (svg.to_vec(), None);
    }
    let Some(resolver) = resolver else {
        return (
            svg.to_vec(),
            Some("failed to bundle images: image resolver is required".into()),
        );
    };
    let (output, error) = run_resolver_workers(
        svg,
        &matches,
        &hrefs,
        resolver,
        MAX_BUNDLED_BYTES,
        16,
        cancel,
    );
    (
        output,
        error.map(|e| format!("failed to bundle images: {e}")),
    )
}

fn context_error(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    cancel
        .and_then(d2_rs_core::CancellationToken::error)
        .map_or(Ok(()), |e| Err(e.to_string()))
}

/// runResolverWorkersWithLimit (296–366). Stop scheduling after the first
/// failure and retain that original cause while in-flight work observes
/// cancellation. Applying completed replacements uses the parent's context,
/// rather than the internally canceled worker context.
pub fn run_resolver_workers(
    svg: &[u8],
    matches: &[ImageMatch],
    hrefs: &[String],
    resolver: &d2_rs_scenebuild::RasterImageResolver,
    max_bundled_bytes: i64,
    max_workers: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> (Vec<u8>, Option<String>) {
    use std::sync::Mutex;
    let worker_cancel = cancel.map_or_else(d2_rs_core::CancellationToken::new, |c| c.child());
    let state = Mutex::new((0usize, BTreeMap::new(), None::<String>));
    let budget = Mutex::new(Budget::new(max_bundled_bytes));
    std::thread::scope(|scope| {
        for _ in 0..max_workers.min(hrefs.len()) {
            scope.spawn(|| {
                loop {
                    let href = {
                        let mut state = state.lock().unwrap();
                        if state.2.is_some()
                            || state.0 >= hrefs.len()
                            || worker_cancel.is_cancelled()
                        {
                            break;
                        }
                        let href = &hrefs[state.0];
                        state.0 += 1;
                        href
                    };
                    // The resolver and DataURI support cancellation independently;
                    // reserve only completed replacement bytes under the shared budget.
                    let result = (|| {
                        let resource = resolver
                            .resolve_with_context(&decode_href(href), Some(&worker_cancel))
                            .map_err(|e| e.message)?;
                        let uri = resource
                            .data_uri(Some(&worker_cancel))
                            .map_err(|e| e.message)?;
                        replacement(
                            std::str::from_utf8(&uri).expect("data URI is ASCII"),
                            &mut budget.lock().unwrap(),
                        )
                    })();
                    let mut state = state.lock().unwrap();
                    match result {
                        Ok(replacement) => {
                            state.1.insert(href.clone(), replacement);
                        }
                        Err(error) => {
                            if state.2.is_none() {
                                state.2 = Some(error);
                            }
                            worker_cancel.cancel();
                            break;
                        }
                    }
                }
            });
        }
    });
    let (_, replacements, cause) = state.into_inner().unwrap();
    let cause = cause.or_else(|| context_error(cancel).err());
    match apply_replacements(svg, matches, &replacements, MAX_OUTPUT_BYTES, &|| {
        context_error(cancel)
    }) {
        Ok(output) => (output, cause),
        Err(error) => (
            svg.to_vec(),
            Some(cause.map_or_else(|| error.clone(), |cause| format!("{error}\n{cause}"))),
        ),
    }
}

/// BundleLocal / BundleLocalWithPolicy (imgbundler.go:84–118). Each deprecated
/// wrapper invocation owns a new resolver and, when requested, a new cache.
/// Passing ImageLocalFiles::default() denies all local access.
pub fn bundle_local_with_policy(
    input_path: &std::path::Path,
    svg: &[u8],
    files: d2_rs_scenebuild::ImageLocalFiles,
    cache_images: bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> (Vec<u8>, Option<String>) {
    use d2_rs_scenebuild::{ImageMemoryCache, RasterImageResolver};
    let prefix = |error| format!("failed to bundle local images: {error}");
    let base = if input_path.as_os_str().is_empty() || input_path == std::path::Path::new("-") {
        files.root_path().unwrap_or(std::path::Path::new("."))
    } else {
        input_path.parent().unwrap_or(std::path::Path::new("."))
    };
    let limits = legacy_image_limits();
    let resolver = (|| {
        let resolver = RasterImageResolver::new(base, limits, std::sync::Arc::new(files.clone()))?;
        if cache_images {
            resolver.with_shared_cache(
                std::sync::Arc::new(ImageMemoryCache::new(
                    MAX_REFERENCES as i64,
                    MAX_BUNDLED_BYTES,
                )?),
                "imgbundler/imageasset/v2/default-http",
                false,
            )
        } else {
            Ok(resolver)
        }
    })();
    let resolver = match resolver {
        Ok(resolver) => resolver,
        Err(e) => return (svg.to_vec(), Some(prefix(e.message))),
    };
    let (output, error) = bundle_with_resolver(
        svg,
        Options {
            local: true,
            remote: false,
        },
        Some(&resolver),
        cancel,
    );
    (output, error.map(prefix))
}

/// BundleRemote / BundleRemoteWithPolicy (imgbundler.go:121–148). A caller's
/// opt-in is scoped to this invocation, never a process-wide HTTP policy.
pub fn bundle_remote_with_policy(
    svg: &[u8],
    cache_images: bool,
    allow_private_networks: bool,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> (Vec<u8>, Option<String>) {
    use d2_rs_scenebuild::{
        ImageLocalFiles, ImageMemoryCache, NativeImageHttpClient, RasterImageResolver,
    };
    let prefix = |error| format!("failed to bundle remote images: {error}");
    let resolver = (|| {
        let resolver = RasterImageResolver::new(
            std::path::Path::new("."),
            legacy_image_limits(),
            std::sync::Arc::new(ImageLocalFiles::default()),
        )?
        .with_http_client(std::sync::Arc::new(NativeImageHttpClient {
            allow_private_networks,
        }));
        if cache_images {
            resolver.with_shared_cache(
                std::sync::Arc::new(ImageMemoryCache::new(
                    MAX_REFERENCES as i64,
                    MAX_BUNDLED_BYTES,
                )?),
                "imgbundler/imageasset/v2/default-http",
                allow_private_networks,
            )
        } else {
            Ok(resolver)
        }
    })();
    let resolver = match resolver {
        Ok(resolver) => resolver,
        Err(e) => return (svg.to_vec(), Some(prefix(e.message))),
    };
    let (output, error) = bundle_with_resolver(
        svg,
        Options {
            local: false,
            remote: true,
        },
        Some(&resolver),
        cancel,
    );
    (output, error.map(prefix))
}

fn legacy_image_limits() -> d2_rs_scenebuild::ImageAssetLimits {
    d2_rs_scenebuild::ImageAssetLimits {
        max_fetched_bytes: 1 << 25,
        max_encoded_bytes: 1 << 25,
        max_decompressed_bytes: 1 << 25,
        max_svg_bytes: 1 << 25,
        max_decoded_width: 32768,
        max_decoded_height: 32768,
        max_decoded_pixels: 64 << 20,
        max_assets: MAX_REFERENCES as i64,
        max_cumulative_encoded_bytes: MAX_BUNDLED_BYTES,
        max_cumulative_decoded_bytes: MAX_BUNDLED_BYTES,
    }
}
