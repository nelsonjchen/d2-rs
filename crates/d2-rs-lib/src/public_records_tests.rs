// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::wire_geometry_tests::run;
use super::*;

pub(super) fn difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => {
            let same = if a.is_f64() || b.is_f64() {
                a.as_f64().unwrap().to_bits() == b.as_f64().unwrap().to_bits()
            } else {
                a == b
            };
            (!same).then(|| format!("{path}: {actual} != {expected}"))
        }
        (Value::Object(a), Value::Object(b)) => {
            let ak: BTreeSet<_> = a.keys().collect();
            let bk: BTreeSet<_> = b.keys().collect();
            if ak != bk {
                return Some(format!(
                    "{path}: missing {:?}, extra {:?}",
                    ak.difference(&bk).collect::<Vec<_>>(),
                    bk.difference(&ak).collect::<Vec<_>>()
                ));
            }
            ak.into_iter()
                .find_map(|key| difference(&a[key], &b[key], &format!("{path}.{key}")))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: length {} != {}", b.len(), a.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (a, b))| difference(a, b, &format!("{path}[{i}]")))
        }
        _ => (expected != actual).then(|| format!("{path}: {actual} != {expected}")),
    }
}

#[test]
fn complete_public_records_source_contract() {
    let fixture: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!("tests/fixtures/public-records-source-v090.json"))
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 6562);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
        assert!(row.get("panic").is_none(), "unexpected source panic");
        let result = run(&row["input"]);
        let error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let name = &row["input"]["name"];
        if error != row["error"] {
            differences.push(format!(
                "{name}: error {error:?}, expected {}",
                row["error"]
            ));
        } else if let Ok(actual) = &result {
            if let Some(diff) = difference(&row["graph"], actual, "$") {
                differences.push(format!("{name}: {diff}"));
            } else {
                successes += 1;
            }
        }
        results.push(json!({"name":name,"error":error,"graph":result.ok()}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_PUBLIC_RECORDS_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, 1445);
}
