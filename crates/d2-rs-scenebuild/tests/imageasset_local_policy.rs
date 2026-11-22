// SPDX-License-Identifier: MPL-2.0
// ImageLocalFiles::rooted is Unix-only, like the descriptor-relative opens it needs.
#![cfg(unix)]
use d2_rs_scenebuild::{ImageAssetLimits, ImageLocalFiles, RasterImageResolver};
use std::path::Path;
use std::sync::Arc;

#[test]
fn omitted_base_uses_policy_root_and_explicit_base_takes_precedence() {
    let root = std::env::temp_dir().join(format!(
        "d2-imageasset-root-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let policy = Arc::new(ImageLocalFiles::rooted(&root).unwrap());
    // A missing file's diagnostic identifies which directory was resolved,
    // without depending on decoding or any generated image fixture.
    let implicit = RasterImageResolver::new(Path::new(""), limits(), policy.clone()).unwrap();
    let error = implicit
        .resolve_with_context("missing.png", None)
        .unwrap_err();
    assert!(error.unavailable);
    assert!(
        error
            .message
            .contains(&root.join("missing.png").to_string_lossy().to_string())
    );
    assert!(error.message.contains("open local file"));
    let explicit = RasterImageResolver::new(Path::new("."), limits(), policy.clone()).unwrap();
    let error = explicit
        .resolve_with_context("missing.png", None)
        .unwrap_err();
    assert!(!error.unavailable);
    assert!(error.message.contains("outside configured root"));
    policy.close();
    std::fs::remove_dir(root).unwrap();
}

fn limits() -> ImageAssetLimits {
    ImageAssetLimits {
        max_fetched_bytes: 1 << 20,
        max_encoded_bytes: 1 << 20,
        max_decompressed_bytes: 1 << 20,
        max_svg_bytes: 1 << 20,
        max_decoded_width: 1000,
        max_decoded_height: 1000,
        max_decoded_pixels: 1_000_000,
        max_assets: 10,
        max_cumulative_encoded_bytes: 1 << 24,
        max_cumulative_decoded_bytes: 1 << 24,
    }
}

#[test]
fn data_only_preserves_local_policy_errors_before_external_load() {
    // Upstream b43d0c44f lib/imageasset TestDataOnlyResolver.
    let resolver = RasterImageResolver::new_data_only(limits()).unwrap();
    let local = resolver.resolve_with_context("icon.png", None).unwrap_err();
    assert!(!local.unavailable);
    assert!(
        local.message.contains("localfile: access denied"),
        "{local}"
    );
    let remote = resolver
        .resolve_with_context("https://example.invalid/icon.png", None)
        .unwrap_err();
    assert!(
        remote
            .message
            .contains("data-only resolver requires a data URI"),
        "{remote}"
    );
    assert!(
        resolver
            .resolve_with_context("//example.invalid/icon.png", None)
            .unwrap_err()
            .message
            .contains("network-path references")
    );
    assert_eq!(resolver.usage().assets, 0);
    let embedded = resolver
        .resolve_with_context(
            "data:image/svg+xml,%3Csvg%20width=%221%22%20height=%221%22/%3E",
            None,
        )
        .unwrap();
    assert_eq!(embedded.resource.mime_type, "image/svg+xml");
    assert_eq!(resolver.usage().assets, 1);
}
