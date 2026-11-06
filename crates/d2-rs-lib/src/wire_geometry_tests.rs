// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

fn cases() -> Value {
    serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/wire-geometry-source-v090.json"
    ))
    .unwrap()
}

pub(super) fn run(input: &Value) -> Result<Value, D2Error> {
    let options = D2Options {
        seeds: serde_json::from_value(input["seeds"].clone()).unwrap(),
        max_concurrency: 1,
    };
    let token = CancellationToken::new();
    if input["cancel"].as_bool().unwrap() {
        token.cancel();
    }
    let bytes = if input["mode"] == "layout" {
        layout_json_with_cancellation(
            &serde_json::to_vec(&input["graph"]).unwrap(),
            &options,
            &token,
        )?
    } else {
        let selected: Vec<usize> = serde_json::from_value(input["requested"].clone()).unwrap();
        let mut requested = input["graph"].clone();
        requested["edges"] = json!(
            selected
                .iter()
                .map(|i| input["graph"]["edges"][*i].clone())
                .collect::<Vec<_>>()
        );
        let envelope = json!({
            "g": BASE64.encode(serde_json::to_vec(&input["graph"]).unwrap()),
            "gEdges": BASE64.encode(serde_json::to_vec(&requested).unwrap()),
        });
        route_edges_json_with_cancellation(
            &serde_json::to_vec(&envelope).unwrap(),
            &options,
            Some(&token),
        )?
    };
    Ok(serde_json::from_slice(&bytes).unwrap())
}

pub(super) fn geometry(graph: &Value) -> Value {
    // Complete inventories and exact float bits; do not normalize point order.
    let bits = |v: &Value| v.as_f64().unwrap().to_bits();
    let objects: BTreeMap<_, _> = graph["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let b = &o["box"];
            (
                o["AbsID"].as_str().unwrap().to_owned(),
                json!([
                    bits(&b["TopLeft"]["x"]),
                    bits(&b["TopLeft"]["y"]),
                    bits(&b["Width"]),
                    bits(&b["Height"]),
                ]),
            )
        })
        .collect();
    assert_eq!(objects.len(), graph["objects"].as_array().unwrap().len());
    let edges: Vec<_> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let route: Vec<_> = e["route"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| [bits(&p["x"]), bits(&p["y"])])
                .collect();
            json!({"source": e["Src"], "target": e["Dst"], "index": e["index"], "route": route})
        })
        .collect();
    json!({"objects":objects,"edges":edges})
}

#[test]
fn layout_rejects_stored_single_point_before_discarding_it() {
    let fixture = cases();
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["input"]["name"] == "single-route-layout")
        .unwrap();
    assert_eq!(
        run(&case["input"])
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default(),
        case["error"]
    );
}

#[test]
fn complete_public_wire_geometry_source_contract() {
    let fixture = cases();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 188);
    let mut differences = Vec::new();
    let mut successes = 0;
    let mut results = Vec::new();
    for row in rows {
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
            successes += 1;
            if geometry(actual) != geometry(&row["graph"]) {
                differences.push(format!(
                    "{name}: geometry {} != {}",
                    geometry(actual),
                    geometry(&row["graph"])
                ));
            }
        }
        results.push(json!({"name":name,"error":error,"graph":result.ok()}));
    }
    if let Some(path) = std::env::var_os("D2_RS_TEST_WIRE_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    assert_eq!(successes, rows.iter().filter(|r| r["error"] == "").count());
}
