// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original local-icon and vector-retention calls.
// Oracle tier: replay_original_calls reads a regenerated fixture (docs/FIXTURES.md).
#![cfg_attr(not(feature = "oracle-fixtures"), allow(dead_code, unused_imports))]
use super::super::{
    framing::NativeSceneOptions,
    native_solids::solid_api_tests as solid,
    render::{Options, build_native_scene_with_options},
    vector_surface_svg::native_surface_svg,
};
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::{Path as FilePath, PathBuf};

#[derive(Default)]
struct Sessions {
    painters: HashMap<u64, SurfaceIconPainter>,
    textures: std::vec::Vec<Rc<Texture>>,
    created: std::vec::Vec<PathBuf>,
}
impl Drop for Sessions {
    fn drop(&mut self) {
        self.painters.clear();
        for root in &self.created {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
impl Sessions {
    fn install_files(&mut self, root: &FilePath, files: &serde_json::Map<String, Value>) {
        // Captured roots are the Go test's TempDir inside the capture
        // workdir: historical captures ran under .reference/codex-runs, the
        // recipe (regen-fixtures.py) under oracle-cache/work, which it
        // removes after the run. Replay recreates them; never elsewhere.
        let workspace = FilePath::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        assert!(
            root.starts_with(workspace.join(".reference/codex-runs"))
                || root.starts_with(workspace.join("oracle-cache/work"))
                || root.starts_with(replay_temp_root()),
            "unexpected captured asset root {}",
            root.display()
        );
        if !self.created.iter().any(|p| p == root) {
            assert!(
                !root.exists(),
                "replay requires a fresh writable asset root"
            );
            std::fs::create_dir_all(root).unwrap();
            self.created.push(root.into());
        }
        for (name, data) in files {
            assert_eq!(FilePath::new(name).file_name().unwrap(), name.as_str());
            let data = STANDARD.decode(data.as_str().unwrap()).unwrap();
            let path = root.join(name);
            if path.exists() {
                assert_eq!(std::fs::read(path).unwrap(), data)
            } else {
                std::fs::write(path, data).unwrap()
            }
        }
    }
    fn replay(&mut self, v: &Value) -> Value {
        match v["op"].as_str().unwrap() {
            "icon_new" => {
                let supplied = &v["assets"];
                let assets = if supplied.is_null() {
                    None
                } else if let Some(owner) = supplied["owner"].as_u64() {
                    Some(d2_rs_scenebuild::AssetOptions {
                        resolver_impl: Some(self.painters[&owner].resolver.clone()),
                        ..Default::default()
                    })
                } else {
                    let root = PathBuf::from(supplied["root"].as_str().unwrap());
                    self.install_files(&root, supplied["files"].as_object().unwrap());
                    let policy = d2_rs_scenebuild::ImageLocalFiles::rooted(&root).unwrap();
                    let resolver = d2_rs_scenebuild::RasterImageResolver::new(
                        FilePath::new(""),
                        serde_json::from_value(supplied["limits"].clone()).unwrap(),
                        Arc::new(policy),
                    )
                    .unwrap();
                    Some(d2_rs_scenebuild::AssetOptions {
                        resolver_impl: Some(Arc::new(resolver)),
                        ..Default::default()
                    })
                };
                let result =
                    SurfaceIconPainter::new(v["count"].as_i64().unwrap(), assets.as_ref(), None);
                let output = super::super::text::api_tests::constructor(&result);
                if let Ok(p) = result {
                    self.painters.insert(v["id"].as_u64().unwrap(), p);
                }
                output
            }
            "icon" => {
                let source: crate::target::Url =
                    serde_json::from_value(v["source"].clone()).unwrap();
                if !v["local_file"].is_null() {
                    let path = FilePath::new(&source.path);
                    let files =
                        json!({path.file_name().unwrap().to_str().unwrap():v["local_file"]});
                    self.install_files(path.parent().unwrap(), files.as_object().unwrap());
                }
                let p = self.painters.get_mut(&v["id"].as_u64().unwrap()).unwrap();
                let result = p.texture(
                    &source,
                    v["width"].as_f64().unwrap(),
                    v["depth"].as_f64().unwrap(),
                    v["radius"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                );
                let (texture, error) = match result {
                    Ok(t) => (t, String::new()),
                    Err(e) => (None, e),
                };
                let mut id = 0;
                if let Some(t) = &texture {
                    id = if let Some(i) = self.textures.iter().position(|p| Rc::ptr_eq(p, t)) {
                        i + 1
                    } else {
                        self.textures.push(t.clone());
                        self.textures.len()
                    };
                }
                json!({"error":error,"id":id,"texture":solid::texture(texture.as_ref())})
            }
            "capture" => {
                let target: crate::target::Diagram =
                    serde_json::from_value(v["target"].clone()).unwrap();
                let scene = crate::scene::build_scene(&target, Some(&Default::default())).unwrap();
                let result = build_native_scene_with_options(
                    &scene,
                    v["width"].as_i64().unwrap(),
                    v["height"].as_i64().unwrap(),
                    &Options::default(),
                    NativeSceneOptions {
                        defer_raster: true,
                        vector: true,
                        // vector_surface_test.go:340 explicitly supplies
                        // nativeVectorContext independently of cap coverage.
                        retain_vector_surfaces: true,
                        ..Default::default()
                    },
                    &Default::default(),
                );
                let mut out = json!({"error":"","scene":scene,"target":target,"mesh":null,"surface_ids":null,"surfaces":null});
                match result {
                    Err(e) => out["error"] = json!(e),
                    Ok(n) => {
                        let mut surfaces = vec![];
                        let mut bindings = vec![];
                        let mut fragments = vec![];
                        for t in &n.triangles {
                            let surface =
                                t.material.as_ref().and_then(|m| m.borrow().vector.clone());
                            let id = if let Some(s) = surface {
                                if let Some(i) = surfaces.iter().position(|p| Rc::ptr_eq(p, &s)) {
                                    i + 1
                                } else {
                                    let (svg, error) = match native_surface_svg(&s, "fixture") {
                                        Ok(svg) => (svg, String::new()),
                                        Err(e) => (String::new(), e),
                                    };
                                    fragments.push(json!({"svg":svg,"error":error}));
                                    surfaces.push(s);
                                    surfaces.len()
                                }
                            } else {
                                0
                            };
                            bindings.push(id);
                        }
                        out["mesh"] = solid::mesh(&n.triangles);
                        out["surface_ids"] = json!(bindings);
                        out["surfaces"] = json!(fragments);
                    }
                }
                out
            }
            _ => panic!("unknown artwork operation"),
        }
    }
}

/// Fresh per-process directory standing in for the captured Go TempDir root.
fn replay_temp_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("d2rs-artwork-replay-{}", std::process::id()))
    })
    .clone()
}

