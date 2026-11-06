// SPDX-License-Identifier: MPL-2.0
use d2_rs_cli::imgbundler::{self, Budget, Options};
use std::collections::BTreeMap;

#[test]
fn reference_limits_count_occurrences_after_pass_selection() {
    let source = format!("<svg>{}</svg>", "<image href=\"same.svg\"/>".repeat(4097));
    assert_eq!(
        imgbundler::find_image_elements(
            source.as_bytes(),
            Options {
                local: true,
                remote: false
            },
            &|| Ok(())
        )
        .unwrap_err(),
        "image references exceed maximum of 4096"
    );
    let huge = "a".repeat(65537);
    for href in [
        format!("data:image/png;base64,{huge}"),
        format!("https://example.com/{huge}"),
    ] {
        let source = format!("<image href=\"{href}\"/>");
        assert!(
            imgbundler::find_image_elements(
                source.as_bytes(),
                Options {
                    local: true,
                    remote: false
                },
                &|| Ok(())
            )
            .unwrap()
            .0
            .is_empty()
        );
    }
}

#[test]
fn raw_entity_spellings_remain_distinct_replacement_keys() {
    let source = b"<image href=\"a&amp;b\"/><image href=\"a&#38;b\"/><image href=\"a&amp;b\"/>";
    let (matches, hrefs) = imgbundler::find_image_elements(
        source,
        Options {
            local: true,
            remote: true,
        },
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(hrefs, vec!["a&amp;b", "a&#38;b"]);
    assert_eq!(imgbundler::decode_href(&hrefs[0]), "a&b");
    let replacements = BTreeMap::from([("a&amp;b".into(), b"<image href=\"ok\"".to_vec())]);
    assert_eq!(
        imgbundler::apply_replacements(source, &matches, &replacements, 1024, &|| Ok(())).unwrap(),
        b"<image href=\"ok\"/><image href=\"a&#38;b\"/><image href=\"ok\"/>"
    );
}

#[test]
fn canceled_replacements_and_sticky_reservations_preserve_state() {
    let source = b"<image href=\"same\"/>";
    let (matches, _) = imgbundler::find_image_elements(
        source,
        Options {
            local: true,
            remote: false,
        },
        &|| Ok(()),
    )
    .unwrap();
    let replacements = BTreeMap::from([("same".into(), b"<image href=\"data:large\"".to_vec())]);
    assert_eq!(
        imgbundler::apply_replacements(source, &matches, &replacements, 1024, &|| Err(
            "context canceled".into()
        ))
        .unwrap_err(),
        "context canceled"
    );
    // Go returns early when there is nothing to replace, even on cancellation.
    assert_eq!(
        imgbundler::apply_replacements(source, &matches, &BTreeMap::new(), 0, &|| Err(
            "context canceled".into()
        ))
        .unwrap(),
        source
    );
    let mut budget = Budget::new(6);
    budget.reserve(4).unwrap();
    let first = budget.reserve(3).unwrap_err();
    assert_eq!(budget.reserve(2).unwrap_err(), first);
    assert_eq!(budget.used, 4);
}

fn resolver() -> d2_rs_scenebuild::RasterImageResolver {
    use d2_rs_scenebuild::{ImageAssetLimits, RasterImageResolver, UnrestrictedImageFiles};
    RasterImageResolver::new(
        std::path::Path::new("."),
        ImageAssetLimits {
            max_fetched_bytes: 1 << 20,
            max_encoded_bytes: 1 << 20,
            max_decompressed_bytes: 1 << 20,
            max_svg_bytes: 1 << 20,
            max_decoded_width: 1000,
            max_decoded_height: 1000,
            max_decoded_pixels: 1_000_000,
            max_assets: 4096,
            max_cumulative_encoded_bytes: 1 << 20,
            max_cumulative_decoded_bytes: 1 << 20,
        },
        std::sync::Arc::new(UnrestrictedImageFiles::new()),
    )
    .unwrap()
}

#[test]
fn resolver_admission_precedes_resolution_and_allows_empty_pass() {
    let source = b"<svg><image href=\"local.svg\"/></svg>";
    let (out, error) = imgbundler::bundle_with_resolver(source, Options::default(), None, None);
    assert_eq!(out, source);
    assert!(error.is_none());
    let options = Options {
        local: true,
        remote: true,
    };
    let (out, error) = imgbundler::bundle_with_resolver(source, options, None, None);
    assert_eq!(out, source);
    assert_eq!(
        error.as_deref(),
        Some("failed to bundle images: image resolver is required")
    );
    let token = d2_rs_core::CancellationToken::new();
    token.cancel();
    let (out, error) = imgbundler::bundle_with_resolver(source, options, None, Some(&token));
    assert_eq!(out, source);
    assert_eq!(
        error.as_deref(),
        Some("failed to bundle images: context canceled")
    );
}

#[test]
fn workers_keep_successes_and_the_first_failure_without_scheduling_later_jobs() {
    let canonical =
        "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciLz4=";
    let good = "data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22/%3E";
    let bad = "data:image/png;base64,bm90IGEgcG5n";
    let later = "data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%3E%3Ctitle%3Elater%3C/title%3E%3C/svg%3E";
    let source = format!(
        "<svg><image href=\"{good}\"/><image href=\"{bad}\"/><image href=\"{later}\"/></svg>"
    );
    // The direct worker API also accepts caller-supplied source matches.
    let hrefs = vec![good.to_string(), bad.to_string(), later.to_string()];
    let matches = hrefs
        .iter()
        .map(|href| {
            let start = source.find(&format!("<image href=\"{href}\"")).unwrap();
            imgbundler::ImageMatch {
                start,
                end: start + 14 + href.len(),
                href_start: start + 13,
                href_end: start + 13 + href.len(),
            }
        })
        .collect::<Vec<_>>();
    let resolver = resolver();
    let (out, error) = imgbundler::run_resolver_workers(
        source.as_bytes(),
        &matches,
        &hrefs,
        &resolver,
        1 << 20,
        1,
        None,
    );
    assert_eq!(out, source.replacen(good, canonical, 1).as_bytes());
    let error = error.unwrap();
    assert!(error.contains("unsupported or malformed"), "{error}");
    assert!(!error.contains("context canceled"));
    assert_eq!(resolver.usage().assets, 1);
}

#[cfg(unix)]
#[test]
fn local_policy_retains_root_handles_and_shares_close() {
    use d2_rs_scenebuild::{ImageFileReader, ImageLocalFiles};
    use std::os::unix::fs::symlink;
    let parent = std::env::temp_dir().join(format!("d2-img-policy-{}", std::process::id()));
    std::fs::create_dir(&parent).unwrap();
    let root = parent.join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("value"), b"original").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    symlink("../value", root.join("sub/inside")).unwrap();
    symlink("../../outside", root.join("sub/escape")).unwrap();
    std::fs::write(parent.join("outside"), b"secret").unwrap();
    let policy = ImageLocalFiles::rooted(&root).unwrap();
    let copy = policy.clone();
    assert_eq!(
        policy.read(&root.join("sub/inside"), 8, None).unwrap(),
        b"original"
    );
    assert!(policy.read(&root.join("sub/escape"), 100, None).is_err());
    assert!(policy.cache_key(&parent.join("outside")).is_err());
    assert!(
        policy
            .cache_key(std::path::Path::new("../outside"))
            .is_err()
    );
    assert!(
        policy
            .read(std::path::Path::new("../../outside"), 100, None)
            .is_err()
    );
    assert!(policy.cache_key(&root).unwrap().unwrap().ends_with(":."));
    let second = ImageLocalFiles::rooted(&root).unwrap();
    assert_ne!(
        policy.cache_key(&root.join("value")).unwrap(),
        second.cache_key(&root.join("value")).unwrap()
    );
    assert!(
        ImageLocalFiles::default()
            .cache_key(&root.join("value"))
            .is_err()
    );
    std::fs::rename(&root, parent.join("moved")).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("value"), b"replacement").unwrap();
    assert_eq!(
        policy.read(&root.join("value"), 8, None).unwrap(),
        b"original"
    );
    copy.close();
    policy.close();
    assert!(policy.read(&root.join("value"), 8, None).is_err());
    second.close();
    std::fs::remove_dir_all(parent).unwrap();
}

