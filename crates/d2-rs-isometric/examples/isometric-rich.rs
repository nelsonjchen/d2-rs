// SPDX-License-Identifier: MPL-2.0
//! Replay actual textPainter calls captured by the Go mesh builder.
use d2_rs_isometric::img::{
    rich_labels::RichLabelPainter,
    text::LabelTextStyle,
    vector_surface::{native_vector_context, native_vector_for_texture},
    vector_surface_svg::native_surface_svg,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let input: Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let mut painters = std::collections::HashMap::new();
    let mut results = vec![];
    for call in input.as_array().ok_or("calls")? {
        let id = call["painter"].as_u64().ok_or("painter")?;
        if !painters.contains_key(&id) {
            let mut p = RichLabelPainter::new(
                call["count"].as_i64().ok_or("count")?,
                native_vector_context(),
            )?;
            p.primary = call["primary"].as_str().unwrap_or("").into();
            p.mono = call["mono"].as_str().unwrap_or("").into();
            p.theme_id = call["themeID"].as_i64();
            p.theme_overrides = serde_json::from_value(call["overrides"].clone())?;
            p.output_density = call["density"].as_f64().ok_or("density")?;
            painters.insert(id, p);
        }
        let p = painters.get_mut(&id).unwrap();
        let style: LabelTextStyle = serde_json::from_value(call["style"].clone())?;
        let actual = match p.texture(&serde_json::from_value(call["value"].clone())?, &style) {
            Err(e) => json!({"error":e}),
            Ok(texture) => {
                let mut result =
                    json!({"viewBox": p.last_document.as_ref().ok_or("document")?.view_box});
                if let Some(t) = texture {
                    result["width"] = json!(t.width);
                    result["height"] = json!(t.height);
                    result["pixels"] = json!(format!("{:x}", Sha256::digest(&*t.pixels.borrow())));
                    if let Some(v) = native_vector_for_texture(&p.vectors, &t) {
                        match native_surface_svg(&v, "text") {
                            Ok(svg) => result["surface"] = json!(svg),
                            Err(e) => result["surfaceError"] = json!(e),
                        }
                    }
                }
                result
            }
        };
        results.push(actual);
    }
    std::fs::write(&args[1], serde_json::to_vec(&results)?)?;
    Ok(())
}
