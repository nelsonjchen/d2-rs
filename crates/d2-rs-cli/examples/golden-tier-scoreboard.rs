// SPDX-License-Identifier: MPL-2.0
//! Test adapter for upstream d2sketch, d2exporter and d2svg golden contracts.
//! Reuse the CLI's native compile/measure/layout/export path without a second
//! implementation or a production diagnostic flag.
#[allow(dead_code)]
#[cfg(not(test))]
mod native {
    include!("../src/main.rs");

    pub fn asset_error(path: &std::path::Path, error: std::io::Error) -> String {
        local_asset_read_error(path, error)
    }

    pub fn capture(request: &serde_json::Value, output: &std::path::Path) -> Result<(), String> {
        let input = request["input"].as_str().ok_or("missing input")?.as_bytes();
        // Every tier but d2exporter renders through d2lib.Compile + d2svg.
        let svg_tier = request["tier"] != "exporter";
        let sketch = request["tier"] == "sketch";
        // HandDrawn measurement/export font: sketch, or dark_theme_test.go's
        // explicit FontFamily without sketch rendering.
        let hand_drawn = sketch || request["font_family"] == "handdrawn";
        let engine = request["engine"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("dagre");
        let mut args = vec![format!("--engine={engine}")];
        if !svg_tier {
            args.push("--utf16".into());
        }
        let theme_id = svg_tier.then(|| request["theme_id"].as_i64().unwrap_or(0));
        let (graph, result, labels, styles) =
            source_layout_with_theme(input, &args, theme_id, hand_drawn)?;
        // d2exporter's direct Compile has a nil config for these inputs;
        // the SVG tiers call d2lib.Compile, which hydrates render options.
        let options = d2_rs_render::RenderOptions {
            theme_id,
            include_config_in_hash: svg_tier,
            hand_drawn_font: hand_drawn,
            ..Default::default()
        };
        let target =
            d2_rs_render::target_diagram_json(&graph, &result, &labels, &styles, &options, sketch)?;
        std::fs::write(output.join("target.json"), target).map_err(|e| e.to_string())?;
        if svg_tier {
            // d2svg.Render hashes the diagram with its descendant boards.
            let mut options = options;
            options.d2svg_hash_input = d2svg_subtree_hash_input(
                input,
                &args,
                None,
                || {
                    d2_rs_render::target_hash_input(
                        &graph, &result, &labels, &styles, &options, sketch,
                    )
                },
            )?;
            let mut svg =
                d2_rs_render::render_d2svg(&graph, &result, &labels, &styles, &options, sketch)?;
            if request["appendix"] == true {
                svg = d2_rs_render::d2svg_append_appendix(
                    &graph, &result, &labels, &styles, &options, sketch, &svg,
                )?;
            }
            std::fs::write(output.join("sketch.svg"), svg).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(not(test))]
fn local_asset_read_error(path: &std::path::Path, error: std::io::Error) -> String {
    native::asset_error(path, error)
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let request = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let output = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(output)?;
    native::capture(&request, output).map_err(Into::into)
}
