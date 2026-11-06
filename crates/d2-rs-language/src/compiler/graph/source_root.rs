// SPDX-License-Identifier: MPL-2.0
//! Source d2graph.Object for the synthetic board root, before layout.
use super::*;
use serde_json::{Value, json};
fn scalar(value: String) -> Value {
    json!({"value":value})
}

pub(super) fn compile(compiler: &Compiler, root: IrNodeId) -> Result<Value, GraphCompileError> {
    object(compiler, root, true)
}

pub(super) fn object(
    compiler: &Compiler,
    root: IrNodeId,
    synthetic_root: bool,
) -> Result<Value, GraphCompileError> {
    object_at(compiler, root, synthetic_root, &[])
}

// Icon URL parsing is the only fallible part of the source projection. Board
// planning keeps this check at the same point in the lowering traversal even
// though it does not retain the projection's styles, references or JSON.
pub(super) fn icon(
    compiler: &Compiler,
    root: IrNodeId,
    prefix: &[&str],
) -> Result<Option<Value>, GraphCompileError> {
    let path: Vec<_> = prefix.iter().copied().chain(["icon"]).collect();
    let Some(source) = property_scalar_inherited(compiler, root, &path) else {
        return Ok(None);
    };
    let value = scalar_text(&source);
    super::super::import_urls::graph_url_metadata(value.as_bytes())
        .map(Some)
        .map_err(|cause| {
            let node = source.unbox().unwrap();
            GraphCompileError::InvalidAttributes(vec![super::super::edge_creation::error(
                &node,
                format!(
                    "bad icon url {}: {cause}",
                    crate::parser::quote_input(value.as_bytes())
                ),
            )])
        })
}

