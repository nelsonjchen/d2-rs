// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The initial typed SerializedGraph decode in d2graph/serde.go.
//! Raw values retain duplicate fields, input order and numeric spelling.

use super::{D2Error, Map, Value};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::fmt;

type Raw = Box<RawValue>;
struct Members(Vec<(String, Raw)>);
impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Members;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

fn kind(raw: &RawValue) -> u8 {
    raw.get().as_bytes()[0]
}
fn type_error(raw: &RawValue, path: &str, go_type: &str, numeric_token: bool) -> D2Error {
    let description = match kind(raw) {
        b't' | b'f' => "bool".into(),
        b'"' => "string".into(),
        b'[' => "array".into(),
        b'{' => "object".into(),
        _ if numeric_token => format!("number {}", raw.get()),
        _ => "number".into(),
    };
    let target = if path.is_empty() {
        "Go value".into()
    } else {
        let last = path.rsplit('.').next().unwrap();
        let field = if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
            ""
        } else {
            "Go struct field "
        };
        format!("{field}.{path}")
    };
    D2Error::GoGraphConversion(format!(
        "json: cannot unmarshal {description} into {target} of type {go_type}"
    ))
}
fn remember(first: &mut Option<D2Error>, error: D2Error) {
    if first.is_none() {
        *first = Some(error);
    }
}
fn nested(path: &str, key: &str) -> String {
    // Go's legacy error wrapper changes JSON-pointer separators into dots,
    // while retaining the pointer escapes within member names.
    let key = key.replace('~', "~0").replace('/', "~1");
    if path.is_empty() {
        key
    } else {
        format!("{path}.{key}")
    }
}

// An interface-valued map entry always decodes into a fresh value. In
// particular, a later object value replaces a prior object at that entry.
fn any(raw: &RawValue, path: &str, first: &mut Option<D2Error>) -> Result<Value, D2Error> {
    Ok(match kind(raw) {
        b'n' => Value::Null,
        b't' | b'f' | b'"' => serde_json::from_str(raw.get())?,
        b'[' => {
            let values: Vec<Raw> = serde_json::from_str(raw.get())?;
            Value::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(i, raw)| any(raw, &nested(path, &i.to_string()), first))
                    .collect::<Result<_, _>>()?,
            )
        }
        b'{' => {
            let mut map = Value::Null;
            read_map(raw, &mut map, path, "map[string]interface {}", first)?;
            map
        }
        _ => {
            match raw
                .get()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
            {
                Some(value) => Value::Number(serde_json::Number::from_f64(value).unwrap()),
                None => {
                    remember(first, type_error(raw, path, "float64", true));
                    Value::Null
                }
            }
        }
    })
}
fn read_map(
    raw: &RawValue,
    target: &mut Value,
    path: &str,
    go_type: &str,
    first: &mut Option<D2Error>,
) -> Result<(), D2Error> {
    match kind(raw) {
        b'n' => *target = Value::Null,
        b'{' => {
            if target.is_null() {
                *target = Value::Object(Map::new());
            }
            let entries: Members = serde_json::from_str(raw.get())?;
            let map = target.as_object_mut().expect("typed map state");
            for (key, value) in entries.0 {
                let value = any(&value, &nested(path, &key), first)?;
                map.insert(key, value);
            }
        }
        _ => remember(first, type_error(raw, path, go_type, false)),
    }
    Ok(())
}

#[derive(Default)]
struct Slice {
    slots: Vec<Value>,
    length: Option<usize>,
}
impl Slice {
    fn read(
        &mut self,
        raw: &RawValue,
        path: &str,
        elem: &str,
        first: &mut Option<D2Error>,
    ) -> Result<(), D2Error> {
        match kind(raw) {
            b'n' => {
                self.slots.clear();
                self.length = None;
            }
            b'[' => {
                let values: Vec<Raw> = serde_json::from_str(raw.get())?;
                if values.is_empty() {
                    self.slots.clear();
                }
                if self.slots.len() < values.len() {
                    self.slots.resize(values.len(), Value::Null);
                }
                for (i, raw) in values.iter().enumerate() {
                    read_map(
                        raw,
                        &mut self.slots[i],
                        &nested(path, &i.to_string()),
                        elem,
                        first,
                    )?;
                }
                self.length = Some(values.len());
            }
            _ => remember(first, type_error(raw, path, &format!("[]{elem}"), false)),
        }
        Ok(())
    }
    fn value(mut self) -> Value {
        match self.length {
            None => Value::Null,
            Some(length) => {
                self.slots.truncate(length);
                Value::Array(self.slots)
            }
        }
    }
}

pub(super) fn decode(input: &[u8]) -> Result<Value, D2Error> {
    // Validate the complete JSON document before typed conversion, as Unmarshal
    // does. RawValue avoids prematurely rejecting large unknown numeric fields.
    let input = super::json_syntax::prepare(input)?;
    let raw: Raw = serde_json::from_slice(&input)?;
    if kind(&raw) != b'{' {
        // A nil *SerializedGraph panics in the Go caller; retain a safe native
        // error for that unsupported raw boundary instead of inventing a graph.
        if kind(&raw) == b'n' {
            return Err(D2Error::RootType);
        }
        return Err(type_error(&raw, "", "d2graph.SerializedGraph", false));
    }
    let members: Members = serde_json::from_str(raw.get())?;
    let (mut root, mut data) = (Value::Null, Value::Null);
    let (mut objects, mut edges) = (Slice::default(), Slice::default());
    let mut root_level = 0i64;
    let mut first = None;
    for (name, value) in members.0 {
        match super::json_records::folded(&name).as_str() {
            "root" => read_map(
                &value,
                &mut root,
                &name,
                "d2graph.SerializedObject",
                &mut first,
            )?,
            "objects" => objects.read(&value, &name, "d2graph.SerializedObject", &mut first)?,
            "edges" => edges.read(&value, &name, "d2graph.SerializedEdge", &mut first)?,
            "data" => read_map(
                &value,
                &mut data,
                &name,
                "map[string]interface {}",
                &mut first,
            )?,
            "rootlevel" if kind(&value) != b'n' => {
                if let Ok(number) = value.get().parse::<i64>() {
                    root_level = number;
                } else {
                    remember(
                        &mut first,
                        type_error(
                            &value,
                            &name,
                            "int",
                            matches!(kind(&value), b'-' | b'0'..=b'9'),
                        ),
                    );
                }
            }
            _ => (),
        }
    }
    if let Some(error) = first {
        return Err(error);
    }
    Ok(
        serde_json::json!({"root":root,"objects":objects.value(),"edges":edges.value(),"rootLevel":root_level,"data":data}),
    )
}
