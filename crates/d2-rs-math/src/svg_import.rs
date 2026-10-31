// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/import.go ImportNodeWithOptions.
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
    let color = current_color.unwrap_or([0, 0, 0, 255]);
    let document = super::svg_xml_document::parse_bytes(source, data, limits, cancel)?;
    let mut compiled = Compilation::with_images(&document);
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
