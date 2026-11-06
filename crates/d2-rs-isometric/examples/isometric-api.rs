// SPDX-License-Identifier: MPL-2.0
//! Public image API replay; complete results are compared against Go test calls.
use base64::Engine;
use d2_rs_isometric::img::{page, render};
use serde_json::{Value, json};
use std::io::BufRead;
#[path = "support/api_options.rs"]
mod api_options;

fn replay(r: &Value) -> Value {
    let diagram = r
        .get("diagram")
        .filter(|d| !d.is_null())
        .map(|d| serde_json::from_value(d.clone()).unwrap());
    let o = &r["options"];
    let options = render::Options {
        width: o["Width"].as_i64().unwrap_or(0),
        height: o["Height"].as_i64().unwrap_or(0),
        fit_content: o["FitContent"].as_bool().unwrap_or(false),
        format: o["Format"].as_str().unwrap_or("").into(),
        timeout_ns: o["Timeout"].as_i64().unwrap_or(0),
        assets: api_options::assets(o),
        link_budget: o.get("LinkBudget").filter(|b| !b.is_null()).map(|b| {
            d2_rs_scenebuild::LinkBudget {
                max_regions: b["MaxRegions"].as_i64().unwrap(),
                max_string_bytes: b["MaxStringBytes"].as_i64().unwrap(),
            }
        }),
        render: d2_rs_isometric::scene::RenderOpts {
            theme_id: o["Render"]["ThemeID"].as_i64(),
            theme_overrides: o["Render"]
                .get("ThemeOverrides")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).unwrap()),
        },
        ..Default::default()
    };
    let token = d2_rs_core::CancellationToken::default();
    match r["context"].as_str().unwrap_or("") {
        "context canceled" => token.cancel(),
        "context deadline exceeded" => token.expire(),
        "" => (),
        other => panic!("unsupported context {other}"),
    }
    if r["api"] == "page" {
        return match page::render_page_with_context(Some(&token), diagram.as_ref(), Some(&options))
        {
            Ok(p) => {
                json!({"result":{"PNG":base64::engine::general_purpose::STANDARD.encode(p.png),"Links":p.links,"Width":p.width,"Height":p.height},"error":""})
            }
            Err(e) => json!({"result":null,"error":e.to_string()}),
        };
    }
    let result = render::normalize(Some(&options)).and_then(|options| {
        if let Some(error) = token.error() {
            return Err(error.to_string());
        }
        match options.format.as_str() {
            "svg" => {
                render::render_svg_with_context(Some(&token), diagram.as_ref(), Some(&options))
                    .map(String::into_bytes)
                    .map_err(|e| e.to_string())
            }
            "png" => {
                render::render_png_with_context(Some(&token), diagram.as_ref(), Some(&options))
                    .map_err(|e| e.to_string())
            }
            _ => Err("unsupported native GIF".into()),
        }
    });
    match result {
        Ok(p) => json!({"result":base64::engine::general_purpose::STANDARD.encode(p),"error":""}),
        Err(e) => json!({"result":null,"error":e}),
    }
}
fn main() {
    for line in std::io::stdin().lock().lines() {
        let r: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", replay(&r));
    }
}
