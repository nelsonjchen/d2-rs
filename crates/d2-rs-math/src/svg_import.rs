// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2svgimport/import.go ImportNodeWithOptions.
// Source labels are already redacted; locator admission remains separate.
use super::{
    svg_compile::Compilation,
    svg_scene::{Node, Scene, Work},
    svg_viewport::Viewport,
    svg_xml_document::Document,
};
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Metrics {
    pub source_bytes: usize,
    pub parsed_elements: usize,
    pub parsed_attributes: usize,
    pub parsed_attribute_bytes: usize,
    pub parsed_path_commands: usize,
    pub parsed_transform_funcs: usize,
    pub declared_resources: usize,
    pub expanded_use_instances: usize,
    pub emitted_elements: usize,
    pub emitted_path_commands: usize,
    pub embedded_raster_assets: usize,
    pub embedded_raster_bytes: usize,
    pub decoded_raster_bytes: usize,
}
#[derive(Debug)]
pub struct Imported {
    pub scene: Scene,
    pub assets: super::svg_image::Assets,
    pub viewport: Viewport,
    pub metrics: Metrics,
}
pub struct PendingImage {
    pub document: Document,
    pub compiled: Compilation,
    pub index: usize,
}
pub enum Outcome {
    Imported(Imported),
    Image(PendingImage),
}
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
/// Native importer orchestration for an already-redacted diagnostic label.
/// Includes owned embedded raster assets and image scene primitives.
pub fn import_labeled(
    source: &str,
    data: &[u8],
    limits: [usize; 9],
    current_color: Option<[u8; 4]>,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Outcome, String> {
    import_labeled_with_decoded_limit(source, data, limits, current_color, 0, cancel)
}

/// Import with an independent embedded-raster decoded-pixel ceiling.
/// Zero retains the original source-byte fallback.
pub fn import_labeled_with_decoded_limit(
    source: &str,
    data: &[u8],
    limits: [usize; 9],
    current_color: Option<[u8; 4]>,
    decoded_limit: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<Outcome, String> {
    let color = current_color.unwrap_or([0, 0, 0, 255]);
    let document = super::svg_xml_document::parse_bytes(source, data, limits, cancel)?;
    let mut compiled = Compilation::with_images(&document);
    compiled.image_assets.as_mut().unwrap().decoded_limit = decoded_limit;
    if let super::svg_compile::Progress::Image(index) =
        super::svg_compile::compile(source, &document, &mut compiled, limits, color, cancel)?
    {
        return Ok(Outcome::Image(PendingImage {
            document,
            compiled,
            index,
        }));
    }
    let viewport = super::svg_viewport::compile(
        source,
        &document.elements[0].attributes.values,
        false,
        compiled.work.mathjax_root,
        cancel,
    )?;
    let mut inherited = super::svg_style::Style::default();
    inherited.paint.color = color;
    inherited.viewport = viewport.view_box;
    let mut scene = Scene::default();
    let mut work = Work {
        budget: super::svg_primitive::Budget {
            max_commands: limits[5],
            max_elements: limits[2],
            ..Default::default()
        },
        max_uses: limits[8],
        max_depth: limits[7],
        ..Default::default()
    };
    super::svg_scene::instantiate(
        source,
        &document,
        &compiled,
        0,
        inherited,
        String::new(),
        0,
        false,
        &mut scene,
        &mut work,
        cancel,
    )?;
    if scene.root.is_none() {
        let index = scene.nodes.len();
        scene.nodes.push(Node {
            id: document.elements[0].attributes.id.clone(),
            classes: Vec::new(),
            transform: [1., 0., 0., 1., 0., 0.],
            opacity: 1.,
            clip: None,
            primitive: None,
            children: Vec::new(),
        });
        scene.root = Some(index);
    }
    context(cancel)?;
    if super::svg_bounds::validate_imported(&scene, scene.root, viewport.transform, cancel).is_err()
    {
        context(cancel)?;
        return Err(format!(
            "d2svgimport: {source}: imported scene has invalid or non-finite visual bounds"
        ));
    }
    context(cancel)?;
    let metrics = Metrics {
        source_bytes: data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data).len(),
        parsed_elements: document.counts[0],
        parsed_attributes: document.counts[1],
        parsed_attribute_bytes: document.counts[2],
        parsed_path_commands: compiled.parsed_commands,
        parsed_transform_funcs: compiled.work.transforms,
        declared_resources: document.ids.len()
            + compiled.image_assets.as_ref().unwrap().entries.len(),
        expanded_use_instances: work.uses,
        emitted_elements: work.budget.elements,
        emitted_path_commands: work.budget.commands,
        embedded_raster_assets: compiled.image_assets.as_ref().unwrap().entries.len(),
        embedded_raster_bytes: compiled.image_assets.as_ref().unwrap().encoded_bytes as usize,
        decoded_raster_bytes: compiled.image_assets.as_ref().unwrap().decoded_bytes as usize,
    };
    Ok(Outcome::Imported(Imported {
        scene,
        assets: compiled.image_assets.take().unwrap(),
        viewport,
        metrics,
    }))
}
#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[test]
    fn original_import_result_scene_viewport_metrics_options_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-import-result-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let token = d2_rs_core::CancellationToken::new();
            let cancel = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = import_labeled(
                "math",
                row["input"].as_str().unwrap().as_bytes(),
                serde_json::from_value(row["limits"].clone()).unwrap(),
                row["custom"].as_bool().unwrap().then_some([1, 2, 3, 200]),
                cancel,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "{row}"
            );
            if let Ok(outcome) = result {
                let Outcome::Imported(imported) = outcome else {
                    panic!("unexpected pending image: {row}")
                };
                let metrics: Metrics =
                    serde_json::from_value(row["output"]["metrics"].clone()).unwrap();
                assert_eq!(imported.metrics, metrics, "{row}");
                let viewport: Viewport =
                    serde_json::from_value(row["output"]["viewport"].clone()).unwrap();
                assert_eq!(imported.viewport, viewport, "{row}");
                let tree: super::super::svg_scene::tests::Tree =
                    serde_json::from_value(row["output"]["tree"].clone()).unwrap();
                assert_eq!(
                    super::super::svg_scene::tests::tree(
                        &imported.scene,
                        imported.scene.root.unwrap()
                    ),
                    tree,
                    "{row}"
                );
            }
        }
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod image_tests {
    use super::*;
    #[test]
    fn original_complete_image_import_tree_assets_viewport_metrics_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-image-import-v090.json"
        ))
        .unwrap();
        for (i, row) in rows.as_array().unwrap().iter().enumerate() {
            let token = d2_rs_core::CancellationToken::new();
            let c = match row["cancel"].as_str().unwrap() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let result = import_labeled(
                "math",
                row["input"].as_str().unwrap().as_bytes(),
                serde_json::from_value(row["limits"].clone()).unwrap(),
                row["custom"].as_bool().unwrap().then_some([1, 2, 3, 200]),
                c,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str).unwrap_or(""),
                row["error"].as_str().unwrap(),
                "control {i}"
            );
            if let Ok(outcome) = result {
                let Outcome::Imported(out) = outcome else {
                    panic!("pending image control {i}")
                };
                let expected = &row["output"];
                let assets: super::super::svg_image::Assets =
                    serde_json::from_value(expected["assets"].clone()).unwrap();
                assert_eq!(out.assets, assets, "control {i}");
                let metrics: Metrics = serde_json::from_value(expected["metrics"].clone()).unwrap();
                assert_eq!(out.metrics, metrics, "control {i}");
                let viewport: Viewport =
                    serde_json::from_value(expected["viewport"].clone()).unwrap();
                assert_eq!(out.viewport, viewport, "control {i}");
                let tree: super::super::svg_scene::tests::Tree =
                    serde_json::from_value(expected["tree"].clone()).unwrap();
                assert_eq!(
                    super::super::svg_scene::tests::tree(&out.scene, out.scene.root.unwrap()),
                    tree,
                    "control {i}"
                );
            }
        }
    }
}

