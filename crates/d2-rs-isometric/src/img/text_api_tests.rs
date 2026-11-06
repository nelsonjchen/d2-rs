// SPDX-License-Identifier: MPL-2.0
//! Test-only replay of original Go painter calls. No expected values enter this path.
use super::super::{
    context::RenderContext,
    geometry::LabelSurface,
    rich_labels::{RichLabelPainter, fit_rich_viewport},
    surface_icons::{api_tests::IconSessions, surface_icon_dimensions, surface_icon_layout},
    vector_surface::{native_vector_context, native_vector_for_texture},
    vector_surface_svg::native_surface_svg,
};
use super::*;
use base64::Engine;
use d2_rs_core::CancellationToken;
use serde_json::{Value, json};

pub(crate) fn context(op: &Value) -> RenderContext {
    let token = CancellationToken::new();
    if op["context"].as_str().unwrap() == "context canceled" {
        token.cancel();
    }
    RenderContext::from_token(&token)
}
pub(crate) fn constructor<T>(result: &Result<T, String>) -> Value {
    match result {
        Ok(_) => json!({"error":"","nil":false}),
        Err(e) => json!({"error":e,"nil":true}),
    }
}
#[derive(Default)]
pub(crate) struct Textures {
    values: Vec<Rc<Texture>>,
}
impl Textures {
    pub(crate) fn result(
        &mut self,
        texture: Option<Rc<Texture>>,
        error: String,
        vectors: &VectorCtx,
    ) -> Value {
        let mut out =
            json!({"frame":null,"error":error,"surface":null,"surface_error":"","texture_id":0});
        if let Some(t) = texture {
            let id = if let Some(index) = self.values.iter().position(|p| Rc::ptr_eq(p, &t)) {
                index + 1
            } else {
                self.values.push(t.clone());
                self.values.len()
            };
            out["texture_id"] = json!(id);
            out["frame"] = json!({"width":t.width,"height":t.height,"stride":t.width*4,"pixels":base64::engine::general_purpose::STANDARD.encode(&*t.pixels.borrow())});
            if let Some(v) = native_vector_for_texture(vectors, &t) {
                match native_surface_svg(&v, "surface") {
                    Ok(svg) => out["surface"] = json!(svg),
                    Err(e) => {
                        out["surface"] = json!("");
                        out["surface_error"] = json!(e);
                    }
                }
            }
        }
        out
    }
}
fn float(v: &Value) -> f64 {
    match v.as_str() {
        Some("NaN") => f64::NAN,
        Some("+Inf") => f64::INFINITY,
        Some("-Inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}
fn area(v: &Value) -> LabelSurface {
    LabelSurface {
        center: super::super::mesh::nv(
            v["center"]["x"].as_f64().unwrap(),
            v["center"]["y"].as_f64().unwrap(),
            v["center"]["z"].as_f64().unwrap(),
        ),
        width: float(&v["width"]),
        depth: float(&v["depth"]),
        angle: float(&v["angle"]),
        ..Default::default()
    }
}
fn area_value(s: &LabelSurface) -> Value {
    json!({"center":s.center,"width":s.width,"depth":s.depth,"angle":s.angle})
}
fn replay(case: &Value) -> Value {
    let mut texts = HashMap::<u64, TextPainter>::new();
    let mut rich = HashMap::<u64, RichLabelPainter>::new();
    let mut icons = IconSessions::default();
    let mut textures = Textures::default();
    let mut rows = vec![];
    for op in case["operations"].as_array().unwrap() {
        let kind = op["op"].as_str().unwrap();
        let id = op["id"].as_u64().unwrap_or(0);
        let output = match kind {
            "new_text" => {
                let vectors = if op["vectors"] == true {
                    native_vector_context()
                } else {
                    None
                };
                let mut result = TextPainter::new(op["count"].as_i64().unwrap(), vectors);
                let output = constructor(&result);
                if let Ok(p) = &mut result {
                    p.context = context(op);
                }
                if let Ok(p) = result {
                    texts.insert(id, p);
                }
                output
            }
            "glyph_work" => {
                texts.get_mut(&id).unwrap().glyph_work = op["value"].as_u64().unwrap() as usize;
                json!({"error":""})
            }
            "text" => {
                let p = texts.get_mut(&id).unwrap();
                p.context = context(op);
                p.output_density = float(&op["density"]);
                p.primary = op["primary"].as_str().unwrap().into();
                p.mono = op["mono"].as_str().unwrap().into();
                let style = serde_json::from_value(op["style"].clone()).unwrap();
                let (texture, layout, error) =
                    match p.texture(op["value"].as_str().unwrap(), &style) {
                        Ok((t, l)) => (t, serde_json::to_value(l).unwrap(), String::new()),
                        Err(e) => (
                            None,
                            json!({"Lines":null,"FontSize":0,"LineHeight":0,"Truncated":false}),
                            e,
                        ),
                    };
                let mut result = textures.result(texture, error, &p.vectors);
                result["layout"] = layout;
                result
            }
            "new_rich" => {
                let vectors = if op["vectors"] == true {
                    native_vector_context()
                } else {
                    None
                };
                let mut result = RichLabelPainter::new(op["count"].as_i64().unwrap(), vectors);
                let output = constructor(&result);
                if let Ok(p) = &mut result {
                    p.context = context(op);
                }
                if let Ok(p) = result {
                    rich.insert(id, p);
                }
                output
            }
            "rich" => {
                let p = rich.get_mut(&id).unwrap();
                p.context = context(op);
                p.output_density = float(&op["density"]);
                p.primary = op["primary"].as_str().unwrap().into();
                p.mono = op["mono"].as_str().unwrap().into();
                let shape = serde_json::from_value(op["value"].clone()).unwrap();
                let style = serde_json::from_value(op["style"].clone()).unwrap();
                let (texture, error) = match p.texture(&shape, &style) {
                    Ok(t) => (t, String::new()),
                    Err(e) => (None, e),
                };
                let mut result = textures.result(texture, error, &p.vectors);
                result["document"] = p
                    .last_document
                    .as_ref()
                    .map(|d| json!({"view_box":d.view_box,"links":d.links}))
                    .unwrap_or(Value::Null);
                result
            }
            "new_icon" | "icon" | "icon_source_budget" => icons.replay(op, &mut textures),
            "icon_dimensions" => {
                let (w, h) = surface_icon_dimensions(
                    float(&op["width"]),
                    float(&op["depth"]),
                    op["maximum"].as_u64().unwrap() as usize,
                );
                json!({"error":"","width":w,"height":h})
            }
            "icon_layout" => {
                let (a, b) = surface_icon_layout(
                    &area(&op["surface"]),
                    &serde_json::from_value(op["original"].clone()).unwrap(),
                    float(&op["scale"]),
                    op["kind"].as_str().unwrap(),
                );
                json!({"error":"","icon":area_value(&a),"text":area_value(&b)})
            }
            "dimensions" => {
                let (w, h) = surface_texture_dimensions_at_density(
                    float(&op["width"]),
                    float(&op["depth"]),
                    op["maximum"].as_u64().unwrap() as usize,
                    op["budget"].as_u64().unwrap() as usize,
                    float(&op["density"]),
                );
                json!({"error":"","width":w,"height":h})
            }
            "fit_rich" => {
                let mut doc = Document {
                    view_box: serde_json::from_value(op["box"].clone()).unwrap(),
                    ..Default::default()
                };
                let error = fit_rich_viewport(
                    &mut doc,
                    &serde_json::from_value(op["style"].clone()).unwrap(),
                )
                .err()
                .unwrap_or_default();
                json!({"error":error,"box":doc.view_box})
            }
            "fit_text" => {
                assert_eq!(op["measure"], "rune_count_times_size");
                let layout = fit_native_label(
                    op["value"].as_str().unwrap(),
                    float(&op["width"]),
                    float(&op["depth"]),
                    float(&op["size"]),
                    |s| Ok(s.chars().count() as f64),
                )
                .unwrap();
                json!({"error":"","layout":layout})
            }
            other => panic!("uncaptured operation {other}"),
        };
        rows.push(output);
    }
    json!(rows)
}
#[test]
#[ignore = "requires an original Go capture; invoked by the surface scoreboard"]
fn replay_original_surface_calls() {
    let input = std::env::var("D2RS_SURFACE_REQUEST").expect("surface capture request");
    let output = std::env::var("D2RS_SURFACE_OUTPUT").unwrap();
    assert!(!std::path::Path::new(&output).exists());
    let request: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    std::fs::create_dir(&output).unwrap();
    for (i, case) in request["cases"].as_array().unwrap().iter().enumerate() {
        let value = replay(case);
        std::fs::write(
            std::path::Path::new(&output).join(format!("{i}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
}
