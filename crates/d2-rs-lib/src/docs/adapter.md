# Using the D2 adapter

This crate translates serialized D2 graph JSON into [`d2_tala_rs::Graph`], runs
the in-process engine, and serializes the typed D2 records with updated geometry. Use it when embedding the Rust engine in another program
that already has D2's measured graph format. External layout-plugin discovery
is outside this project's scope.

```text
serialized D2 JSON bytes
    → typed D2 record conversion (defaults, aliases, and validation)
    → decode supported layout facts into d2_tala_rs::Graph
    → Engine::layout
    → update boxes, routes, and supported label metadata
    → serialize canonical records; preserve application data
```

The original Go serializer emits known fields with their typed defaults and
omission rules. Unknown object and edge members are discarded. Store arbitrary
application metadata inside the graph’s `data` map.

## Basic layout

```
use serde_json::Value;
use d2_rs_lib::{D2Options, layout_json};

let input = br#"{
  "root": {"AbsID": "", "id": "", "ChildrenArray": ["a", "b"]},
  "objects": [
    {"AbsID": "a", "id": "a", "ChildrenArray": [], "box": {"Width": 80, "Height": 40}},
    {"AbsID": "b", "id": "b", "ChildrenArray": [], "box": {"Width": 80, "Height": 40}}
  ],
  "edges": [{"Src": "a", "Dst": "b", "index": 0}]
}"#;

let output = layout_json(input, &D2Options {
    seeds: vec![1],
    max_concurrency: 0,
})?;
let document: Value = serde_json::from_slice(&output)?;
assert!(document["objects"][0]["box"]["TopLeft"].is_object());
assert!(document["edges"][0]["route"].is_array());
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Choosing an entry point

| Function | Use it when |
|---|---|
| [`crate::layout_json`] | Only updated JSON bytes are needed. |
| [`crate::layout_json_with_report`] | Candidate diagnostics are also needed. |
| [`crate::route_edges_json`] | D2 already placed nodes and requests standalone edge routing. |
| [`crate::layout_snapshot_json`] | Compatibility tooling needs one intermediate stage. |

All functions are pure with respect to their input byte slice. Errors are
reported as [`crate::D2Error`]; no partial JSON document is returned.

## Options

[`crate::D2Options::seeds`] is ordered. One seed gives the simplest
reproducibility contract. Multiple seeds run TALA's complete-candidate selection;
see [`d2_tala_rs::guide::seeds_and_scoring`] for selection details.
