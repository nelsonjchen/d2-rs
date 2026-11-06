// SPDX-License-Identifier: MPL-2.0
//! Immutable, caller-supplied HTTP responses for whole-board asset comparisons.
use d2_rs_scenebuild::{
    AssetOptions, ImageAssetLimits, ImageHttpClient, ImageHttpResponse, ImageResolveError,
    RasterImageResolver, UnrestrictedImageFiles,
};
use std::sync::Arc;
#[derive(Debug)]
struct Transport(serde_json::Value);
impl ImageHttpClient for Transport {
    fn get(
        &self,
        url: &str,
        maximum: i64,
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<ImageHttpResponse, ImageResolveError> {
        let error = |message: String| ImageResolveError {
            unavailable: false,
            message,
        };
        if let Some(cause) = cancel.and_then(|c| c.error()) {
            return Err(error(cause.to_string()));
        }
        let v = self
            .0
            .get(url)
            .ok_or_else(|| error(format!("unrecorded isometric asset source {url:?}")))?;
        let path = v["path"]
            .as_str()
            .ok_or_else(|| error("asset path missing".into()))?;
        let length = std::fs::metadata(path)
            .map_err(|e| error(e.to_string()))?
            .len();
        if length > maximum as u64 || length > 8 << 20 {
            return Err(error("isometric probe asset exceeds byte limit".into()));
        }
        let body = std::fs::read(path).map_err(|e| error(e.to_string()))?;
        Ok(ImageHttpResponse {
            status: 200,
            content_length: body.len() as i64,
            headers: vec![(
                "Content-Type".into(),
                v["mime"]
                    .as_str()
                    .ok_or_else(|| error("asset MIME missing".into()))?
                    .into(),
            )],
            body,
        })
    }
}
pub fn options(request: &serde_json::Value) -> Result<Option<AssetOptions>, String> {
    let Some(sources) = request.get("assetSources") else {
        return Ok(None);
    };
    let resolver = RasterImageResolver::new(
        std::path::Path::new("."),
        ImageAssetLimits {
            max_fetched_bytes: 12 << 20,
            max_encoded_bytes: 8 << 20,
            max_decompressed_bytes: 8 << 20,
            max_svg_bytes: 2 << 20,
            max_decoded_width: 8192,
            max_decoded_height: 8192,
            max_decoded_pixels: 8 << 20,
            max_assets: 7000,
            max_cumulative_encoded_bytes: 16 << 20,
            max_cumulative_decoded_bytes: 64 << 20,
        },
        Arc::new(UnrestrictedImageFiles::new()),
    )
    .map_err(|e| e.to_string())?
    .with_http_client(Arc::new(Transport(sources.clone())));
    Ok(Some(AssetOptions {
        resolver_impl: Some(Arc::new(resolver)),
        ..Default::default()
    }))
}
