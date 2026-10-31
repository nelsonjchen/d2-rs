// SPDX-License-Identifier: MPL-2.0
use d2_rs_lib::config::*;
use serde_json::json;

#[cfg(feature = "oracle-fixtures")]
#[test]
fn original_config_and_default_calls() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/library-config-api.json")).unwrap();
    assert_eq!(cases.len(), 10);
    for case in cases {
        let req = &case["request"];
        let mut compile: LibraryCompileOptions =
            serde_json::from_value(req["options"].clone()).unwrap();
        let mut render: LibraryRenderOptions =
            serde_json::from_value(req["options"]["Render"].clone()).unwrap();
        if req["op"] == "configs" {
            let config: Option<LibraryConfig> =
                serde_json::from_value(req["config"].clone()).unwrap();
            apply_configs(config.as_ref(), &mut compile, &mut render);
        } else {
            apply_defaults(&mut compile, &mut render);
        }
        let mut got = serde_json::to_value(compile).unwrap();
        got["Render"] = serde_json::to_value(render).unwrap();
        assert_eq!(got, case["got"], "{}", case["name"]);
    }
}
#[test]
fn explicit_empty_palette_false_and_zero_win_and_own_their_values() {
    let mut source = LibraryConfig {
        isometric: Some(true),
        theme_id: Some(300),
        pad: Some(42),
        theme_overrides: Some([("b1".into(), Some("red".into()))].into()),
        ..Default::default()
    };
    let mut c = LibraryCompileOptions::default();
    let mut r = LibraryRenderOptions {
        isometric: Some(false),
        theme_id: Some(0),
        pad: Some(0),
        theme_overrides: Some(Default::default()),
        ..Default::default()
    };
    let resolved = resolve_config(Some(&source), &mut c, &mut r).unwrap();
    assert_eq!(r.theme_overrides, Some(Default::default()));
    assert_eq!(r.pad, Some(0));
    assert_eq!(r.theme_id, Some(0));
    assert_eq!(resolved.isometric, None);
    source
        .theme_overrides
        .as_mut()
        .unwrap()
        .insert("b1".into(), Some("blue".into()));
    assert_eq!(
        serde_json::to_value(resolved).unwrap()["themeOverrides"]["b1"],
        json!("red")
    );
}
#[test]
fn sketch_controls_only_its_own_font_default_and_conflict() {
    let mut c = LibraryCompileOptions::default();
    let mut r = LibraryRenderOptions {
        isometric: Some(true),
        ..Default::default()
    };
    resolve_config(None, &mut c, &mut r).unwrap();
    assert_eq!(c.font_family, None);
    r.sketch = Some(true);
    assert_eq!(
        resolve_config(None, &mut c, &mut r).unwrap_err(),
        "sketch cannot be combined with isometric rendering"
    );
    assert_eq!(c.font_family.as_deref(), Some("HandDrawn"));
    c.font_family = Some("custom".into());
    r.isometric = Some(false);
    resolve_config(None, &mut c, &mut r).unwrap();
    assert_eq!(c.font_family.as_deref(), Some("custom"));
}
