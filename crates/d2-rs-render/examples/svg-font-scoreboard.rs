// SPDX-License-Identifier: MPL-2.0
//! Original direct font rendering and ThemeCSS calls, with input-only registration.
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_render::{SvgRenderOptions, target::TargetDiagram};
use serde_json::{Value, json};

fn evaluate(request: &Value) -> Value {
    let o = &request["opts"];
    let theme =
        |key: &str| (!o[key].is_null()).then(|| serde_json::from_value(o[key].clone()).unwrap());
    let options = SvgRenderOptions {
        theme_id: o["ThemeID"].as_i64(),
        dark_theme_id: o["DarkThemeID"].as_i64(),
        theme_overrides: theme("ThemeOverrides"),
        dark_theme_overrides: theme("DarkThemeOverrides"),
        ..Default::default()
    };
    if request["api"] == "ThemeCSS" {
        return match d2_rs_render::svg_theme_css(request["hash"].as_str().unwrap(), &options) {
            Ok(output) => json!({"output":output,"error":""}),
            Err(error) => json!({"output":"","error":error}),
        };
    }
    for family in request["families"].as_array().into_iter().flatten() {
        let decode = |key: &str| family[key].as_str().map(|s| STANDARD.decode(s).unwrap());
        let (regular, italic, bold, semibold) = (
            decode("regular"),
            decode("italic"),
            decode("bold"),
            decode("semibold"),
        );
        if let Err(error) = d2_rs_text::d2fonts::add_font_family(
            family["name"].as_str().unwrap(),
            regular.as_deref(),
            italic.as_deref(),
            bold.as_deref(),
            semibold.as_deref(),
        ) {
            return json!({"output":null,"error":error});
        }
    }
    let target = TargetDiagram::decode_json(&serde_json::to_vec(&request["diagram"]).unwrap())
        .unwrap()
        .unwrap();
    match target.render_flat_svg(&options) {
        Ok(output) => json!({"output":STANDARD.encode(output),"error":""}),
        Err(error) => json!({"output":null,"error":error}),
    }
}
fn main() {
    let path = std::env::args().nth(1).expect("request.json");
    let request: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!("{}", evaluate(&request));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_registered_font_and_theme_css_calls() {
        let rows: Vec<Value> =
            serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/svg-font-api.json")).unwrap();
        assert_eq!(rows.len(), 6);
        for row in rows {
            assert_eq!(evaluate(&row["request"]), row["got"], "{}", row["name"]);
        }
    }
}
