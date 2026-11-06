// SPDX-License-Identifier: MPL-2.0
//! Original d2lib public-call adapter. No oracle output enters native calls.
#[allow(dead_code)]
#[cfg(not(test))]
mod native {
    include!("../src/main.rs");
    pub fn asset_error(path: &std::path::Path, error: std::io::Error) -> String {
        local_asset_read_error(path, error)
    }
    pub fn evaluate(request: &serde_json::Value) -> serde_json::Value {
        use d2_rs_lib::config::{self, LibraryCompileOptions, LibraryConfig, LibraryRenderOptions};
        use serde_json::json;
        if request["callback_contract"] == true {
            return json!({"unsupported":"d2lib layout resolver/reuse callback API"});
        }
        let mut compile: LibraryCompileOptions =
            serde_json::from_value(request["options"].clone()).unwrap();
        let mut render: LibraryRenderOptions =
            serde_json::from_value(request["options"]["Render"].clone()).unwrap();
        let options_json = |c: &LibraryCompileOptions, r: &LibraryRenderOptions| {
            let mut out = serde_json::to_value(c).unwrap();
            out["Render"] = serde_json::to_value(r).unwrap();
            out
        };
        match request["op"].as_str().unwrap() {
            "configs" => {
                let config: Option<LibraryConfig> =
                    serde_json::from_value(request["config"].clone()).unwrap();
                config::apply_configs(config.as_ref(), &mut compile, &mut render);
                options_json(&compile, &render)
            }
            "defaults" => {
                config::apply_defaults(&mut compile, &mut render);
                options_json(&compile, &render)
            }
            "compile" => {
                let input = request["input"].as_str().unwrap().as_bytes();
                let source = match source_config(input, "") {
                    Ok(source) => source.as_ref().map(LibraryConfig::from),
                    Err(error) => return json!({"unsupported":error}),
                };
                let config = config::resolve_config(source.as_ref(), &mut compile, &mut render);
                let args = vec![
                    "--path=".to_owned(),
                    format!("--engine={}", compile.layout.as_deref().unwrap()),
                ];
                let mut error = config.as_ref().err().cloned().unwrap_or_default();
                let mut svg = None;
                let mut layout_calls = 0;
                // Empty library diagrams do not invoke a layout resolver.
                if !input.is_empty() && error.is_empty() {
                    match source_layout_with_theme(
                        input,
                        &args,
                        render.theme_id,
                        render.sketch.unwrap(),
                    ) {
                        Err(e) => error = e,
                        Ok((graph, result, labels, styles)) => {
                            // This adapter invocation calls the native core path exactly once;
                            // arbitrary callbacks and recursive board reuse remain unsupported above.
                            layout_calls += 1;
                            let options = d2_rs_render::RenderOptions {
                                pad: render.pad.unwrap() as f64,
                                theme_id: render.theme_id,
                                center: render.center.unwrap(),
                                scale: render.scale,
                                dark_theme_id: render.dark_theme_id,
                                ..Default::default()
                            };
                            match d2_rs_render::render_d2svg(
                                &graph,
                                &result,
                                &labels,
                                &styles,
                                &options,
                                render.sketch.unwrap(),
                            ) {
                                Ok(bytes) => svg = Some(String::from_utf8(bytes).unwrap()),
                                Err(e) => error = e,
                            }
                        }
                    }
                }
                let mut visible = request["options"].clone();
                if request["has_compile_options"] == true {
                    let value = serde_json::to_value(&compile).unwrap();
                    visible["Layout"] = value["Layout"].clone();
                    visible["FontFamily"] = value["FontFamily"].clone();
                }
                if request["has_render_options"] == true {
                    visible["Render"] = serde_json::to_value(&render).unwrap();
                }
                let mut out = json!({"config":if error.is_empty(){serde_json::to_value(config.unwrap()).unwrap()}else{serde_json::Value::Null},"error":error,"options":visible,"layout_calls":layout_calls});
                if let Some(svg) = svg {
                    out["svg"] = json!(svg);
                }
                out
            }
            _ => json!({"unsupported":"unknown operation"}),
        }
    }
}
#[cfg(not(test))]
fn local_asset_read_error(path: &std::path::Path, error: std::io::Error) -> String {
    native::asset_error(path, error)
}
#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    let results: Vec<_> = requests.iter().map(native::evaluate).collect();
    println!("{}", serde_json::to_string(&results)?);
    Ok(())
}
