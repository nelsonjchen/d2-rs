// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typed Object/Edge boundary in d2graph.DeserializeGraph/SerializeGraph.
//! Field metadata is a translation of the pinned source, not fixture output.

use super::{D2Error, Map, Value, decode_go_json_integer};
use std::{collections::BTreeMap, sync::OnceLock};

struct Type {
    kind: String,
    elem: String,
    fields: Vec<Field>,
}
struct Field {
    name: String,
    kind: String,
    omit: bool,
}
fn types() -> &'static BTreeMap<String, Type> {
    static TYPES: OnceLock<BTreeMap<String, Type>> = OnceLock::new();
    TYPES.get_or_init(|| include!("json_record_types.rs").into_iter().collect())
}

enum State {
    Scalar(Value),
    Range(Range),
    Struct(BTreeMap<String, State>),
    Pointer(Option<Box<State>>),
    // A shorter alias does not discard the Go slice's backing elements.
    Slice {
        slots: Vec<State>,
        length: Option<usize>,
    },
}
#[derive(Default)]
struct Range {
    path: String,
    start: [i64; 3],
    end: [i64; 3],
}

fn zero(kind: &str) -> State {
    let ty = &types()[kind];
    match ty.kind.as_str() {
        "struct" => State::Struct(
            ty.fields
                .iter()
                .map(|f| (f.name.clone(), zero(&f.kind)))
                .collect(),
        ),
        "ptr" => State::Pointer(None),
        "slice" => State::Slice {
            slots: Vec::new(),
            length: None,
        },
        "string" => State::Scalar(Value::String(String::new())),
        "bool" => State::Scalar(Value::Bool(false)),
        "int" | "float64" => State::Scalar(Value::from(0)),
        "text" => State::Range(Range::default()),
        _ => unreachable!("uncovered source type {kind}"),
    }
}

pub(super) fn folded(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            '\u{212a}' => 'k',
            '\u{017f}' => 's',
            _ => c.to_ascii_lowercase(),
        })
        .collect()
}

fn type_error(value: &Value, record: &str, path: &str, kind: &str) -> D2Error {
    let description = match value {
        Value::Bool(_) => "bool",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => unreachable!(),
    };
    let last = path.rsplit('.').next().unwrap_or_default();
    let field = if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        ""
    } else {
        "Go struct field "
    };
    D2Error::GoGraphConversion(format!(
        "json: cannot unmarshal {description} into {field}{record}.{path} of type {kind}"
    ))
}

fn read(
    value: &Value,
    state: &mut State,
    kind: &str,
    record: &str,
    path: &str,
    first: &mut Option<D2Error>,
) {
    let ty = &types()[kind];
    let mut remember = |error| {
        if first.is_none() {
            *first = Some(error);
        }
    };
    match state {
        State::Range(target) => {
            if value.is_null() {
                return;
            }
            if let Some(value) = value.as_str() {
                if let Err(error) = range(value, target) {
                    remember(error);
                }
            } else {
                remember(type_error(value, record, path, kind));
            }
        }
        State::Pointer(target) => {
            if value.is_null() {
                *target = None;
            } else {
                read(
                    value,
                    target.get_or_insert_with(|| Box::new(zero(&ty.elem))),
                    &ty.elem,
                    record,
                    path,
                    first,
                );
            }
        }
        State::Slice { slots, length } => {
            if value.is_null() {
                slots.clear();
                *length = None;
                return;
            }
            let Some(values) = value.as_array() else {
                remember(type_error(value, record, path, kind));
                return;
            };
            if values.is_empty() {
                slots.clear();
            }
            while slots.len() < values.len() {
                slots.push(zero(&ty.elem));
            }
            for (index, value) in values.iter().enumerate() {
                read(
                    value,
                    &mut slots[index],
                    &ty.elem,
                    record,
                    &format!("{path}.{index}"),
                    first,
                );
            }
            *length = Some(values.len());
        }
        State::Struct(target) => {
            if value.is_null() {
                return;
            }
            let Some(values) = value.as_object() else {
                remember(type_error(value, record, path, kind));
                return;
            };
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort();
            for key in keys {
                let field = ty
                    .fields
                    .iter()
                    .find(|f| &f.name == key)
                    .or_else(|| ty.fields.iter().find(|f| folded(&f.name) == folded(key)));
                if let Some(field) = field {
                    let next = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    read(
                        &values[key],
                        target.get_mut(&field.name).unwrap(),
                        &field.kind,
                        record,
                        &next,
                        first,
                    );
                }
            }
        }
        State::Scalar(target) => {
            if value.is_null() {
                return;
            }
            match ty.kind.as_str() {
                "string" if value.is_string() => *target = value.clone(),
                "bool" if value.is_boolean() => *target = value.clone(),
                "float64" if value.is_number() => *target = Value::from(value.as_f64().unwrap()),
                "int" => match decode_go_json_integer(Some(value), record, path) {
                    Ok(Some(n)) => *target = Value::from(n),
                    Ok(None) => {}
                    Err(error) => remember(error),
                },
                _ => remember(type_error(value, record, path, kind)),
            }
        }
    }
}

