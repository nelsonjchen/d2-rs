// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Typed, ordered updates and v1-compatible errors from Go 1.27 encoding/json/v2.
//! Syntax diagnostics and invalid-Unicode handling remain pending.
//! Go error formatting: Copyright 2010, 2024 The Go Authors.
//! BSD terms are preserved in THIRD_PARTY_NOTICES.md.
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Bool,
    Int,
    NativeInt,
    Float,
    String,
    Struct(&'static str),
    List(&'static Kind),
    Map(&'static Kind),
    Pointer(&'static Kind),
}

// A serde_json::Value map would discard duplicate keys before typed updates.
enum Raw {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Raw>),
    Object(Vec<(String, Raw)>),
}
impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // RawValue retains exponent spelling and numbers outside f64/i64 range.
        // Conversion belongs to the destination field, not the JSON scanner.
        let raw = Box::<serde_json::value::RawValue>::deserialize(deserializer)?;
        let text = raw.get();
        match text.as_bytes()[0] {
            b'n' => Ok(Self::Null),
            b't' => Ok(Self::Bool(true)),
            b'f' => Ok(Self::Bool(false)),
            b'"' => serde_json::from_str(text)
                .map(Self::String)
                .map_err(de::Error::custom),
            b'[' => serde_json::from_str(text)
                .map(Self::Array)
                .map_err(de::Error::custom),
            b'{' => {
                struct Entries(Vec<(String, Raw)>);
                impl<'de> Deserialize<'de> for Entries {
                    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                        struct EntriesVisitor;
                        impl<'de> Visitor<'de> for EntriesVisitor {
                            type Value = Entries;
                            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                                f.write_str("JSON object")
                            }
                            fn visit_map<A: MapAccess<'de>>(
                                self,
                                mut map: A,
                            ) -> Result<Entries, A::Error> {
                                let mut entries = Vec::new();
                                while let Some(entry) = map.next_entry()? {
                                    entries.push(entry);
                                }
                                Ok(Entries(entries))
                            }
                        }
                        d.deserialize_map(EntriesVisitor)
                    }
                }
                serde_json::from_str::<Entries>(text)
                    .map(|v| Self::Object(v.0))
                    .map_err(de::Error::custom)
            }
            _ => Ok(Self::Number(text.into())),
        }
    }
}

enum State {
    Null,
    Scalar(Value),
    Object(BTreeMap<String, State>),
    // Preserve decoded elements past the current length, as Go slices do when
    // a repeated array field shrinks and then expands into its existing storage.
    Array { storage: Vec<State>, len: usize },
}
impl State {
    fn zero(kind: Kind) -> Self {
        match kind {
            Kind::Bool => Self::Scalar(Value::Bool(false)),
            Kind::Int | Kind::NativeInt | Kind::Float => Self::Scalar(Value::Number(0.into())),
            Kind::String => Self::Scalar(Value::String(String::new())),
            Kind::Struct(_) => Self::Object(BTreeMap::new()),
            _ => Self::Null,
        }
    }
    fn value(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Scalar(v) => v,
            Self::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, v.value()))
                    .collect::<Map<_, _>>(),
            ),
            Self::Array { storage, len } => {
                Value::Array(storage.into_iter().take(len).map(Self::value).collect())
            }
        }
    }
}

