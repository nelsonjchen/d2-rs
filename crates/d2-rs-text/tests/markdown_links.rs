// SPDX-License-Identifier: MPL-2.0
#[cfg(feature = "oracle-fixtures")]
#[test]
fn d2_render_markdown_sanitizes_links_after_raw_goldmark_rendering() {
    fn decode(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    let fixture: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-link-sanitization-source-v090.json"))
    .unwrap();
    let requests = fixture["requests"].as_array().unwrap();
    let source = fixture["source"].as_array().unwrap();
    assert_eq!(requests.len(), 83);
    assert_eq!(source.len(), requests.len());
    for (request, source) in requests.iter().zip(source) {
        let html = d2_rs_text::markdown::html::render_markdown_html_bytes(&decode(
            request["hex"].as_str().unwrap(),
        ));
        assert!(source["error"].is_null());
        assert_eq!(
            html,
            decode(source["html_hex"].as_str().unwrap()),
            "{request}"
        );
        assert_eq!(
            d2_rs_text::xml::validate_markdown_html_bytes(&html)
                .err()
                .as_deref(),
            source["xml_error"].as_str(),
            "{request}"
        );
    }
}