#[cfg(feature = "oracle-fixtures")]
#[test]
fn replay_original_calls() {
    let input = std::env::var("D2RS_GEOMETRY_REQUEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            d2_rs_testdata::oracle_fixture_path!("tests/fixtures/artwork-api.json")
        });
    // The recipe writes the Go t.TempDir() roots as @D2RS_TMP@/<Test>/<NNN>
    // (scripts/isometric_oracle_fixtures.py); requests and expected errors
    // both name that root, so it is replaced by one fresh replay directory.
    let text = String::from_utf8(std::fs::read(input).unwrap()).unwrap();
    let root = replay_temp_root();
    let cases: Value =
        serde_json::from_str(&text.replace("@D2RS_TMP@", root.to_str().unwrap())).unwrap();
    let output: std::vec::Vec<_> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let mut sessions = Sessions::default();
            c["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| sessions.replay(&o["request"]))
                .collect::<std::vec::Vec<_>>()
        })
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    if let Ok(path) = std::env::var("D2RS_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec(&output).unwrap()).unwrap();
    } else {
        let expected: std::vec::Vec<_> = cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| o["got"].clone())
                    .collect::<std::vec::Vec<_>>()
            })
            .collect();
        super::super::geometry_api_tests::assert_same(&json!(expected), &json!(output));
    }
}