fn empty(state: &State) -> bool {
    match state {
        State::Scalar(v) => match v {
            Value::String(s) => s.is_empty(),
            Value::Bool(b) => !b,
            Value::Number(n) => n.as_f64() == Some(0.0),
            _ => false,
        },
        State::Pointer(v) => v.is_none(),
        State::Slice { length, .. } => length.unwrap_or(0) == 0,
        State::Struct(_) | State::Range(_) => false, // encoding/json v1 never omits struct values.
    }
}
fn write(state: &State, kind: &str) -> Value {
    let ty = &types()[kind];
    match state {
        State::Range(r) => Value::String(format!(
            "{},{}:{}:{}-{}:{}:{}",
            r.path, r.start[0], r.start[1], r.start[2], r.end[0], r.end[1], r.end[2]
        )),
        State::Scalar(v) => v.clone(),
        State::Pointer(v) => v.as_ref().map_or(Value::Null, |v| write(v, &ty.elem)),
        State::Slice { slots, length } => length.map_or(Value::Null, |n| {
            Value::Array(slots[..n].iter().map(|v| write(v, &ty.elem)).collect())
        }),
        State::Struct(values) => Value::Object(
            ty.fields
                .iter()
                .filter_map(|f| {
                    let state = &values[&f.name];
                    (!f.omit || !empty(state)).then(|| (f.name.clone(), write(state, &f.kind)))
                })
                .collect(),
        ),
    }
}

fn range(input: &str, output: &mut Range) -> Result<(), D2Error> {
    use d2_rs_core::quote_go_bytes as quote;
    let fail = |reason: String| {
        D2Error::GoGraphConversion(format!(
            "failed to unmarshal Range from {}: {reason}",
            quote(input.as_bytes())
        ))
    };
    let Some((start, end)) = input.rsplit_once('-') else {
        return Err(fail("missing End field".into()));
    };
    let Some((path, start)) = start.rsplit_once(',') else {
        return Err(fail("missing Start field".into()));
    };
    output.path = path.into();
    let parse = |text: &str, dst: &mut [i64; 3]| -> Result<(), String> {
        let error = |reason| {
            format!(
                "failed to unmarshal Position from {}: {reason}",
                quote(text.as_bytes())
            )
        };
        let fields: Vec<_> = text.split(':').collect();
        if fields.len() != 3 {
            return Err(error("expected three fields".into()));
        }
        for (token, dst) in fields.into_iter().zip(dst) {
            match token.parse::<i64>() {
                Ok(n) => *dst = n,
                Err(e) => {
                    use std::num::IntErrorKind::*;
                    let reason = match e.kind() {
                        PosOverflow => {
                            *dst = i64::MAX;
                            "value out of range"
                        }
                        NegOverflow => {
                            *dst = i64::MIN;
                            "value out of range"
                        }
                        _ => {
                            *dst = 0;
                            "invalid syntax"
                        }
                    };
                    return Err(error(format!(
                        "strconv.Atoi: parsing {}: {reason}",
                        quote(token.as_bytes())
                    )));
                }
            }
        }
        Ok(())
    };
    let result = parse(start, &mut output.start).and_then(|_| parse(end, &mut output.end));
    result.map_err(fail)
}

fn record(value: &Value, kind: &str) -> (Value, Option<D2Error>) {
    let mut state = zero(kind);
    let mut error = None;
    read(
        value,
        &mut state,
        kind,
        kind.rsplit('.').next().unwrap(),
        "",
        &mut error,
    );
    let mut output = write(&state, kind);
    // These keys are populated from raw maps by DeserializeGraph, separately
    // from the typed records. Keep that identity data until serialization.
    let keys: &[&str] = if kind == "d2graph.Object" {
        &["AbsID", "ChildrenArray"]
    } else {
        &["Src", "Dst"]
    };
    for &key in keys {
        if let Some(v) = value.get(key) {
            output
                .as_object_mut()
                .unwrap()
                .insert(key.into(), v.clone());
        }
    }
    (output, error)
}

