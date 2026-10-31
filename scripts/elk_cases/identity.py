"""elk-go JSON admission controls: list presence, label IDs, junction points, coordinate frames
(crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 junction-alignment, label-traversal and
coordinate-frame probes (their inline generators were not retained; these
reproduce the recorded inputs and stdin text exactly). `raw_input` is the
exact stdin text where it matters (`-0`, compact spellings, raw UTF-8, or
values that are not valid graph JSON); `input` is its parse. The `invalid_*`
functions return rows whose stdin elk-go must reject.
"""
import copy
import json

from elk_cases import junctions

SENTINEL = '"@@D2RS-RAW@@"'


def _raw(graph, literal: str) -> str:
    """json.dumps(graph) with the one SENTINEL string replaced by `literal` text."""
    text = json.dumps(graph)
    assert text.count(SENTINEL) == 1
    return text.replace(SENTINEL, literal)


def root_list_presence():
    rows = []
    absent = object()
    lists = [("absent", absent), ("null", None), ("empty", [])]
    options = [("absent", absent), ("null", None), ("empty", {}), ("layered", {"elk.algorithm": "layered"})]
    for children_name, children in lists:
        for edges_name, edges in lists:
            for options_name, layout in options:
                g = {"id": "root"}
                for key, value in [("children", children), ("edges", edges), ("layoutOptions", layout)]:
                    if value is not absent:
                        g[key] = copy.deepcopy(value)
                rows.append(dict(name=f"presence-empty-children{children_name}-edges{edges_name}-options{options_name}",
                                 input=g))
    nodes = [{"id": "a", "width": 61.25, "height": 26.5}, {"id": "b", "width": 68.375, "height": 29.5625}]
    for direction in ["RIGHT", "DOWN", "LEFT", "UP"]:
        for edges_name, edges in lists:
            g = {"id": "root", "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction},
                 "children": copy.deepcopy(nodes)}
            if edges is not absent:
                g["edges"] = copy.deepcopy(edges)
            rows.append(dict(name=f"presence-{direction.lower()}-isolated-edges{edges_name}", input=g))
    for options_name, layout in options[:3]:
        for shape, edges in [("isolated", []), ("path", [{"id": "ab", "sources": ["a"], "targets": ["b"]}])]:
            g = {"id": "root", "children": copy.deepcopy(nodes), "edges": copy.deepcopy(edges)}
            if layout is not absent:
                g["layoutOptions"] = copy.deepcopy(layout)
            rows.append(dict(name=f"presence-default-{shape}-options{options_name}", input=g))
    return rows


LABEL_IDS = [("absent", None), ("null", "null"), ("empty", '""'), ("string", '"label"'), ("unicode", '"数式⚙"'),
             ("integer", "42"), ("negative", "-7"), ("negative-zero", "-0"), ("maximum", "9223372036854775807"),
             ("minimum", "-9223372036854775808")]


def label_identity():
    rows = []
    for place in ["node", "edge", "loop"]:
        for name, literal in LABEL_IDS:
            label = {"text": "label", "width": 27.25, "height": 9.5}
            if literal is not None:
                label["id"] = "@@D2RS-RAW@@"
            a = {"id": "a", "width": 60, "height": 40}
            edge = {"id": "e", "sources": ["a"], "targets": ["a" if place == "loop" else "b"]}
            if place == "node":
                a["labels"] = [label]
            else:
                edge["labels"] = [label]
            g = {"id": "root", "layoutOptions": {"elk.algorithm": "layered", "elk.direction": "RIGHT"},
                 "children": [a, {"id": "b", "width": 60, "height": 40}], "edges": [edge]}
            raw = json.dumps(g) if literal is None else _raw(g, literal)
            rows.append(dict(name=f"label-id-{place}-{name}", input=json.loads(raw), raw_input=raw))
    return rows


def invalid_label_identity():
    rows = []
    for name, literal in [("boolean", "true"), ("fraction", "1.25"), ("decimal-integer", "1.0"), ("exponent", "1e3"),
                          ("overflow", "9223372036854775808"), ("underflow", "-9223372036854775809"),
                          ("object", "{}"), ("array", "[]")]:
        raw = ('{"id":"root","children":[{"id":"a","width":60,"height":40,"labels":[{"id":' + literal +
               ',"text":"label","width":27.25,"height":9.5}]}]}')
        rows.append(dict(name=f"label-id-invalid-{name}", raw_input=raw))
    return rows


def _junction_graph():
    return junctions.shared_port_alignment()[0]["input"]


def junction_admission():
    rows = []
    for name, literal in [("null", "null"), ("empty", "[]"), ("empty-point", "[{}]"), ("null-point", "[null]"),
                          ("null-coordinates", '[{"x":null,"y":null}]'), ("missing-y", '[{"x":19.25}]'),
                          ("missing-x", '[{"y":-0}]'), ("stale", '[{"x":-500,"y":900}]'),
                          ("extra", '[{"x":1,"y":2,"extra":{"value":1}}]'), ("capital-coordinates", '[{"X":1,"Y":null}]')]:
        g = _junction_graph()
        g["edges"][3]["junctionPoints"] = "@@D2RS-RAW@@"
        raw = _raw(g, literal)
        rows.append(dict(name=f"junction-admission-{name}", input=json.loads(raw), raw_input=raw))
    return rows


def invalid_junction_admission():
    rows = []
    for name, literal in [("capital-string-x", '[{"X":"bad"}]'), ("boolean", "true"), ("boolean-point", "[true]"),
                          ("string-x", '[{"x":"x"}]'), ("boolean-y", '[{"y":false}]'), ("array-point", "[[]]")]:
        g = _junction_graph()
        g["edges"][3]["junctionPoints"] = "@@D2RS-RAW@@"
        rows.append(dict(name=name, raw_input=_raw(g, literal)))
    return rows


def coordinate_ancestor_cases():
    rows = []
    for name, edges, extra in [("forward", [("C", "Z")], False), ("reverse", [("Z", "C")], False),
                               ("ancestor-source", [("A", "C")], False), ("ancestor-target", [("C", "A")], False),
                               ("ancestor-fanout", [("A", "C"), ("A", "Y")], True)]:
        children = [{"id": "A", "width": 40, "height": 30, "children": [
            {"id": "B", "width": 40, "height": 30, "children": [{"id": "C", "width": 40, "height": 30}]}]},
                    {"id": "Z", "width": 40, "height": 30}]
        if extra:
            children.append({"id": "Y", "width": 40, "height": 30})
        g = {"id": "root", "layoutOptions": {
            "elk.algorithm": "layered", "elk.direction": "RIGHT", "elk.hierarchyHandling": "INCLUDE_CHILDREN",
            "elk.nodeSize.constraints": "MINIMUM_SIZE", "elk.contentAlignment": "H_CENTER V_CENTER",
            "elk.spacing.nodeNodeBetweenLayers": 20, "elk.spacing.edgeNodeBetweenLayers": 10,
            "elk.layered.spacing.edgeEdgeBetweenLayers": 50},
             "children": children,
             "edges": [{"id": f"e{i}", "sources": [s], "targets": [t]} for i, (s, t) in enumerate(edges)]}
        rows.append(dict(name="coordinate-" + name, input=g, raw_input=json.dumps(g) + "\n"))
    return rows
