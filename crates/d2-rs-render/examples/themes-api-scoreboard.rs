// SPDX-License-Identifier: MPL-2.0
//! Native transport for original d2themes element test calls.
use d2_rs_render::themes::ThemableElement;
use serde_json::{Value, json};

fn evaluate(request: &Value) -> Value {
    let result = std::panic::catch_unwind(|| {
        let tag = request["tag"].as_str().unwrap_or("rect");
        let mut element = ThemableElement::new(tag, None);
        if request["op"] == "mask_url" {
            element.set_mask_url(request["value"].as_str().unwrap());
            return String::new();
        }
        macro_rules! strings {
            ($($source:literal => $field:ident),* $(,)?) => { $(
                element.$field = request["strings"][$source].as_str().unwrap().into();
            )* };
        }
        strings! {
            "D"=>d, "Points"=>points, "Transform"=>transform, "Href"=>href,
            "Xmlns"=>xmlns, "Fill"=>fill, "Stroke"=>stroke, "StrokeDashArray"=>stroke_dash_array,
            "BackgroundColor"=>background_color, "Color"=>color, "ClassName"=>class_name,
            "Style"=>style, "Mask"=>mask, "ClipPath"=>clip_path, "Attributes"=>attributes,
            "Content"=>content, "FillPattern"=>fill_pattern,
        }
        macro_rules! numbers {
            ($($source:literal => $field:ident),* $(,)?) => { $(
                element.$field = f64::from_bits(u64::from_str_radix(
                    request["numbers"][$source].as_str().unwrap(), 16).unwrap());
            )* };
        }
        numbers! {
            "X"=>x,"X1"=>x1,"X2"=>x2,"Y"=>y,"Y1"=>y1,"Y2"=>y2,
            "Width"=>width,"Height"=>height,"R"=>r,"Rx"=>rx,"Ry"=>ry,"Cx"=>cx,"Cy"=>cy,
        }
        if !request["inner"].is_null() {
            let value = request["inner"]["value"].as_str().unwrap();
            match request["inner"]["kind"].as_str().unwrap() {
                "text" => element.set_text(value),
                "trusted" => element.set_inner_svg(d2_rs_render::svg::trusted_fragment(value)),
                kind => panic!("unknown inner kind {kind}"),
            }
        }
        element.render()
    });
    match result {
        Ok(value) => json!({"value":value}),
        Err(_) => json!({"panic":true}),
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let request: Value = serde_json::from_slice(
        &std::fs::read(std::env::args().nth(1).expect("request.json")).unwrap(),
    )
    .unwrap();
    let result: Vec<_> = request.as_array().unwrap().iter().map(evaluate).collect();
    println!("{}", json!(result));
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_pinned_theme_calls() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/themes-api.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 15);
        for case in cases {
            assert_eq!(
                super::evaluate(&case["request"]),
                case["got"],
                "{}",
                case["name"]
            );
        }
    }
}
