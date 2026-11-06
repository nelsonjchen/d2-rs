// SPDX-License-Identifier: MPL-2.0
//! Native compiler boundary for the master inline-test scoreboard.
use d2_rs_core::ast::Shared;
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};

fn errors(errors: &[d2_rs_core::ast::Error]) -> Value {
    json!({"error":errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"), "graph":null, "config":null})
}
pub(crate) fn evaluate(r: Value) -> Value {
    if r["has_context"] == true {
        return json!({"unsupported":"context"});
    }
    let parsed = parse(
        r["path"].as_str().unwrap(),
        r["input"].as_str().unwrap().as_bytes(),
        None,
    );
    if let Some(err) = parsed.error {
        return errors(err.errors.as_deref().unwrap_or_default());
    }
    let ast = Shared::new(parsed.map);
    let opener = if r["has_fs"] == true {
        let files = r["files"].clone();
        Some(Box::new(move |path: &str| {
            files[path]
                .as_str()
                .map(|text| {
                    Box::new(std::io::Cursor::new(text.as_bytes().to_vec()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        }) as compiler::ImportOpener)
    } else {
        None
    };
    let (c, root) = match compiler::compile_ir_with_limits(
        ast.clone(),
        compiler::CompileOptions {
            import_opener: opener,
            ..Default::default()
        },
        compiler::CompileLimits {
            max_glob_expansion: r["max_glob_expansion"].as_i64().unwrap_or_default(),
            max_variable_expansion: r["max_variable_expansion"].as_i64().unwrap_or_default(),
            max_edge_expansion: r["max_edge_expansion"].as_i64().unwrap_or_default(),
            max_edge_expansion_work: r["max_edge_expansion_work"].as_i64().unwrap_or_default(),
        },
    ) {
        Ok(value) => value,
        Err(err) => return json!({"unsupported":err.to_string()}),
    };
    if !c.errors.is_empty() {
        return errors(&c.errors);
    }
    let graph = match compiler::source_graph_json(&c, root, ast) {
        Ok(value) => value,
        Err(compiler::GraphCompileError::InvalidAttributes(err)) => return errors(&err),
        Err(err) => return json!({"unsupported":err.to_string()}),
    };
    match compiler::extract_config_checked(&c, root) {
        Ok(None) => json!({"error":"","graph":graph,"config":null}),
        Ok(Some(config)) => {
            let mut config_json = json!({"sketch":config.sketch,"themeID":config.theme_id,"darkThemeID":config.dark_theme_id,"pad":config.pad,
                "center":config.center,"layoutEngine":config.layout_engine});
            if let Some(isometric) = config.isometric {
                config_json["isometric"] = json!(isometric);
            }
            if !config.data.is_empty() {
                config_json["data"] = json!(config.data);
            }
            json!({"error":"","graph":graph,"config":config_json})
        }
        Err(err) => errors(&err),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let r: Value = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    println!("{}", evaluate(r));
    Ok(())
}
