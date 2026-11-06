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
