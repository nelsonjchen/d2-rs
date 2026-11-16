// SPDX-License-Identifier: MPL-2.0
// Contracts from pinned d2js/d2wasm/functions.go and lib/urlenc/urlenc.go.
use super::*;

fn response(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap()
}

#[test]
fn legacy_codec_preserves_go_bytes_and_response_envelopes() {
    // Go 1.27 BestCompression, padded URL base64, and wrapWASMCall's data.
    for (script, encoded) in [
        ("", "AwA="),
        ("a -> b", "SlTQtVNIAgwA"),
        ("雪 e\u{301} مرحبا", "ejl7lULqmUaFm603Nt5Ye2PFjeWAAQA="),
        ("\0\n\r\t", "YuDi5QQMAA=="),
    ] {
        assert_eq!(
            response(encode(Some(script.into()))),
            serde_json::json!({"data": {"result": encoded}})
        );
        assert_eq!(
            response(decode(Some(encoded.into()))),
            serde_json::json!({"data": {"result": script}})
        );
    }
    assert_eq!(response(decode(Some("!".into())))["error"]["code"], 500);
}

#[test]
fn legacy_object_order_rejects_compile_errors_as_bad_requests() {
    for source in [
        "a: {shape: invalid}",
        "a: {shape: package; style.3d: true}",
        "a: {style.opacity: 2}",
    ] {
        let value = response(get_obj_order(Some(source.into())));
        assert_eq!(value["error"]["code"], 400, "{value}");
        assert!(!value["error"]["message"].as_str().unwrap().is_empty());
    }
    assert_eq!(
        response(get_obj_order(Some("a: {b}; c".into())))["data"]["order"],
        serde_json::json!(["a", "c", "a.b"])
    );
}

#[test]
fn legacy_elk_graph_rejects_compile_errors_as_bad_requests() {
    for source in [
        "a: {shape: invalid}",
        "a: {style.opacity: 2}",
        "a: {shape: image}",
        "a: {near: missing}",
        "a: {shape: package; style.3d: true}",
        "a: |md\n  <hr>\n|",
    ] {
        let request = serde_json::json!({"fs": {"index": source}});
        let value = response(get_elk_graph(Some(request.to_string())));
        assert_eq!(value["error"]["code"], 400, "{value}");
    }
}

#[test]
fn legacy_elk_graph_reserves_outside_labels_icons_and_modifiers() {
    // Pinned d2elklayout.ConvertGraph: positionLabelsIcons precedes degree
    // sizing and the icon row, followed by d2graph.SpacingOpt(5, 5, false).
    for (source, width, height) in [
        ("a: {height: -1}", 48.0, 25.0),
        ("a: {height: 19}", 48.0, 45.0),
        ("a: {height: 21}", 48.0, 21.0),
        ("a: {style.3d: true}", 68.0, 81.0),
        ("a: {shape: hexagon; style.3d: true}", 65.0, 76.0),
        ("a: {style.multiple: true}", 63.0, 76.0),
        ("a: {style.3d: true; style.multiple: true}", 68.0, 81.0),
        ("a: {width: 1; height: 1; b}", 1.0, 42.0),
        (
            "a: {icon: https://example.com/icon.svg; width: 21; height: 19}",
            21.0,
            71.0,
        ),
        (
            "a: {icon: https://example.com/icon.svg; icon.near: outside-top-left}",
            79.0,
            163.0,
        ),
    ] {
        let request = serde_json::json!({"fs": {"index": source}});
        let value = response(get_elk_graph(Some(request.to_string())));
        let node = &value["data"]["children"][0];
        assert_eq!(node["width"].as_f64(), Some(width), "{source}: {value}");
        assert_eq!(node["height"].as_f64(), Some(height), "{source}: {value}");
    }
}

#[test]
fn legacy_elk_graph_uses_structured_labels_and_shaped_container_padding() {
    for source in [
        "a: {shape: class}",
        "a: {shape: sql_table}",
        "a: {shape: text}",
        "a: |go\n  var n = 1\n|",
    ] {
        let request = serde_json::json!({"fs": {"index": source}});
        let value = response(get_elk_graph(Some(request.to_string())));
        let node = &value["data"]["children"][0];
        assert_eq!(node["id"], "a", "{source}: {value}");
        assert!(node.get("labels").is_none(), "{source}: {value}");
    }
    let request =
        serde_json::json!({"fs": {"index": "a: {shape: hexagon; style.multiple: true; b}"}});
    let value = response(get_elk_graph(Some(request.to_string())));
    assert_eq!(
        value["data"]["children"][0]["layoutOptions"]["elk.padding"],
        "[top=82,left=50,bottom=50,right=50]"
    );
    let request = serde_json::json!({"fs": {"index": "a: {shape: text; icon: https://example.com/icon.svg}"}});
    let value = response(get_elk_graph(Some(request.to_string())));
    assert_eq!(value["data"]["children"][0]["height"].as_f64(), Some(21.0));
    for (source, height) in [
        (
            "a: |md\n  hello\n|\na.icon: https://example.com/icon.svg",
            53.0,
        ),
        (
            "a: |latex\n  x^2\n|\na.icon: https://example.com/icon.svg",
            39.0,
        ),
    ] {
        let request = serde_json::json!({"fs": {"index": source}});
        let value = response(get_elk_graph(Some(request.to_string())));
        assert_eq!(
            value["data"]["children"][0]["height"].as_f64(),
            Some(height),
            "{source}: {value}"
        );
    }
}

#[test]
fn legacy_reference_arguments_follow_go_json_zero_values() {
    let lookup = |files: &str, board: &str| {
        response(get_ref_ranges(
            Some(files.into()),
            Some("index".into()),
            Some("missing".into()),
            Some(board.into()),
        ))
    };
    let empty = serde_json::json!({"data": {"ranges": null, "importRanges": null}});
    assert_eq!(lookup(r#"{"index":"a"}"#, "null"), empty);
    assert_eq!(lookup(r#"{"index":null}"#, "[]"), empty);
    for files in ["null", "{}"] {
        assert_eq!(
            lookup(files, "null"),
            serde_json::json!({"error": {"code":500,"message":"\"index\" not found"}})
        );
    }
    assert_eq!(
        lookup(r#"{"index":"a"}"#, r#"["layers","one"]"#),
        serde_json::json!({"error": {"code":500,"message":"board \"[layers one]\" not found"}})
    );
    assert_eq!(
        lookup("[]", "[]")["error"]["message"],
        "invalid fs argument"
    );
    assert_eq!(
        lookup(r#"{"index":"a"}"#, "{}")["error"]["message"],
        "invalid boardPath argument"
    );
}
