#!/usr/bin/env python3
"""Compiler contract fixtures for crates/d2-rs-language (tools/fixtures/langlibelk-language-contracts.toml).

usage: check-language-compile-contracts.py <family>

Each family replays constructed D2 inputs through the unchanged Go compiler at
the manifest pin and projects the result onto the fields its Rust test
compares. Inputs are the committed seeds in
tools/fixtures/seeds/lang-compile-contracts/<family>.json (the input halves of
the v0.9.0-era fixtures, no Go output) or, for the legend and config-data
families, the hand-written request files the tests read directly
(crates/d2-rs-language/tests/fixtures/{legend-validation,config-data}-inputs-v090.json).

Go side: tools/public-compiler-oracle (d2compiler.Compile; `inspect` adds the
legend, object parents/tooltips/declaration lines, edge AbsIDs/reference
scopes and the d2exporter shape tooltips; `measure` runs SetDimensions with
textmeasure.NewRuler) and, for style-colors, the lib/color hook
tools/language-master-scoreboard/style_color_hook_test.go (color.ValidColor).
Logical paths keep their historical names. `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
import sys

from fixture_recipe import go_env, workdir
from language_oracle import call, fixture, upstream, write
from references import ROOT

import importlib.util

_spec = importlib.util.spec_from_file_location("public_compiler", ROOT / "scripts/check-public-compiler.py")
public_compiler = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(public_compiler)

SEEDS = ROOT / "tools/fixtures/seeds/lang-compile-contracts"


def seed(name):
    return json.loads((SEEDS / f"{name}.json").read_text())


def inputs(name):
    return json.loads((ROOT / "crates/d2-rs-language/tests/fixtures" / name).read_text())


_oracle = None


def oracle():
    global _oracle
    if _oracle is None:
        _oracle = public_compiler.oracle()
    return _oracle


def run(mode, path, sources, utf16=None):
    requests = [{"hex": source.encode().hex(), "path": path, "utf16": bool(utf16[i]) if utf16 else False, "files_hex": {}}
                for i, source in enumerate(sources)]
    rows = call(oracle(), requests, mode)
    assert len(rows) == len(requests)
    assert not any(row.get("panic") for row in rows), "upstream panic: review before accepting"
    return rows


def objects(row, key="graph"):
    return {o["AbsID"]: o for o in (row[key].get("objects") or [])}


def errors_family(name, logical):
    cases = seed(name)
    rows = run("compile", "validation.d2", [c["source"] for c in cases])
    out = [{"name": c["name"], "source": c["source"], "errors": row["error"].split("\n") if row["error"] else []}
           for c, row in zip(cases, rows)]
    write(fixture(logical), {"cases": out})


def position_validation():
    cases = seed("position-validation")
    rows = run("compile", "validation.d2", [c["source"] for c in cases], [c.get("utf16", False) for c in cases])
    write(fixture("position-validation-source-v090.json"),
          [dict(c, error=row["error"] or "") for c, row in zip(cases, rows)])


def markdown_compiler():
    cases = seed("markdown-compiler")
    rows = run("compile", "validation.d2", [c["source"] for c in cases])
    write(fixture("markdown-compiler-source-v090.json"),
          [{"error": row["error"] or "", "name": c["name"], "source": c["source"]} for c, row in zip(cases, rows)])


def legend_validation():
    cases = inputs("legend-validation-inputs-v090.json")
    rows = run("inspect", "legend.d2", [c["source"] for c in cases])
    write(fixture("legend-validation-source-v090.json"),
          [{"error": row["error"], "legend": None if row["error"] else row["inspect"]["legend"], "name": c["name"]}
           for c, row in zip(cases, rows)])


def config_data():
    cases = inputs("config-data-inputs-v090.json")
    rows = run("compile", "config.d2", [c["source"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        assert row["error"] is None and row["config"] is not None, (c, row)
        out.append({"data": row["config"]["data"], "name": c["name"]})
    write(fixture("config-data-source-v090.json"), out)


def tooltip_handoff():
    cases = seed("tooltip-handoff")
    rows = run("inspect", "tooltip.d2", [c["source"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        if row["error"]:
            out.append({"error": row["error"], "name": c["name"], "objects": [], "source": c["source"], "targets": []})
            continue
        inspected = row["inspect"]
        assert "export_error" not in inspected, (c, inspected)
        out.append({"error": "", "name": c["name"],
                    "objects": [{"id": o["id"], "position": o["tooltip_position"], "tooltip": o["tooltip"]} for o in inspected["objects"]],
                    "source": c["source"], "targets": inspected["targets"]})
    write(fixture("tooltip-handoff-source-v090.json"), out)


def raw_label_bytes():
    cases = seed("raw-label-bytes")
    rows = run("measure", "source.d2", [c["input"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        if row["error"]:
            out.append(dict(c, compile_error=row["error"]))
            continue
        # SetDimensions applies text-transform to the label bytes before it
        # measures them; a later measurement error (invalid UTF-8 LaTeX) does
        # not change the transformed bytes this fixture pins.
        assert not row["measurement"]["panic"], (c, row["measurement"])
        before ={o["id"]: o["label_hex"] for o in row["raw_labels"]["objects"]}
        items = []
        for o in row["measured_labels"]["objects"]:
            after = bytes.fromhex(o["label_hex"])
            try:
                after.decode("utf-8")
                valid = True
            except UnicodeDecodeError:
                valid = False
            items.append({"after_hex": o["label_hex"], "before_hex": before[o["id"]], "id": o["id"], "valid_utf8": valid})
        out.append(dict(c, objects=items))
    write(fixture("raw-label-bytes-source-v090.json"), out)


def style_value(attributes, key):
    value = (attributes.get("style") or {}).get(key)
    return {"value": value["value"]} if value is not None else None


def sequence_canonical_identity():
    cases = seed("sequence-canonical-identity")
    rows = run("inspect", "identity.d2", [c["source"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        assert row["error"] is None, (c, row["error"])
        serialized = objects(row)
        inspected = row["inspect"]
        objs = [{"fill": style_value(serialized[o["id"]]["attributes"], "fill"), "id": o["id"],
                 "label": serialized[o["id"]]["attributes"]["label"]["value"], "parent": o["parent"],
                 "stroke": style_value(serialized[o["id"]]["attributes"], "stroke")} for o in inspected["objects"]]
        edges = []
        for e, extra in zip(row["graph"].get("edges") or [], inspected["edges"]):
            scopes = list(dict.fromkeys(extra["scopes"])) or [""]
            edges.append({"scopes": scopes, "source": e["Src"], "target": e["Dst"]})
        out.append({"name": c["name"], "source": c["source"], "expected": {"edges": edges, "objects": objs}})
    write(fixture("sequence-canonical-identity-source-v090.json"), out)


def sequence_declaration_lines():
    cases = seed("sequence-declaration-lines")
    rows = run("inspect", "lines.d2", [c["source"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        assert row["error"] is None, (c, row["error"])
        inspected = row["inspect"]
        edges = [{"line": extra["line"], "source": e["Src"], "target": e["Dst"]}
                 for e, extra in zip(row["graph"].get("edges") or [], inspected["edges"])]
        out.append({"edges": edges, "name": c["name"], "objects": {o["id"]: o["line"] for o in inspected["objects"]},
                    "source": c["source"]})
    write(fixture("sequence-declaration-lines-source-v090.json"), out)


def compiled_edge_identity():
    cases = seed("compiled-edge-identity")
    rows = run("inspect", "-", [c["source"] for c in cases])
    out = []
    for c, row in zip(cases, rows):
        if row["error"]:
            out.append({"edges": [], "error": row["error"], "input": c})
            continue
        edges = [{"abs_id": extra["abs_id"], "dst": e["Dst"], "dst_arrow": e["dst_arrow"], "index": e["index"],
                  "src": e["Src"], "src_arrow": e["src_arrow"]}
                 for e, extra in zip(row["graph"].get("edges") or [], row["inspect"]["edges"])]
        out.append({"edges": edges, "error": "", "input": c})
    write(fixture("compiled-edge-identity-v090.json"),
          {"oracle": "d2compiler.Compile; d2graph.Edge.Index/AbsID", "cases": out})


def measured(name):
    cases = seed(name)
    rows = run("measure", "fixture.d2", [c["input"] for c in cases])
    for c, row in zip(cases, rows):
        assert row["error"] is None and row["measurement"] == {"error": None, "panic": False}, (c, row)
    return cases, rows


def authored_shape_sizing():
    cases, rows = measured("authored-shape-sizing")
    out = []
    for c, row in zip(cases, rows):
        box = objects(row, "after")["a"]["box"]
        out.append({"input": c["input"], "width": box["Width"], "height": box["Height"]})
    write(fixture("authored-shape-sizing-v090.json"), out)


def sequence_default_shapes():
    cases, rows = measured("sequence-default-shapes")
    out = []
    for c, row in zip(cases, rows):
        o = objects(row, "after")[c["id"]]
        dims = o["attributes"]["labelDimensions"]
        out.append({"input": c["input"], "id": c["id"], "shape": o["attributes"]["shape"]["value"],
                    "width": o["box"]["Width"], "height": o["box"]["Height"],
                    "label_width": dims["width"], "label_height": dims["height"]})
    write(fixture("sequence-default-shapes-v090.json"), out)


def connection_label_language():
    cases, rows = measured("connection-label-language")
    out = []
    for c, row in zip(cases, rows):
        attributes = row["after"]["edges"][0]["attributes"]
        out.append({"input": c["input"], "text": attributes["label"]["value"], "language": attributes.get("language", ""),
                    "size": {"height": attributes["labelDimensions"]["height"], "width": attributes["labelDimensions"]["width"]}})
    write(fixture("connection-label-language-v090.json"), out)


def style_colors():
    """color.ValidColor over the retained corpus via the env-gated lib/color hook."""
    source = upstream()
    work = workdir() / "style-colors"
    work.mkdir(parents=True)
    corpus = work / "input.json"
    corpus.write_text(json.dumps([{"input": value, "valid": False} for value in seed("style-colors")]))
    result = work / "output.json"
    overlay = work / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(source / "lib/color/d2rs_master_colors_test.go"): str(ROOT / "tools/language-master-scoreboard/style_color_hook_test.go")}}))
    env = dict(go_env(), D2RS_STYLE_COLOR_INPUT=str(corpus), D2RS_STYLE_COLOR_OUTPUT=str(result))
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay), "-run", "^TestD2rsMasterStyleColors$",
                    "./lib/color"], cwd=source, env=env, check=True)
    values = json.loads(result.read_text())
    assert [v["input"] for v in values] == seed("style-colors")
    write(fixture("style-color-go-master.json"), values, compact=True)


FAMILIES = {
    "style-color-diagnostics": lambda: errors_family("style-color-diagnostics", "style-color-diagnostics-go-master.json"),
    "style-color-scope": lambda: errors_family("style-color-scope", "style-color-scope-go-v090.json"),
    "style-enumerations": lambda: errors_family("style-enumerations", "style-enumerations-go-v090.json"),
    "style-opacity": lambda: errors_family("style-opacity", "style-opacity-go-v090.json"),
    "style-colors": style_colors,
    "position-validation": position_validation,
    "markdown-compiler": markdown_compiler,
    "legend-validation": legend_validation,
    "config-data": config_data,
    "tooltip-handoff": tooltip_handoff,
    "raw-label-bytes": raw_label_bytes,
    "sequence-canonical-identity": sequence_canonical_identity,
    "sequence-declaration-lines": sequence_declaration_lines,
    "compiled-edge-identity": compiled_edge_identity,
    "authored-shape-sizing": authored_shape_sizing,
    "sequence-default-shapes": sequence_default_shapes,
    "connection-label-language": connection_label_language,
}

if __name__ == "__main__":
    FAMILIES[sys.argv[1]]()
    print(f"{sys.argv[1]}: captured")