#[cfg(unix)]
#[test]
fn local_policy_rejects_fifo_without_blocking_or_reading_past_limit() {
    use d2_rs_scenebuild::{ImageFileReader, ImageLocalFiles};
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let root = std::env::temp_dir().join(format!("d2-img-fifo-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let fifo = root.join("fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let rooted = ImageLocalFiles::rooted(&root).unwrap();
    let started = std::time::Instant::now();
    for policy in [&rooted, &ImageLocalFiles::Unrestricted] {
        assert!(
            policy
                .read(&fifo, 100, None)
                .unwrap_err()
                .message
                .contains("not a regular file")
        );
        assert!(policy.read(&root, 100, None).is_err());
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    std::fs::write(root.join("large"), b"oversized").unwrap();
    assert_eq!(
        rooted
            .read(&root.join("large"), 8, None)
            .unwrap_err()
            .message,
        "fetched bytes 9 exceeds limit 8"
    );
    let cancel = d2_rs_core::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        rooted
            .read(&root.join("large"), 100, Some(&cancel))
            .unwrap_err()
            .message,
        "context canceled"
    );
    rooted.close();
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn standalone_policy_preserves_denial_identity_and_open_file_ownership() {
    use d2_rs_scenebuild::ImageLocalFiles;
    use std::io::Read;
    use std::path::Path;
    let root = std::env::temp_dir().join(format!("d2-open-policy-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("value"), b"original").unwrap();
    let denied = ImageLocalFiles::default();
    assert!(denied.open(&root.join("value")).unwrap_err().denied);
    assert!(
        denied
            .policy_cache_key(&root.join("value"))
            .unwrap_err()
            .denied
    );
    let policy = ImageLocalFiles::rooted(&root).unwrap();
    assert!(policy.open(Path::new("../outside")).unwrap_err().denied);
    std::os::unix::fs::symlink("../outside", root.join("escape")).unwrap();
    assert!(!policy.open(Path::new("escape")).unwrap_err().denied);
    let mut file = policy.open(Path::new("value")).unwrap();
    let copy = policy.clone();
    std::thread::scope(|scope| {
        for _ in 0..32 {
            let copy = copy.clone();
            scope.spawn(move || copy.try_close().unwrap());
        }
    });
    policy.try_close().unwrap();
    assert!(!copy.open(Path::new("value")).unwrap_err().denied);
    // Closing a root revokes future opens, not descriptors already owned by
    // callers. This is the os.Root/fs.File ownership boundary.
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "original");
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}