#[cfg(test)]
mod catalog_contracts {
    use super::*;
    use crate::svg_primitive::Paint;
    fn import(source: &str) -> Imported {
        match import_labeled(
            "icon",
            source.as_bytes(),
            [262144, 32, 2048, 4096, 262144, 16384, 512, 8, 512],
            None,
            None,
        )
        .unwrap()
        {
            Outcome::Imported(imported) => imported,
            _ => panic!("image unexpectedly pending"),
        }
    }
    #[test]
    fn radial_defaults_percentages_and_opacity() {
        // Upstream 8558856dc gradient.go: radial percentages use the normalized diagonal.
        let imported = import(
            r##"<svg viewBox="0 0 40 20"><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="25%" fr="10%" spreadMethod="repeat"><stop stop-color="red"/><stop offset="100%" stop-color="blue"/></radialGradient><rect width="40" height="20" fill="url(#g)" fill-opacity=".5"/><svg width="10" height="10" viewBox="0 0 8 4"><rect width="8" height="4" fill="url(#g)"/></svg></svg>"##,
        );
        let paints: Vec<_> = imported
            .scene
            .nodes
            .iter()
            .filter_map(|n| n.primitive.as_ref()?.fill.as_ref())
            .collect();
        assert_eq!(paints.len(), 2);
        for (paint, w, h, alpha) in [
            (paints[0], 40_f64, 20_f64, 128),
            (paints[1], 8_f64, 4_f64, 255),
        ] {
            let Paint::Radial {
                center,
                focal,
                radius,
                focal_radius,
                units,
                spread,
                stops,
                ..
            } = paint
            else {
                panic!("radial")
            };
            assert_eq!(*center, [w * 0.25, h * 0.5]);
            assert_eq!(center, focal);
            let diagonal = (w / std::f64::consts::SQRT_2).hypot(h / std::f64::consts::SQRT_2);
            assert_eq!(*radius, diagonal * 0.5);
            assert_eq!(*focal_radius, diagonal * 0.1);
            assert_eq!((*units, *spread), (1, 2));
            assert_eq!(stops[0].color, [255, 0, 0, alpha]);
        }
    }
    #[test]
    fn radial_rejects_negative_radii_and_invalid_units() {
        for attrs in [
            r#"r="-1""#,
            r#"fr="-1%""#,
            r#"gradientUnits="bad""#,
            r#"r="2 %""#,
        ] {
            let source =
                format!(r#"<svg width="4" height="4"><radialGradient id="g" {attrs}/></svg>"#);
            assert!(
                import_labeled(
                    "icon",
                    source.as_bytes(),
                    [262144, 32, 2048, 4096, 262144, 16384, 512, 8, 512],
                    None,
                    None
                )
                .is_err(),
                "{attrs}"
            );
        }
    }
    #[test]
    fn gradient_defaults_and_referencing_viewports() {
        // Upstream 8558856dc TestIconGradientCoordinatesAndPixels.
        let imported = import(
            r##"<svg viewBox="0 0 40 20"><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="25%" spreadMethod="reflect"><stop stop-color="red"/><stop offset="100%" stop-color="blue"/></linearGradient><rect width="40" height="20" fill="url(#g)"/><svg width="10" height="10" viewBox="0 0 8 4"><rect width="8" height="4" fill="url(#g)"/></svg></svg>"##,
        );
        let paints: Vec<_> = imported
            .scene
            .nodes
            .iter()
            .filter_map(|n| n.primitive.as_ref())
            .filter_map(|p| p.fill.as_ref())
            .collect();
        assert_eq!(paints.len(), 2);
        for (paint, expected) in paints.into_iter().zip([(10., 40.), (2., 8.)]) {
            let Paint::Linear {
                start,
                end,
                units,
                spread,
                stops,
                ..
            } = paint
            else {
                panic!("linear")
            };
            assert_eq!((start[0], end[0]), expected);
            assert_eq!((*units, *spread), (1, 1));
            assert_eq!(stops[0].color, [255, 0, 0, 255]);
            assert_eq!(stops[1].color, [0, 0, 255, 255]);
        }
        let imported = import(
            r##"<svg viewBox="0 0 40 20"><linearGradient id="g"><stop/><stop offset="100%"/></linearGradient><rect width="40" height="20" fill="url(#g)"/></svg>"##,
        );
        let paint = imported
            .scene
            .nodes
            .iter()
            .find_map(|n| n.primitive.as_ref().and_then(|p| p.fill.as_ref()))
            .unwrap();
        assert!(matches!(
            paint,
            Paint::Linear {
                start: [0., 0.],
                end: [1., 0.],
                units: 0,
                ..
            }
        ));
    }
    #[test]
    fn empty_degenerate_gradients_and_clamped_stops() {
        // Upstream 8558856dc TestIconGradientStopDefaultsAndDegeneratePaint.
        let imported = import(
            r##"<svg viewBox="0 0 4 2"><defs><linearGradient/><linearGradient id="empty"/><linearGradient id="step" x2="0" spreadMethod="repeat"><stop offset="-20%"/><stop offset=".75"/><stop offset=".25"/><stop offset="150%" stop-color="red"/></linearGradient></defs><rect width="2" height="2" fill="url(#empty)"/><rect x="2" width="2" height="2" fill="url(#step)"/></svg>"##,
        );
        let primitives: Vec<_> = imported
            .scene
            .nodes
            .iter()
            .filter_map(|n| n.primitive.as_ref())
            .collect();
        // compile.go svgGeometry.primitive omits an unpainted rectangle.
        assert_eq!(primitives.len(), 1);
        assert_eq!(primitives[0].geometry.bounds[0], 2.);
        let Some(Paint::Linear {
            start,
            end,
            stops,
            spread,
            ..
        }) = &primitives[0].fill
        else {
            panic!("linear")
        };
        assert_eq!(start, end);
        assert_eq!(*spread, 2);
        assert_eq!(
            stops.iter().map(|s| s.offset).collect::<Vec<_>>(),
            [0., 0.75, 0.75, 1.]
        );
    }
    #[test]
    fn editor_inert_properties_and_unreferenced_clips() {
        // Upstream 8558856dc style.go/boilerplate.go and TestIconResourceLimitsAndVisibility.
        let imported = import(
            r##"<svg width="8" height="8"><style>.one{isolation:isolate;mix-blend-mode:normal;vector-effect:none;enable-background:accumulate}</style><clipPath><rect width="2" height="2"/></clipPath><g class="one" style="enable-background:new 0 0 8 8"><svg width="8" height="8" viewBox="0 0 8 8" enable-background="inherit"><path d="M0 0H8V8Z" href="#editor" text-indent="0" text-align="start" text-decoration-line="none" text-decoration-style="solid" text-decoration-color="black" text-transform="none" block-progression="tb" white-space="normal" font-size="16" font-family="sans-serif" solid-color="red" solid-opacity="1" fill="red"/></svg></g><defs><path id="p" d="M0 0H1V1Z"/></defs><use href="#p" overflow="visible"/></svg>"##,
        );
        assert_eq!(
            imported
                .scene
                .nodes
                .iter()
                .filter(|n| n.primitive.is_some())
                .count(),
            2
        );
        for body in [
            r#"<g isolation="bad"/>"#,
            r#"<g mix-blend-mode="multiply"/>"#,
            r#"<path vector-effect="non-scaling-stroke"/>"#,
            r#"<g overflow="visible"/>"#,
            r#"<image overflow="hidden"/>"#,
            r#"<g enable-background="new 0 0 0 8"/>"#,
            r##"<path href="#bad id"/>"##,
            r#"<style>.unused{isolation:bad}</style>"#,
        ] {
            let source = format!("<svg width=\"8\" height=\"8\">{body}</svg>");
            assert!(
                import_labeled(
                    "icon",
                    source.as_bytes(),
                    [262144, 32, 2048, 4096, 262144, 16384, 512, 8, 512],
                    None,
                    None
                )
                .is_err(),
                "{body}"
            );
        }
    }
    #[test]
    fn editor_descriptions_optional_metadata_and_class_lists() {
        let imported = import(
            r##"<svg viewBox="0 0 48 24" version="1.2" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:cc="http://creativecommons.org/ns#" xmlns:dc="http://purl.org/dc/elements/1.1/"><desc>Editor description</desc><metadata><rdf:RDF><cc:Work><dc:format>image/svg+xml</dc:format><dc:title>Logo</dc:title></cc:Work></rdf:RDF></metadata><style>.one,.two{fill:red}.two{fill:blue}</style><rect width="4" height="4" class="one"/><rect x="4" width="4" height="4" class="two"/></svg>"##,
        );
        let colors: Vec<_> = imported
            .scene
            .nodes
            .iter()
            .filter_map(|n| n.primitive.as_ref())
            .map(|p| p.fill.clone())
            .collect();
        assert_eq!(
            colors,
            [
                Some(Paint::Solid {
                    color: [255, 0, 0, 255]
                }),
                Some(Paint::Solid {
                    color: [0, 0, 255, 255]
                })
            ]
        );
    }
}
