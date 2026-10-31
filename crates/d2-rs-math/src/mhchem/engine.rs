// Copyright (c) 2015-2021 Martin Hensel
// Copyright (c) 2026 luo-studio
// Copyright (c) 2026 erweixin (upstream RaTeX)
// SPDX-License-Identifier: Apache-2.0 AND MIT
// Source: mathjax-go v0.1.0 internal/mhchem/{buffer,data,engine,mhchem}.go.
use super::{Error, actions, patterns::match_pattern, texify};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock};
#[derive(Default)]
pub(super) struct Buffer {
    pub parenthesis_level: i64,
    pub begins_with_bond: bool,
    pub sb: bool,
    pub a: Option<String>,
    pub b: Option<String>,
    pub p: Option<String>,
    pub o: Option<String>,
    pub q: Option<String>,
    pub d: Option<String>,
    pub d_type: Option<String>,
    pub r: Option<String>,
    pub rdt: Option<String>,
    pub rd: Option<String>,
    pub rqt: Option<String>,
    pub rq: Option<String>,
    pub text: Option<String>,
    pub rm: Option<String>,
}
impl Buffer {
    pub fn clear_soft(&mut self) {
        let level = self.parenthesis_level;
        let bond = self.begins_with_bond;
        *self = Self::default();
        self.parenthesis_level = level;
        self.begins_with_bond = bond;
    }
    pub fn clear_all(&mut self) {
        *self = Self::default();
    }
}
pub(super) fn slot(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("")
}
#[derive(Deserialize)]
struct Machine {
    transitions: HashMap<String, Vec<Transition>>,
}
#[derive(Deserialize)]
struct Transition {
    pattern: String,
    task: Task,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    next_state: Option<String>,
    #[serde(default)]
    revisit: bool,
    #[serde(default)]
    to_continue: bool,
    #[serde(rename = "action_", default)]
    actions: Vec<Action>,
}
#[derive(Deserialize)]
pub(super) struct Action {
    #[serde(rename = "type_")]
    pub kind: String,
    #[serde(default)]
    pub option: Value,
}
static MACHINES: LazyLock<HashMap<String, Machine>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/machines.json")).expect("pinned mhchem machine table")
});
pub(super) fn error(message: impl Into<String>) -> Error {
    Error {
        id: "MhchemError".into(),
        message: format!("mhchem: {}", message.into()),
    }
}
fn bug() -> Error {
    Error {
        id: "MhchemBugU".into(),
        message: "mhchem bug U. Please report.".into(),
    }
}
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push(' '),
            '−' | '–' | '—' | '‐' => out.push('-'),
            '…' => out.push_str("..."),
            _ => out.push(c),
        }
    }
    out
}
pub(super) fn run(source: &str, machine: &str) -> Result<Vec<Value>, Error> {
    let mut input = normalize(source);
    if input.is_empty() {
        return Ok(vec![]);
    }
    let m = MACHINES
        .get(machine)
        .ok_or_else(|| error(format!("unknown state machine {machine}")))?;
    let mut state = "0";
    let mut buffer = Buffer::default();
    let mut output = Vec::new();
    let mut last_input = String::new();
    let mut watchdog = 10;
    loop {
        if input != last_input {
            watchdog = 10;
            last_input.clone_from(&input);
        } else {
            watchdog -= 1;
        }
        if watchdog <= 0 {
            return Err(bug());
        }
        let transitions = m
            .transitions
            .get(state)
            .or_else(|| m.transitions.get("*"))
            .ok_or_else(|| error(format!("no transitions for state {state}")))?;
        let mut consumed = false;
        for tr in transitions {
            let Some(hit) = match_pattern(&tr.pattern, &input)? else {
                continue;
            };
            consumed = true;
            let empty = input.is_empty();
            for action in &tr.task.actions {
                output.extend(actions::apply(machine, &mut buffer, &hit.token, action)?);
            }
            if let Some(next) = &tr.task.next_state {
                state = next;
            }
            if empty {
                return Ok(output);
            }
            if !tr.task.revisit {
                input = hit.remainder;
            }
            if !tr.task.to_continue {
                break;
            }
        }
        if !consumed {
            return Err(bug());
        }
    }
}
pub(super) fn to_tex(input: &str, mode: &str) -> Result<String, Error> {
    if !matches!(mode, "ce" | "pu" | "tex") {
        return Err(error(format!(
            "unknown mode {} (expected tex, ce, or pu)",
            d2_rs_core::quote_go_bytes(mode.as_bytes())
        )));
    }
    texify(&run(input, mode)?, mode != "tex")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_complete_expansions_and_errors() {
        let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/mhchem-expansion-source-v090.json"))
        .unwrap();
        assert_eq!(rows.len(), 973);
        for row in rows {
            match to_tex(
                row["input"].as_str().unwrap(),
                row["mode"].as_str().unwrap(),
            ) {
                Ok(tex) => {
                    assert_eq!(row["error"], Value::Null);
                    assert_eq!(
                        tex,
                        row["tex"].as_str().unwrap(),
                        "{} {:?}",
                        row["mode"],
                        row["input"]
                    );
                }
                Err(e) => {
                    assert_eq!(
                        e.id,
                        row["error"]["id"].as_str().unwrap(),
                        "{} {:?}",
                        row["mode"],
                        row["input"]
                    );
                    assert_eq!(e.message, row["error"]["message"].as_str().unwrap());
                }
            }
        }
    }
}
