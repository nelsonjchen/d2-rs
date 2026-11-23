// SPDX-License-Identifier: MPL-2.0
//! CLI adapter for the bounded imgbundler algorithms and existing native
//! imageasset resolver/workers. This is one document-scoped budget.
use super::*;
use d2_rs_cli::imgbundler::{self, Budget, Options};

pub(super) fn bundle(
    source_args: &[String],
    include_remote: bool,
    svg: &[u8],
) -> (Vec<u8>, Vec<(String, String)>) {
    let context = || match watch::active_cancellation_token().and_then(|token| token.error()) {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    };
    let (matches, hrefs) = match imgbundler::find_image_elements(
        svg,
        Options {
            local: true,
            remote: include_remote,
        },
        &context,
    ) {
        Ok(result) => result,
        Err(error) => return (svg.to_vec(), vec![(String::new(), error)]),
    };
    let base = raster_asset::resolver_base(source_args);
    let budget = Mutex::new(Budget::new(imgbundler::MAX_BUNDLED_BYTES));
    let load = |raw: &str| {
        let href = imgbundler::decode_href(raw);
        match raster_asset::resolve(&href, &base)? {
            raster_asset::Outcome::Data(resolved) => {
                let uri = format!(
                    "data:{};base64,{}",
                    resolved.mime,
                    base64::engine::general_purpose::STANDARD.encode(&resolved.bytes)
                );
                imgbundler::replacement(&uri, &mut budget.lock().unwrap())?;
                Ok(Some(AssetData {
                    bytes: resolved.bytes,
                    content_type: Some(resolved.mime.to_owned()),
                }))
            }
            raster_asset::Outcome::Unavailable(error) => Err(error),
        }
    };
    let mut loaded = collect_asset_results_concurrent(hrefs, load);
    let replacements = loaded
        .assets
        .iter()
        .map(|(href, bytes)| {
            let uri = format!(
                "data:{};base64,{}",
                loaded.content_types[href],
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            (href.clone(), format!("<image href=\"{uri}\"").into_bytes())
        })
        .collect();
    match imgbundler::apply_replacements(
        svg,
        &matches,
        &replacements,
        imgbundler::MAX_OUTPUT_BYTES,
        &context,
    ) {
        Ok(output) => (output, loaded.failures),
        Err(error) => {
            // errors.Join(replacementErr, cause), preserving completed source
            // bytes when output-size/context admission fails.
            if let Some((_, cause)) = loaded.failures.first() {
                loaded.failures = vec![(String::new(), format!("{error}\n{cause}"))];
            } else {
                loaded.failures.push((String::new(), error));
            }
            (svg.to_vec(), loaded.failures)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scanner_failure_precedes_acquisition_and_keeps_original_svg() {
        let source = format!("<svg>{}</svg>", "<image href=\"absent.svg\"/>".repeat(4097));
        let (out, errors) = bundle(&[], true, source.as_bytes());
        assert_eq!(out, source.as_bytes());
        assert_eq!(
            errors,
            vec![(
                String::new(),
                "image references exceed maximum of 4096".into()
            )]
        );
    }
    #[test]
    fn document_budget_adapter_preserves_duplicate_local_references() {
        let root = std::env::temp_dir().join(format!("d2-imgbundler-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let data = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        std::fs::write(root.join("a&b.svg"), data).unwrap();
        let source = b"<svg><image href=\"a&amp;b.svg\"/><image href=\"a&#38;b.svg\"/></svg>";
        let (out, errors) = bundle(
            &[format!("--path={}", root.join("input.d2").display())],
            false,
            source,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let uri = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(data)
        );
        assert_eq!(
            out,
            format!("<svg><image href=\"{uri}\"/><image href=\"{uri}\"/></svg>").as_bytes()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// raster.go bundleRasterPreview: embed the native scene's immutable snapshot.
/// Missing resources use the same SVG as the PNG, without retrying acquisition.
pub(super) fn raster_preview(
    svg: &[u8],
    options: &d2_rs_render::RenderOptions,
) -> Result<Vec<u8>, String> {
    const MAX_SOURCE: usize = 32 << 20;
    const MAX_OUTPUT: usize = 192 << 20;
    const PREFIX: &[u8] = b"<image href=\"";
    let context = || match watch::active_cancellation_token().and_then(|token| token.error()) {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    };
    context()?;
    if svg.len() > MAX_SOURCE {
        return Err(format!(
            "source bytes {} exceed limit {MAX_SOURCE}",
            svg.len()
        ));
    }
    let mut references = Vec::new();
    let mut resources = BTreeMap::<String, (&str, &[u8], usize)>::new();
    let mut offset = 0;
    let mut count = 0;
    let mut output_bytes = svg.len();
    while let Some(relative) = svg[offset..]
        .windows(PREFIX.len())
        .position(|s| s == PREFIX)
    {
        context()?;
        let start = offset + relative + PREFIX.len();
        let Some(length) = svg[start..].iter().position(|b| *b == b'"') else {
            break;
        };
        let end = start + length;
        offset = end + 1;
        count += 1;
        if count > 4096 {
            return Err(format!("image references {count} exceed limit 4096"));
        }
        let raw = String::from_utf8_lossy(&svg[start..end]);
        if raw
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("data:"))
        {
            continue;
        }
        let source = imgbundler::decode_href(&raw);
        if !resources.contains_key(&source) {
            let (mime, bytes) = if options.unavailable_images.contains(&source) {
                (
                    "image/svg+xml",
                    d2_rs_scenebuild::UNAVAILABLE_IMAGE_SVG.as_bytes(),
                )
            } else {
                let bytes = options.asset_data.get(&source).ok_or_else(|| {
                    format!("preview image {source:?} was not resolved by the native scene")
                })?;
                (raster_asset::resource_mime(bytes), bytes.as_slice())
            };
            let uri_bytes = 5 + mime.len() + 8 + bytes.len().div_ceil(3) * 4;
            if uri_bytes > MAX_OUTPUT {
                return Err(format!("expanded output bytes exceed limit {MAX_OUTPUT}"));
            }
            resources.insert(source.clone(), (mime, bytes, uri_bytes));
        }
        output_bytes = output_bytes
            .checked_sub(length)
            .and_then(|n| n.checked_add(resources[&source].2))
            .filter(|n| *n <= MAX_OUTPUT)
            .ok_or_else(|| format!("expanded output bytes exceed limit {MAX_OUTPUT}"))?;
        references.push((start, end, source));
    }
    let mut encoded = BTreeMap::new();
    for (source, (mime, bytes, uri_bytes)) in resources {
        let mut uri = String::with_capacity(uri_bytes);
        uri.push_str(&format!("data:{mime};base64,"));
        for chunk in bytes.chunks(48 << 10) {
            context()?;
            base64::engine::general_purpose::STANDARD.encode_string(chunk, &mut uri);
        }
        encoded.insert(source, uri);
    }
    let mut result = Vec::with_capacity(output_bytes);
    let mut copied = 0;
    for (start, end, source) in references {
        context()?;
        result.extend_from_slice(&svg[copied..start]);
        result.extend_from_slice(encoded[&source].as_bytes());
        copied = end;
    }
    result.extend_from_slice(&svg[copied..]);
    context()?;
    Ok(result)
}

#[cfg(test)]
mod raster_preview_tests {
    use super::*;
    #[test]
    fn snapshot_and_placeholder_unescape_without_reopening_sources() {
        let mut options = d2_rs_render::RenderOptions::default();
        let original = b"<svg width=\"1\" height=\"1\"/>".to_vec();
        options
            .asset_data
            .insert("asset&one.svg".into(), original.clone());
        options
            .unavailable_images
            .insert("missing&image.png".into());
        let source =
            br#"<svg><image href="asset&amp;one.svg"/><image href="missing&amp;image.png"/></svg>"#;
        let output = String::from_utf8(raster_preview(source, &options).unwrap()).unwrap();
        for bytes in [
            original.as_slice(),
            d2_rs_scenebuild::UNAVAILABLE_IMAGE_SVG.as_bytes(),
        ] {
            assert!(output.contains(&format!(
                "data:image/svg+xml;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )));
        }
        assert!(!output.contains("&amp;"));
    }
    #[test]
    fn counts_embedded_references_and_bounds_expansion() {
        let mut options = d2_rs_render::RenderOptions::default();
        let too_many = r#"<image href="data:image/png;base64,AA=="/>"#.repeat(4097);
        assert!(
            raster_preview(too_many.as_bytes(), &options)
                .unwrap_err()
                .contains("image references")
        );
        options
            .asset_data
            .insert("asset.svg".into(), vec![b' '; 65536]);
        let expanding = r#"<image href="asset.svg"/>"#.repeat(3000);
        assert!(
            raster_preview(expanding.as_bytes(), &options)
                .unwrap_err()
                .contains("expanded output bytes")
        );
    }
}
