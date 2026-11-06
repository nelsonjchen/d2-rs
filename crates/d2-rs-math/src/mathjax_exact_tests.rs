// SPDX-License-Identifier: MPL-2.0
//! Byte-exact MathJax contract of `render_exact` against the pinned
//! mathjax-go `mathjax.Render` (oracle tier: recipe mathtext-latex-mathjax-exact).
use super::*;

#[cfg(feature = "oracle-fixtures")]
#[test]
fn render_exact_matches_or_refuses_pinned_mathjax() {
    use sha2::{Digest, Sha256};
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/mathjax-exact-v090.json"
    ))
    .unwrap();
    let mut failures = Vec::new();
    let mut counts = [0usize; 3];
    for case in fixture["cases"].as_array().unwrap() {
        let tex = case["tex"].as_str().unwrap();
        let expected = case["sha256"].as_str().unwrap();
        let rendered = render_exact(tex);
        let digest = rendered
            .as_ref()
            .ok()
            .map(|svg| format!("{:x}", Sha256::digest(svg.as_bytes())));
        let ok = match case["outcome"].as_str().unwrap() {
            "match" => {
                counts[0] += 1;
                digest.as_deref() == Some(expected)
            }
            // An untranslated construct must stay an explicit error.
            "refused" => {
                counts[1] += 1;
                rendered == Err(MathError::Unsupported)
            }
            // Recorded defects: still differ (promote the entry when fixed).
            "mismatch" => {
                counts[2] += 1;
                digest.is_some() && digest.as_deref() != Some(expected)
            }
            other => panic!("unknown outcome {other}"),
        };
        if !ok {
            failures.push(format!("{tex:?} ({})", case["outcome"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures: {:?}",
        failures.len(),
        failures
    );
    let recorded = &fixture["counts"];
    assert_eq!(counts[0], recorded["match"].as_u64().unwrap() as usize);
    assert_eq!(counts[1], recorded["refused"].as_u64().unwrap() as usize);
    assert_eq!(counts[2], recorded["mismatch"].as_u64().unwrap() as usize);
}

#[test]
fn render_exact_never_returns_the_visible_text_fallback() {
    // `render` keeps its visible fallback for unsupported TeX; the exact
    // entry point used by d2svg refuses instead.
    let tex = r"\begin{dcases*}x&y\end{dcases*}";
    assert!(render(tex).unwrap().contains("<text x="));
    assert_eq!(render_exact(tex), Err(MathError::Unsupported));
}