fn fold(name: &str) -> String {
    // All declared graphjson fields are ASCII. These are the two non-ASCII
    // SimpleFold equivalents of ASCII letters accepted by Go's foldName.
    name.chars()
        .map(|c| match c {
            '\u{212a}' => 'K',
            '\u{017f}' => 'S',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}
fn error(message: impl fmt::Display) -> serde_json::Error {
    de::Error::custom(message)
}
fn go_type(kind: Kind) -> String {
    match kind {
        Kind::Bool => "bool".into(),
        Kind::Int => "int64".into(),
        Kind::NativeInt => "int".into(),
        Kind::Float => "float64".into(),
        Kind::String => "string".into(),
        Kind::Struct(name) => format!("graphjson.{name}"),
        Kind::List(inner) => format!("[]{}", go_type(*inner)),
        Kind::Map(inner) => format!("map[int64]{}", go_type(*inner)),
        Kind::Pointer(inner) => format!("*{}", go_type(*inner)),
    }
}
fn type_error(kind: Kind, raw: &Raw, path: &[String]) -> serde_json::Error {
    let value = match raw {
        Raw::Null => "null".into(),
        Raw::Bool(_) => "bool".into(),
        Raw::String(_) => "string".into(),
        Raw::Array(_) => "array".into(),
        Raw::Object(_) => "object".into(),
        Raw::Number(n) if matches!(kind, Kind::Int | Kind::NativeInt | Kind::Float) => {
            format!("number {n}")
        }
        Raw::Number(_) => "number".into(),
    };
    let target = if path.is_empty() {
        "Go value".into()
    } else {
        let field = path
            .iter()
            .map(|s| s.replace('~', "~0").replace('/', "~1"))
            .collect::<Vec<_>>()
            .join(".");
        let last = field.rsplit('.').next().unwrap();
        let prefix = if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
            ""
        } else {
            "Go struct field "
        };
        format!("{prefix}SerializedGraph.{field}")
    };
    error(format!(
        "json: cannot unmarshal {value} into {target} of type {}",
        go_type(kind)
    ))
}
fn apply(
    kind: Kind,
    state: &mut State,
    raw: Raw,
    path: &mut Vec<String>,
) -> Result<(), serde_json::Error> {
    if matches!(raw, Raw::Null) {
        if matches!(kind, Kind::Pointer(_) | Kind::Map(_) | Kind::List(_)) {
            *state = State::Null;
        }
        return Ok(());
    }
    match (kind, raw) {
        (Kind::Pointer(inner), raw) => {
            if matches!(state, State::Null) {
                *state = State::zero(*inner);
            }
            apply(*inner, state, raw, path)?;
        }
        (Kind::Struct(name), Raw::Object(entries)) => {
            let State::Object(fields) = state else {
                unreachable!()
            };
            let schema = super::schema::fields(name);
            for (key, raw) in entries {
                let field = schema
                    .iter()
                    .find(|(name, _)| *name == key)
                    .or_else(|| schema.iter().find(|(name, _)| fold(name) == fold(&key)));
                let Some(&(name, kind)) = field else {
                    return Err(error(format!("json: unknown field {key:?}")));
                };
                path.push(key);
                apply(
                    kind,
                    fields
                        .entry(name.into())
                        .or_insert_with(|| State::zero(kind)),
                    raw,
                    path,
                )?;
                path.pop();
            }
        }
        (Kind::Map(inner), Raw::Object(entries)) => {
            if matches!(state, State::Null) {
                *state = State::Object(BTreeMap::new());
            }
            let State::Object(fields) = state else {
                unreachable!()
            };
            for (key, raw) in entries {
                path.push(key.clone());
                let key = key
                    .parse::<i64>()
                    .map_err(|_| type_error(Kind::Int, &Raw::Number(key), path))?
                    .to_string();
                // encoding/json zeroes a scratch map element before each key;
                // unlike a struct field, an existing map value is not reused.
                let mut value = State::zero(*inner);
                apply(*inner, &mut value, raw, path)?;
                path.pop();
                fields.insert(key, value);
            }
        }
        (Kind::List(inner), Raw::Array(values)) => {
            if matches!(state, State::Null) || values.is_empty() {
                *state = State::Array {
                    storage: Vec::new(),
                    len: 0,
                };
            }
            let State::Array { storage, len } = state else {
                unreachable!()
            };
            *len = values.len();
            for (i, raw) in values.into_iter().enumerate() {
                if i == storage.len() {
                    storage.push(State::zero(*inner));
                }
                path.push(i.to_string());
                apply(*inner, &mut storage[i], raw, path)?;
                path.pop();
            }
        }
        (Kind::Bool, Raw::Bool(v)) => *state = State::Scalar(Value::Bool(v)),
        (Kind::String, Raw::String(v)) => *state = State::Scalar(Value::String(v)),
        (kind @ (Kind::Int | Kind::NativeInt), Raw::Number(v)) => {
            let number = v
                .parse::<i64>()
                .map_err(|_| type_error(kind, &Raw::Number(v), path))?;
            *state = State::Scalar(Value::Number(number.into()));
        }
        (Kind::Float, Raw::Number(v)) => {
            // Use serde's correctly rounded conversion, not a second parser.
            let number = serde_json::from_str::<f64>(&v)
                .ok()
                .and_then(Number::from_f64)
                .ok_or_else(|| type_error(Kind::Float, &Raw::Number(v), path))?;
            *state = State::Scalar(Value::Number(number));
        }
        (kind, raw) => return Err(type_error(kind, &raw, path)),
    }
    Ok(())
}

pub(super) fn graph(input: &[u8]) -> Result<super::wire::SerializedGraph, serde_json::Error> {
    let raw: Raw = serde_json::from_slice(input)?;
    let mut state = State::zero(Kind::Struct("SerializedGraph"));
    apply(
        Kind::Struct("SerializedGraph"),
        &mut state,
        raw,
        &mut Vec::new(),
    )?;
    serde_json::from_value(state.value())
}
