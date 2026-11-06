// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Geometry fields at DeserializeGraph's map-to-typed-struct boundary.
//! Go remarshal sorts map keys; case variants then update the same field.

use super::{
    D2Error, Map, Point, Size, Value, decode_go_json_integer, decode_validation_attributes,
};
use d2_rs_core::D2InputBox;
use std::collections::BTreeSet;

fn fields(object: &Map<String, Value>) -> Vec<(&str, &Value)> {
    let mut fields: Vec<_> = object
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    fields.sort_unstable_by_key(|(key, _)| *key);
    fields
}

fn type_error(value: &Value, record: &str, path: &str, expected: &str) -> D2Error {
    let kind = match value {
        Value::Bool(_) => "bool",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => unreachable!(),
    };
    // Go 1.27's encoding/json v1 wrapper reports the original JSON pointer,
    // including input spelling and array indexes. A terminal array index omits
    // the "Go struct field" phrase (v2_decode.go:132-155).
    let last = path.rsplit('.').next().unwrap_or_default();
    let field = if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        ""
    } else {
        "Go struct field "
    };
    D2Error::GoGraphConversion(format!(
        "json: cannot unmarshal {kind} into {field}{record}.{path} of type {expected}"
    ))
}

fn float(value: &Value, target: &mut f64, record: &str, path: &str) -> Result<(), D2Error> {
    if !value.is_null() {
        *target = value
            .as_f64()
            .ok_or_else(|| type_error(value, record, path, "float64"))?;
    }
    Ok(())
}

fn point(
    value: &Value,
    target: &mut Option<Point>,
    record: &str,
    path: &str,
) -> Result<(), D2Error> {
    if value.is_null() {
        *target = None;
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| type_error(value, record, path, "geo.Point"))?;
    let target = target.get_or_insert_with(Point::default);
    for (key, value) in fields(object) {
        if key.eq_ignore_ascii_case("x") {
            float(value, &mut target.x, record, &format!("{path}.{key}"))?;
        } else if key.eq_ignore_ascii_case("y") {
            float(value, &mut target.y, record, &format!("{path}.{key}"))?;
        }
    }
    Ok(())
}

fn box_field(value: &Value, target: &mut Option<D2InputBox>, path: &str) -> Result<(), D2Error> {
    if value.is_null() {
        *target = None;
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| type_error(value, "Object", path, "geo.Box"))?;
    let target = target.get_or_insert(D2InputBox {
        size: Size {
            width: 0.0,
            height: 0.0,
        },
        top_left: None,
    });
    for (key, value) in fields(object) {
        if key.eq_ignore_ascii_case("Width") {
            float(
                value,
                &mut target.size.width,
                "Object",
                &format!("{path}.{key}"),
            )?;
        } else if key.eq_ignore_ascii_case("Height") {
            float(
                value,
                &mut target.size.height,
                "Object",
                &format!("{path}.{key}"),
            )?;
        } else if key.eq_ignore_ascii_case("TopLeft") {
            point(
                value,
                &mut target.top_left,
                "Object",
                &format!("{path}.{key}"),
            )?;
        }
    }
    Ok(())
}

pub(super) fn object_box(object: &Map<String, Value>) -> Result<Option<D2InputBox>, D2Error> {
    let mut result = None;
    for (key, value) in fields(object) {
        if key.eq_ignore_ascii_case("box") {
            box_field(value, &mut result, key)?;
        } else if key == "attributes" {
            // Existing typed integer conversions participate in remarshal key
            // order, before the adapter's separate semantic validation stage.
            decode_validation_attributes(value.as_object(), "Object", "attributes")?;
        }
    }
    Ok(result)
}

pub(super) struct InputRoute {
    pub points: Vec<Point>,
    pub nil_points: BTreeSet<usize>,
}

pub(super) fn edge_route(edge: &Map<String, Value>) -> Result<InputRoute, D2Error> {
    // Go reuses slice backing storage. Keep truncated slots until a null or
    // empty array resets the slice, so subsequent case variants can regrow it.
    let mut slots = Vec::<Option<Point>>::new();
    let mut length = 0;
    for (key, value) in fields(edge) {
        if key.eq_ignore_ascii_case("route") {
            if value.is_null() {
                slots.clear();
                length = 0;
                continue;
            }
            let values = value
                .as_array()
                .ok_or_else(|| type_error(value, "Edge", key, "[]*geo.Point"))?;
            if values.is_empty() {
                slots.clear();
                length = 0;
                continue;
            }
            if values.len() > slots.len() {
                slots.resize(values.len(), None);
            }
            for (index, value) in values.iter().enumerate() {
                point(value, &mut slots[index], "Edge", &format!("{key}.{index}"))?;
            }
            length = values.len();
        } else if matches!(key, "attributes" | "dstArrowhead" | "srcArrowhead") {
            decode_validation_attributes(value.as_object(), "Edge", key)?;
        } else if matches!(key, "dstTableColumnIndex" | "srcTableColumnIndex") {
            decode_go_json_integer(Some(value), "Edge", key)?;
        }
    }
    slots.truncate(length);
    let nil_points = slots
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.is_none().then_some(i))
        .collect();
    Ok(InputRoute {
        points: slots.into_iter().map(Option::unwrap_or_default).collect(),
        nil_points,
    })
}
