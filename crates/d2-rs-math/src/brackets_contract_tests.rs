// SPDX-License-Identifier: MPL-2.0
use super::*;
#[test]
fn mathtools_brackets_match_original_go_state_options_and_border_geometry() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/brackets-source-v090.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 278);
    for row in rows {
        let expression = row["expression"].as_str().unwrap();
        match (measure(expression), row["measure_error"].as_str()) {
            (Ok(size), None) => {
                assert_eq!(size.width, row["width"].as_f64().unwrap(), "{expression}");
                assert_eq!(size.height, row["height"].as_f64().unwrap(), "{expression}");
            }
            (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected, "{expression}"),
            (actual, expected) => panic!("{expression}: {actual:?} vs {expected:?}"),
        }
        match (render(expression), row["render_error"].as_str()) {
            (Ok(svg), None) => {
                let source = row["svg"].as_str().unwrap();
                assert_eq!(
                    enclose_contract_tests::inventory(&svg),
                    enclose_contract_tests::inventory(source),
                    "{expression}"
                );
                let generated = scene::import_scene_svg(&svg).err();
                let original_generated = scene::import_scene_svg(source).err();
                assert_eq!(
                    generated, original_generated,
                    "{expression}: generated math raster admission"
                );
                let typed = svg_import::import_labeled(
                    "bracket",
                    source.as_bytes(),
                    [10_000_000; 9],
                    None,
                    None,
                )
                .err();
                assert_eq!(
                    generated,
                    typed.map(|e| e
                        .strip_prefix("d2svgimport: bracket: ")
                        .unwrap_or(&e)
                        .to_owned()),
                    "{expression}: generated/typed scene agreement"
                );
                let styles = |raw: &str| {
                    let d = roxmltree::Document::parse(raw).unwrap();
                    d.descendants()
                        .filter(|n| {
                            n.has_tag_name("g") && n.attribute("data-mml-node") == Some("mpadded")
                        })
                        .filter_map(|n| n.attribute("style").map(str::to_owned))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    styles(&svg),
                    styles(source),
                    "{expression}: padded border CSS"
                );
                // Original mpadded.addChildren retains the phantom wrapper
                // after border paint; its copied argument stays unpainted.
                let children = |raw: &str| {
                    let d = roxmltree::Document::parse(raw).unwrap();
                    d.descendants()
                        .filter(|n| {
                            n.attribute("data-mml-node") == Some("mpadded")
                                && n.attribute("style")
                                    .is_some_and(|s| s.contains("border-top:"))
                        })
                        .map(|n| {
                            (
                                n.children()
                                    .filter(|c| c.is_element())
                                    .map(|c| {
                                        (
                                            c.tag_name().name().to_owned(),
                                            c.attribute("data-mml-node").map(str::to_owned),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                                n.parent()
                                    .unwrap()
                                    .children()
                                    .filter(|c| c.is_element())
                                    .map(|c| c.attribute("data-mml-node").map(str::to_owned))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    children(&svg),
                    children(source),
                    "{expression}: padded children"
                );
                if matches!(
                    expression,
                    r"\overbracket{x}"
                        | r"\underbracket{x}"
                        | r"\overbracket{a+b}"
                        | r"\underbracket{a+b}"
                        | r"\textstyle \overbracket{\sum}"
                        | r"\scriptstyle \overbracket{\sum}"
                        | r"\textstyle \underbracket{\sum}"
                        | r"\scriptstyle \underbracket{\sum}"
                ) {
                    assert_eq!(svg, source, "{expression}: complete bracket SVG");
                }
                assert_eq!(
                    svg_import::import_labeled(
                        "bracket",
                        svg.as_bytes(),
                        [10_000_000; 9],
                        None,
                        None
                    )
                    .err(),
                    svg_import::import_labeled(
                        "bracket",
                        source.as_bytes(),
                        [10_000_000; 9],
                        None,
                        None
                    )
                    .err(),
                    "{expression}: original scene admission"
                );
            }
            (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected, "{expression}"),
            (actual, expected) => panic!("{expression}: {actual:?} vs {expected:?}"),
        }
    }
}
