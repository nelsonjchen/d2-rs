// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2scenebuild/assets.go resolved-SVG cache and vector wrapper.
use super::{
    svg_document_import::DocumentImports,
    svg_path::Command,
    svg_scene::{Clip, Node, Scene},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Vector {
    pub view_box: [f64; 4],
    pub scene: Scene,
}
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AssetCache {
    pub document: DocumentImports,
    pub vectors: BTreeMap<String, Vector>,
    pub by_digest: BTreeMap<String, String>,
    pub by_source: BTreeMap<String, String>,
}
fn context(c: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match c.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
pub fn content_digest(
    mime: &str,
    data: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<String, String> {
    image_content_digest(2, mime, data, cancel)
}
/// Source resolved raster content identity (kind1), independent of alias names.
pub fn raster_content_digest(
    mime: &str,
    data: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<String, String> {
    image_content_digest(1, mime, data, cancel)
}
fn image_content_digest(
    kind: u8,
    mime: &str,
    data: &[u8],
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update([kind, 0]);
    hash.update(mime.as_bytes());
    hash.update([0]);
    for chunk in data.chunks(32768) {
        context(cancel)?;
        hash.update(chunk);
    }
    context(cancel)?;
    Ok(format!("{:x}", hash.finalize()))
}
impl AssetCache {
    /// Source d2scenebuild.resolveUnavailableImageAsset: one native vector per
    /// document, with source aliases and no SVG-import or content-hash charges.
    pub fn resolve_unavailable(&mut self, source: &str) -> String {
        if let Some(id) = self.by_source.get(source) {
            return id.clone();
        }
        let id = "image:unavailable".to_owned();
        self.vectors
            .entry(id.clone())
            .or_insert_with(unavailable_image);
        self.document.resources.entry(id.clone()).or_insert(None);
        self.by_source.insert(source.to_owned(), id.clone());
        id
    }
    /// Resolve already-acquired SVG bytes. The diagnostic label is already redacted.
    /// Acquisition/resolver policy belongs to the caller. Source cache hits bypass work.
    pub fn resolve_svg_labeled(
        &mut self,
        object: &str,
        source_key: &str,
        label: &str,
        mime: &str,
        data: &[u8],
        limits: [usize; 9],
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<String, String> {
        if let Some(id) = self.by_source.get(source_key).filter(|id| !id.is_empty()) {
            return Ok(id.clone());
        }
        let digest = content_digest(mime, data, cancel)
            .map_err(|e| format!("scene: {object} image asset hash: {e}"))?;
        if let Some(id) = self.by_digest.get(&digest).filter(|id| !id.is_empty()) {
            let id = id.clone();
            self.by_source.insert(source_key.into(), id.clone());
            return Ok(id);
        }
        let id = format!("image:{digest}");
        let imported = self
            .document
            .import_labeled(object, label, data, limits, cancel)?;
        let mut scene = imported.scene;
        let content = scene.nodes.len();
        scene.nodes.push(Node {
            id: format!("{id}:content"),
            classes: Vec::new(),
            transform: imported.viewport.transform,
            opacity: 1.,
            clip: None,
            primitive: None,
            children: vec![scene.root.unwrap()],
        });
        let w = imported.viewport.width;
        let h = imported.viewport.height;
        let commands = vec![
            Command {
                points: [[0., 0.], [0.; 2], [0.; 2]],
                ..Default::default()
            },
            Command {
                kind: 1,
                points: [[w, 0.], [0.; 2], [0.; 2]],
                ..Default::default()
            },
            Command {
                kind: 1,
                points: [[w, h], [0.; 2], [0.; 2]],
                ..Default::default()
            },
            Command {
                kind: 1,
                points: [[0., h], [0.; 2], [0.; 2]],
                ..Default::default()
            },
            Command {
                kind: 5,
                ..Default::default()
            },
        ];
        let root = scene.nodes.len();
        scene.nodes.push(Node {
            id: format!("{id}:viewport"),
            classes: Vec::new(),
            transform: [1., 0., 0., 1., 0., 0.],
            opacity: 1.,
            clip: Some(Clip {
                commands,
                fill_rule: 0,
                transform: [1., 0., 0., 1., 0., 0.],
            }),
            primitive: None,
            children: vec![content],
        });
        scene.root = Some(root);
        self.document.resources.insert(id.clone(), None);
        self.vectors.insert(
            id.clone(),
            Vector {
                view_box: [0., 0., w, h],
                scene,
            },
        );
        self.by_digest.insert(digest, id.clone());
        self.by_source.insert(source_key.into(), id.clone());
        Ok(id)
    }
}

/// Source D2 v0.9.0 d2scenebuild/assets.go newUnavailableImageAsset.
fn unavailable_image() -> Vector {
    use super::{
        svg_geometry::Geometry,
        svg_primitive::{Paint, Primitive, Stroke},
    };
    let page = [0xf7, 0xf8, 0xfa, 0xff];
    let outline = [0x7b, 0x83, 0x8c, 0xff];
    let stroke = |color, width, cap| Stroke {
        paint: Paint::Solid { color },
        width,
        cap,
        join: 1,
        miter_limit: 0.,
        dashes: None,
        dash_offset: 0.,
    };
    let path = |points: &[[f64; 2]], close: bool| Geometry {
        kind: 3,
        commands: points
            .iter()
            .enumerate()
            .map(|(i, p)| Command {
                kind: u8::from(i != 0),
                points: [*p, [0.; 2], [0.; 2]],
                ..Default::default()
            })
            .chain(close.then_some(Command {
                kind: 5,
                ..Default::default()
            }))
            .collect(),
        ..Default::default()
    };
    let rect = || Geometry {
        kind: 1,
        bounds: [13., 23., 38., 30.],
        ..Default::default()
    };
    let primitives = [
        (
            "page",
            path(
                &[[7., 3.], [42., 3.], [57., 18.], [57., 61.], [7., 61.]],
                true,
            ),
            Some(page),
            Some(stroke(outline, 2., 1)),
        ),
        ("sky", rect(), Some([0xb8, 0xd7, 0xf2, 0xff]), None),
        (
            "sun",
            Geometry {
                kind: 2,
                center: [23., 31.],
                radii: [4., 4.],
                ..Default::default()
            },
            Some([0xff; 4]),
            None,
        ),
        (
            "hills",
            path(
                &[
                    [13., 49.],
                    [24., 37.],
                    [32., 45.],
                    [39., 39.],
                    [51., 49.],
                    [51., 53.],
                    [13., 53.],
                ],
                true,
            ),
            Some([0x55, 0xb9, 0x4b, 0xff]),
            None,
        ),
        (
            "break",
            path(&[[37., 54.], [51., 40.]], false),
            None,
            Some(stroke(page, 5., 2)),
        ),
        ("image-outline", rect(), None, Some(stroke(outline, 1., 1))),
        (
            "fold",
            path(&[[42., 3.], [42., 18.], [57., 18.]], false),
            None,
            Some(stroke(outline, 2., 1)),
        ),
    ];
    let mut scene = Scene::default();
    for (name, geometry, fill, stroke) in primitives {
        scene.nodes.push(Node {
            id: format!("image-unavailable:{name}"),
            classes: Vec::new(),
            transform: [1., 0., 0., 1., 0., 0.],
            opacity: 1.,
            clip: None,
            primitive: Some(Primitive {
                geometry,
                fill: fill.map(|color| Paint::Solid { color }),
                stroke,
                fill_rule: 0,
            }),
            children: Vec::new(),
        });
    }
    scene.root = Some(scene.nodes.len());
    scene.nodes.push(Node {
        id: "image-unavailable".into(),
        classes: Vec::new(),
        transform: [1., 0., 0., 1., 0., 0.],
        opacity: 1.,
        clip: None,
        primitive: None,
        children: (0..7).collect(),
    });
    Vector {
        view_box: [0., 0., 64., 64.],
        scene,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_images_share_native_vector_without_import_charges() {
        let mut cache = AssetCache::default();
        for source in ["first", "second", "first"] {
            assert_eq!(cache.resolve_unavailable(source), "image:unavailable");
        }
        assert_eq!(cache.vectors.len(), 1);
        assert_eq!(cache.by_source.len(), 2);
        assert!(cache.by_digest.is_empty());
        let mut expected_document = DocumentImports::default();
        expected_document
            .resources
            .insert("image:unavailable".into(), None);
        assert_eq!(cache.document, expected_document);
        let vector = &cache.vectors["image:unavailable"];
        assert_eq!(vector.view_box, [0., 0., 64., 64.]);
        let root = &vector.scene.nodes[vector.scene.root.unwrap()];
        assert_eq!(root.id, "image-unavailable");
        assert_eq!(root.children.len(), 7);
    }
    #[cfg(feature = "oracle-fixtures")]
    #[derive(serde::Deserialize)]
    struct ExpectedVector {
        view_box: [f64; 4],
        tree: super::super::svg_scene::tests::Tree,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[derive(serde::Deserialize)]
    struct ExpectedState {
        document: DocumentImports,
        vectors: BTreeMap<String, ExpectedVector>,
        by_digest: BTreeMap<String, String>,
        by_source: BTreeMap<String, String>,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[derive(serde::Deserialize)]
    struct Call {
        source: String,
        cancel: String,
        error: String,
        id: String,
        state: ExpectedState,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[derive(serde::Deserialize)]
    struct Row {
        input: String,
        limits: [usize; 9],
        before: AssetCache,
        calls: Vec<Call>,
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_resolved_svg_cache_hits_content_aliases_and_complete_vector_wrappers() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-asset-cache-v090.json"
        ))
        .unwrap();
        for (i, row) in rows.into_iter().enumerate() {
            let mut cache = row.before;
            for (n, e) in row.calls.iter().enumerate() {
                let token = d2_rs_core::CancellationToken::new();
                let c = match e.cancel.as_str() {
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
                let result = cache.resolve_svg_labeled(
                    "shape \"n\"",
                    &e.source,
                    "data URI",
                    "image/svg+xml",
                    row.input.as_bytes(),
                    row.limits,
                    c,
                );
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    e.error,
                    "profile {i} call {n}"
                );
                assert_eq!(result.unwrap_or_default(), e.id, "profile {i} call {n}");
                assert_eq!(cache.document, e.state.document, "profile {i} call {n}");
                assert_eq!(cache.by_digest, e.state.by_digest, "profile {i} call {n}");
                assert_eq!(cache.by_source, e.state.by_source, "profile {i} call {n}");
                assert_eq!(cache.vectors.len(), e.state.vectors.len());
                for (id, vector) in &cache.vectors {
                    let expected = &e.state.vectors[id];
                    assert_eq!(vector.view_box, expected.view_box);
                    assert_eq!(
                        super::super::svg_scene::tests::tree(
                            &vector.scene,
                            vector.scene.root.unwrap()
                        ),
                        expected.tree,
                        "profile {i} call {n}"
                    );
                }
            }
        }
    }
}