/// Project attributes at an applied edge submap without copying the IR arena.
pub(super) fn object_at(
    compiler: &Compiler,
    root: IrNodeId,
    synthetic_root: bool,
    prefix: &[&str],
) -> Result<Value, GraphCompileError> {
    let scoped_scalar = |path: &[&str]| {
        let path: Vec<_> = prefix.iter().copied().chain(path.iter().copied()).collect();
        property_scalar_inherited(compiler, root, &path)
    };
    let text = |path: &[&str]| scoped_scalar(path).map(|v| scalar_text(&v));
    let mut object = json!({"id":"","id_val":"","attributes":{
        "label":scalar(text(&["label"]).unwrap_or_default()),"labelDimensions":{"width":0,"height":0},
        "style":{},"iconStyle":{},"near_key":null,
        "shape":scalar(text(&["shape"]).unwrap_or_default().to_lowercase()),
        "direction":scalar(text(&["direction"]).unwrap_or_default().to_lowercase()),"constraint":null},"zIndex":0});
    let attrs = object["attributes"].as_object_mut().unwrap();
    for (source, key) in [
        ("opacity", "opacity"),
        ("stroke", "stroke"),
        ("fill", "fill"),
        ("fill-pattern", "fillPattern"),
        ("stroke-width", "strokeWidth"),
        ("stroke-dash", "strokeDash"),
        ("border-radius", "borderRadius"),
        ("shadow", "shadow"),
        ("3d", "3d"),
        ("multiple", "multiple"),
        ("font", "font"),
        ("font-size", "fontSize"),
        ("font-color", "fontColor"),
        ("animated", "animated"),
        ("bold", "bold"),
        ("italic", "italic"),
        ("underline", "underline"),
        ("filled", "filled"),
        ("double-border", "doubleBorder"),
        ("text-transform", "textTransform"),
    ] {
        if let Some(mut value) = text(&["style", source]) {
            if source == "font" {
                value = String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap();
            }
            attrs.get_mut("style").unwrap()[key] = scalar(value);
        }
        if text(&["icon"]).is_some() {
            if let Some(mut value) = text(&["icon", "style", source]) {
                if source == "font" {
                    value =
                        String::from_utf8(d2_rs_core::ir::go_lowercase(value.as_bytes())).unwrap();
                }
                attrs.get_mut("iconStyle").unwrap()[key] = scalar(value);
            }
        }
    }
    for (source, key) in [
        ("tooltip", "tooltip"),
        ("link", "link"),
        ("width", "width"),
        ("height", "height"),
        ("top", "top"),
        ("left", "left"),
        ("grid-rows", "gridRows"),
        ("grid-columns", "gridColumns"),
        ("grid-gap", "gridGap"),
        ("vertical-gap", "verticalGap"),
        ("horizontal-gap", "horizontalGap"),
    ] {
        if let Some(value) = text(&[source]) {
            attrs.insert(key.into(), scalar(value));
        }
    }
    for (source, key) in [
        ("label", "labelPosition"),
        ("icon", "iconPosition"),
        ("tooltip", "tooltipPosition"),
    ] {
        if let Some(value) = text(&[source, "near"]) {
            attrs.insert(key.into(), scalar(value));
        }
    }
    if let Some(value) = scoped_scalar(&["near"]) {
        if let Ok(mut key) = crate::parse_key(&value.scalar_bytes()) {
            key.range = value.unbox().unwrap().range();
            attrs.insert("near_key".into(), serde_json::to_value(key).unwrap());
        }
    }
    if let Some(url) = icon(compiler, root, prefix)? {
        attrs.insert("icon".into(), url);
    }
    let language = if prefix.is_empty() {
        field_language(compiler, root)
    } else {
        scoped_scalar(&[]).and_then(|v| v.block_string.map(|v| normalize_language(&v.read().tag)))
    };
    if let Some(language) = language.filter(|v| !v.is_empty()) {
        attrs.insert("language".into(), json!(language));
    }
    let path: Vec<_> = prefix.iter().copied().chain(["constraint"]).collect();
    let constraints = field_values(compiler, root, &path);
    if !constraints.is_empty() {
        attrs.insert("constraint".into(), json!(constraints));
    }
    let classes = if prefix.is_empty() {
        target_class_names(compiler, root)
    } else {
        Vec::new()
    };
    if !classes.is_empty() {
        attrs.insert("classes".into(), json!(classes));
    }
    // compileBoardsField applies a board's primary label after compileBoard.
    if let Some(field) = compiler
        .arena
        .parent_field(root)
        .filter(|_| synthetic_root && prefix.is_empty())
    {
        if let Some(primary) = compiler.arena.primary(field) {
            if let IrNode::Scalar(value) = compiler.arena.node(primary) {
                attrs.insert("label".into(), scalar(scalar_text(&value.value)));
                if let Some(block) = &value.value.block_string {
                    attrs.insert(
                        "language".into(),
                        json!(normalize_language(&block.read().tag)),
                    );
                }
            }
        }
    }
    match object["attributes"]["shape"]["value"]
        .as_str()
        .unwrap_or("")
    {
        "class" => {
            let members = |methods| {
                let rows = if methods {
                    class_method_rows(compiler, root)
                } else {
                    class_field_rows(compiler, root)
                };
                let underlines = class_member_underlines(compiler, root, methods);
                let rows=rows.into_iter().enumerate().map(|(i,(raw,value))| {
                    let (name,visibility)=match raw.as_bytes().first() {
                        Some(b'+')=>(&raw[1..],"public"),Some(b'-')=>(&raw[1..],"private"),
                        Some(b'#')=>(&raw[1..],"protected"),_ =>(raw.as_str(),"public")};
                    let mut row=json!({"name":name,"visibility":visibility,"underline":underlines.get(i).copied().unwrap_or(false)});
                    row[if methods {"return"}else{"type"}]=json!(value);row
                }).collect::<Vec<_>>();
                if rows.is_empty() {
                    Value::Null
                } else {
                    json!(rows)
                }
            };
            object["class"] = json!({"fields":members(false),"methods":members(true)});
        }
        "sql_table" => {
            let mut columns = Vec::new();
            if let Some(map) = compiler.arena.map(root) {
                if let IrNode::Map(map) = compiler.arena.node(map) {
                    for &column in map.fields.as_deref().unwrap_or_default() {
                        if is_metadata_field(compiler, column) {
                            continue;
                        }
                        let Some(name) = field_name(compiler, column) else {
                            continue;
                        };
                        let value = property_scalar_inherited(compiler, column, &["label"])
                            .or_else(|| {
                                compiler.arena.primary(column).and_then(|id| {
                                    match compiler.arena.node(id) {
                                        IrNode::Scalar(s) => Some(s.value.clone()),
                                        _ => None,
                                    }
                                })
                            })
                            .map(|v| scalar_text(&v))
                            .unwrap_or_else(|| name.clone());
                        let value = if value == name { String::new() } else { value };
                        let constraints = field_values(compiler, column, &["constraint"]);
                        columns.push(SourceSqlColumn {
                            name: SourceSqlText {
                                label: name,
                                ..Default::default()
                            },
                            type_text: SourceSqlText {
                                label: value,
                                ..Default::default()
                            },
                            constraint: (!constraints.is_empty()).then_some(constraints),
                            reference: String::new(),
                        });
                    }
                }
            }
            object["sql_table"] =
                json!({"columns":if columns.is_empty(){Value::Null}else{json!(columns)}});
        }
        _ => {}
    }
    Ok(object)
}
