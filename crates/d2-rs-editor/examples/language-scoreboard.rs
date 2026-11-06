// SPDX-License-Identifier: MPL-2.0
//! Native golden runner. No Go/JS code is called by this executable.
use d2_rs_core::ast;
use d2_rs_language::{ParseOptions, compiler, parse};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
struct Request {
    package: String,
    input: String,
    input_hex: Option<String>,
    path: String,
    #[serde(default)]
    files: Option<HashMap<String, String>>,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    new_key: String,
    #[serde(default)]
    board_path: Option<Vec<String>>,
    #[serde(default)]
    include_descendants: bool,
    value: Option<String>,
    tag: Option<String>,
    #[serde(default)]
    src: String,
    #[serde(default)]
    dst: String,
    #[serde(default)]
    old_path: String,
    new_path: Option<String>,
}

#[path = "../../d2-rs-language/examples/support/ir_json.rs"]
mod ir_projection;
use ir_projection::ir_json;

fn error_value(errors: &[ast::Error]) -> Value {
    json!({"value":{"graph":null,"err":{"errs":errors}},
        "error":errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")})
}

fn evaluate(request: Request) -> Value {
    if request.package == "d2oracle" {
        return edit(request);
    }
    let bytes = request.input_hex.as_ref().map(|hex| {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("input hex"))
            .collect::<Vec<_>>()
    });
    let parsed = parse(
        &request.path,
        bytes.as_deref().unwrap_or(request.input.as_bytes()),
        Some(ParseOptions::default()),
    );
    if request.package == "d2parser" {
        return json!({"value":{"ast":parsed.map,"err":parsed.error}});
    }
    if let Some(error) = parsed.error {
        return error_value(error.errors.as_deref().unwrap_or_default());
    }
    let ast = ast::Shared::new(parsed.map);
    let files = request.files.unwrap_or_default();
    let options = compiler::CompileOptions {
        import_opener: Some(Box::new(move |path| {
            files
                .get(path)
                .map(|text| {
                    Box::new(std::io::Cursor::new(text.clone().into_bytes()))
                        as Box<dyn d2_rs_language::stream::ByteReader>
                })
                .ok_or_else(|| format!("open {path}: file does not exist"))
        })),
        ..Default::default()
    };
    let (compiler, root) = match compiler::compile_ir_with_options(ast.clone(), options) {
        Ok(value) => value,
        Err(error) => return json!({"unsupported":error.to_string()}),
    };
    if !compiler.errors.is_empty() {
        return error_value(&compiler.errors);
    }
    if request.package == "d2ir" {
        return json!({"value":ir_json(&compiler.arena,root)});
    }
    match compiler::source_graph_json(&compiler, root, ast) {
        Ok(value) => match compiler::extract_config_checked(&compiler, root) {
            Ok(_) => json!({"value":{"graph":value,"err":null}}),
            Err(errors) => error_value(&errors),
        },
        Err(compiler::GraphCompileError::InvalidAttributes(errors)) => error_value(&errors),
        Err(error) => json!({"unsupported":error.to_string()}),
    }
}

fn edit(request: Request) -> Value {
    if request.operation == "update_import" {
        return match d2_rs_editor::update_import(
            &request.input,
            &request.old_path,
            request.new_path.as_deref(),
        ) {
            Ok(text) => json!({"text":text}),
            Err(error) => json!({"error":error.to_string()}),
        };
    }
    let result = match request.operation.as_str() {
        "create" => d2_rs_editor::create_object_with_options(
            &request.input,
            &request.key,
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        "set" => d2_rs_editor::set_object_with_options(
            &request.input,
            &request.key,
            request.tag.as_deref(),
            request.value.as_deref(),
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        "delete" => d2_rs_editor::delete_object_with_options(
            &request.input,
            &request.key,
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        "rename" => d2_rs_editor::rename_object_with_options(
            &request.input,
            &request.key,
            &request.new_key,
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        "move" => d2_rs_editor::move_object_with_options(
            &request.input,
            &request.key,
            &request.new_key,
            request.include_descendants,
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        "reconnect" => d2_rs_editor::reconnect_edge_with_options(
            &request.input,
            &request.key,
            (!request.src.is_empty()).then_some(request.src.as_str()),
            (!request.dst.is_empty()).then_some(request.dst.as_str()),
            &d2_rs_editor::MoveOptions {
                path: request.path,
                files: request.files.unwrap_or_default(),
                board_path: request.board_path.unwrap_or_default(),
            },
        )
        .map(|v| (v.source, v.graph)),
        _ => return json!({"unsupported":"native editor operation is not implemented"}),
    };
    match result {
        Ok((text, graph)) => json!({"text":text,"value":{"graph":graph,"err":"<nil>"},"error":""}),
        Err(error) => {
            // edit_test.go serializes fmt.Sprintf("%#v", err). Wrapped edit
            // errors implement GoString; bare errors and OutsideScope retain
            // their concrete representation. The text tier uses Error().
            let debug = match &error {
                d2_rs_editor::GraphEditError::Reconnect(message) => format!(
                    "&errors.errorString{{s:{}}}",
                    d2_rs_core::quote_go_bytes(message.as_bytes())
                ),
                d2_rs_editor::GraphEditError::OutsideScope => "d2oracle.OutsideScopeError{}".into(),
                _ => error.to_string(),
            };
            json!({"error":error.to_string(),"value":{"graph":null,"err":debug}})
        }
    }
}

fn main() {
    let request: Request = serde_json::from_reader(std::io::stdin()).expect("scoreboard request");
    let output = evaluate(request);
    serde_json::to_writer(std::io::stdout(), &output).expect("scoreboard response");
}