pub(super) fn prepare(document: &mut Value) -> Result<(), D2Error> {
    if let Some(root) = document.get_mut("root") {
        // serde.go deliberately ignores Convert's root error.
        *root = record(root, "d2graph.Object").0;
    }
    for (key, kind) in [("objects", "d2graph.Object"), ("edges", "d2graph.Edge")] {
        if let Some(records) = document.get_mut(key).and_then(Value::as_array_mut) {
            for value in records {
                // Preserve the existing outer-record error until that raw-map
                // boundary has been decoded, just as for absent inventories.
                if !value.is_object() {
                    continue;
                }
                let (converted, error) = record(value, kind);
                if let Some(error) = error {
                    return Err(error);
                }
                *value = converted;
            }
        }
    }
    Ok(())
}

fn project(value: &Value, kind: &str) -> Value {
    let ty = &types()[kind];
    match ty.kind.as_str() {
        "struct" => Value::Object(
            ty.fields
                .iter()
                .filter_map(|f| {
                    let v = value
                        .get(&f.name)
                        .cloned()
                        .unwrap_or_else(|| write(&zero(&f.kind), &f.kind));
                    let field_ty = &types()[&f.kind];
                    let omitted = f.omit
                        && match field_ty.kind.as_str() {
                            "ptr" => v.is_null(),
                            "slice" => v.is_null() || v.as_array().is_some_and(Vec::is_empty),
                            "string" => v.as_str().is_some_and(str::is_empty),
                            "bool" => v == Value::Bool(false),
                            "int" | "float64" => v.as_f64() == Some(0.0),
                            _ => false,
                        };
                    (!omitted).then(|| (f.name.clone(), project(&v, &f.kind)))
                })
                .collect(),
        ),
        "ptr" if !value.is_null() => project(value, &ty.elem),
        "slice" if value.is_array() => Value::Array(
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| project(v, &ty.elem))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn project_record(value: &mut Value, kind: &str, keys: &[&str]) {
    let mut projected = project(value, kind);
    for &key in keys {
        if let Some(v) = value.get(key) {
            projected
                .as_object_mut()
                .unwrap()
                .insert(key.into(), v.clone());
        }
    }
    *value = projected;
}
pub(super) fn serialize(document: &mut Value) {
    if let Some(root) = document.get_mut("root") {
        project_record(root, "d2graph.Object", &["AbsID", "ChildrenArray"]);
        // SerializeGraph computes AbsID from the object, even when the input
        // map omitted it. The root has no parent, so its AbsID is its typed ID.
        let id = root["id"].clone();
        root.as_object_mut().unwrap().insert("AbsID".into(), id);
    }
    for (key, kind, identity) in [
        ("objects", "d2graph.Object", &["AbsID", "ChildrenArray"][..]),
        ("edges", "d2graph.Edge", &["Src", "Dst"][..]),
    ] {
        if let Some(values) = document.get_mut(key).and_then(Value::as_array_mut) {
            for value in values {
                project_record(value, kind, identity);
            }
        }
    }
    if let Some(root) = document.get_mut("root") {
        omit_empty_children(root);
    }
    if let Some(objects) = document.get_mut("objects").and_then(Value::as_array_mut) {
        for object in objects {
            omit_empty_children(object);
        }
    }
    let Some(graph) = document.as_object_mut() else {
        return;
    };
    graph.retain(|key, _| {
        matches!(
            key.as_str(),
            "root" | "objects" | "edges" | "rootLevel" | "data"
        )
    });
    graph.entry("rootLevel").or_insert(Value::from(0));
    for key in ["objects", "edges"] {
        if graph
            .get(key)
            .is_some_and(|v| v.as_array().is_some_and(Vec::is_empty))
        {
            graph.insert(key.into(), Value::Null);
        }
    }
    if graph
        .get("data")
        .is_some_and(|v| v.is_null() || v.as_object().is_some_and(Map::is_empty))
    {
        graph.remove("data");
    }
}
fn omit_empty_children(value: &mut Value) {
    if value
        .get("ChildrenArray")
        .is_some_and(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
    {
        value.as_object_mut().unwrap().remove("ChildrenArray");
    }
}
