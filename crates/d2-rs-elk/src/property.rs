// Copyright (c) 2018, 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// elk-go/internal/rectpacking/options.go and core/registry.go.
use crate::{ElkNode, utility::option};
use serde_json::Value;
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => {
            let v = n.as_f64()?;
            Some(if v.fract() == 0.0 && v.abs() < 1e6 {
                format!("{v:.0}")
            } else {
                n.to_string()
            })
        }
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
pub(crate) fn enum_value(o: &Value, name: &str, values: &[&str], default: &str) -> String {
    let Some(value) = option(o, name).and_then(text) else {
        return default.into();
    };
    if values.contains(&value.as_str()) {
        return value;
    }
    value
        .parse::<isize>()
        .ok()
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| values.get(i))
        .unwrap_or(&default)
        .to_string()
}
pub(crate) fn flag(o: &Value, name: &str, default: bool) -> bool {
    option(o, name)
        .and_then(text)
        .and_then(|s| {
            if s.eq_ignore_ascii_case("true") {
                Some(true)
            } else if s.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
pub(crate) fn node_flag(n: &ElkNode, name: &str) -> bool {
    n.layout_options
        .as_ref()
        .is_some_and(|o| flag(o, name, false))
}
pub(crate) fn number_value(o: &Value, name: &str) -> Option<f64> {
    let text = option(o, name).and_then(text)?;
    let mut s = text.trim();
    for suffix in ['d', 'D', 'f', 'F'] {
        s = s.strip_suffix(suffix).unwrap_or(s);
    }
    d2_rs_core::parse_d2_float(s)
}
pub(crate) fn number(o: &Value, name: &str, default: f64) -> f64 {
    number_value(o, name).unwrap_or(default)
}
pub(crate) fn integer(o: &Value, name: &str) -> Option<i32> {
    let v = option(o, name)?;
    match v {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(_) => v
            .as_f64()
            .filter(|n| n.is_finite() && n.fract() == 0.0 && n.abs() < 1e6)
            .map(|n| n as i32),
        _ => None,
    }
}
