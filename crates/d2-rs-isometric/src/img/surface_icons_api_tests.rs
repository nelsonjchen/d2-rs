// SPDX-License-Identifier: MPL-2.0
//! Test-only private resource-budget mutation, matching the original Go test.
use super::super::text::api_tests::{Textures, constructor, context};
use super::super::vector_surface::native_vector_context;
use super::*;
use serde_json::{Value, json};
#[derive(Default)]
pub(crate) struct IconSessions {
    painters: HashMap<u64, SurfaceIconPainter>,
}
impl IconSessions {
    pub(crate) fn replay(&mut self, op: &Value, textures: &mut Textures) -> Value {
        let id = op["id"].as_u64().unwrap();
        match op["op"].as_str().unwrap() {
            "new_icon" => {
                let vectors = if op["vectors"] == true {
                    native_vector_context()
                } else {
                    None
                };
                let mut result = SurfaceIconPainter::new_with_context(
                    context(op),
                    op["count"].as_i64().unwrap(),
                    None,
                    vectors,
                );
                let output = constructor(&result);
                if let Ok(p) = &mut result {
                    p.context = context(op);
                }
                if let Ok(p) = result {
                    self.painters.insert(id, p);
                }
                output
            }
            "icon_source_budget" => {
                self.painters
                    .get_mut(&id)
                    .unwrap()
                    .remaining
                    .max_source_bytes = op["value"].as_i64().unwrap();
                json!({"error":""})
            }
            "icon" => {
                let p = self.painters.get_mut(&id).unwrap();
                p.context = context(op);
                p.output_density = op["density"].as_f64().unwrap();
                let radius = op["radius"]
                    .as_array()
                    .and_then(|v| v.first())
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let raw = op["source"].as_str().unwrap();
                let (scheme, opaque) = raw.split_once(':').unwrap();
                assert_eq!(scheme, "data");
                let source = crate::target::Url {
                    scheme: scheme.into(),
                    opaque: opaque.into(),
                    ..Default::default()
                };
                let (texture, error) = match p.texture(
                    &source,
                    op["width"].as_f64().unwrap(),
                    op["depth"].as_f64().unwrap(),
                    radius,
                ) {
                    Ok(t) => (t, String::new()),
                    Err(e) => (None, e),
                };
                textures.result(texture, error, &p.vectors)
            }
            _ => panic!("unknown icon operation"),
        }
    }
}
