#!/usr/bin/env python3
"""Public d2compiler oracle fixtures (tools/fixtures/langlibelk-language-public.toml).

usage: check-public-compiler.py link-security|class-applications|measured-state|<seeded family>

Builds tools/public-compiler-oracle against the runner's patched upstream tree
(upstream's own go.mod/go.sum, via scripts/language_oracle.py), replays the
requests from scripts/public_compiler_cases.py and writes
crates/d2-rs-language/tests/fixtures/public-<family>-v090.json (the historical
logical path; the content is a capture at the manifest pin).
`cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).

Seeded families (SEEDED below) replay constructed D2 inputs committed under
tools/fixtures/seeds/lang-public-compiler/<family>.json: {path, text|hex,
utf16?, files?} rows, where `text` is UTF-8 source and `files` maps import
paths to UTF-8 text (or {"hex": ...}). They were the request halves of the
v0.9.0-era fixtures; a few `pinned/d2compiler/compile_test.go#L…` rows are
literals taken from that upstream test, labelled with their origin. Only Go
output is regenerated.
"""
import json
import shutil
import sys

import public_compiler_cases as cases
from language_oracle import _build, _module, call, fixture, write
from references import ROOT


def oracle():
    module = _module("public-compiler-oracle")
    shutil.copy2(ROOT / "tools/public-compiler-oracle/main.go", module / "main.go")
    return _build(module)


def measured_rows(rows):
    """Node/edge label bytes, sizes, label dimensions and attributes after SetDimensions."""
    out = []
    for row in rows:
        assert row["error"] is None and not row["panic"] and row["measurement"] == {"error": None, "panic": False}, row
        g = row["after"]
        raw = {o["id"]: o["label_hex"] for o in row["measured_labels"]["objects"]}
        nodes = [dict(id=o["AbsID"], label_hex=raw[o["AbsID"]], size=[o["box"]["Width"], o["box"]["Height"]],
                      label_dimensions=[o["attributes"]["labelDimensions"]["width"], o["attributes"]["labelDimensions"]["height"]],
                      attributes=o["attributes"]) for o in g.get("objects") or []]
        edges = [dict(label_hex=row["measured_labels"]["edges"][i]["label_hex"],
                      label_dimensions=[e["attributes"]["labelDimensions"]["width"], e["attributes"]["labelDimensions"]["height"]],
                      attributes=e["attributes"]) for i, e in enumerate(g.get("edges") or [])]
        out.append(dict(nodes=nodes, edges=edges))
    return out


SEEDS = ROOT / "tools/fixtures/seeds/lang-public-compiler"
# family -> (seed name, file system); "memfs" compiles with lib/memfs as d2js does.
SEEDED = {
    "compiler-diagnostics": ("compiler-diagnostics", None),
    "compiler-memfs-diagnostics": ("compiler-memfs-diagnostics", "memfs"),
    "compiler-markdown-links": ("compiler-markdown-links", None),
    "graph-field-diagnostics": ("graph-field-diagnostics", None),
    "icon-url-compiler": ("icon-url-compiler", None),
    "late-validation": ("late-validation", None),
}


def seeded_requests(name):
    def encoded(value):
        return value.encode().hex() if isinstance(value, str) else value["hex"]
    rows = json.loads((SEEDS / f"{name}.json").read_text())
    return [{"hex": row["hex"] if "hex" in row else row["text"].encode().hex(), "path": row["path"],
             "utf16": row.get("utf16", False),
             "files_hex": {path: encoded(value) for path, value in row.get("files", {}).items()}} for row in rows]


def main():
    family = sys.argv[1]
    go = oracle()
    if family == "link-security":
        requests = cases.link_security()
        value = dict(scope="All4622 generated security/order observations,40 identity/early-return controls,and all4 retained "
                           "compiler differences. Tests diagnostic/acceptance contracts only;full raw source graphs preserved,not claimed matching.",
                     requests=requests, source=call(go, requests, "compile"))
    elif family == "class-applications":
        requests = cases.class_applications()
        value = dict(requests=requests, source=call(go, requests, "compile"))
    elif family == "measured-state":
        requests = cases.measurement_boundary() + cases.class_headers()
        value = dict(scope="Original Go public compiler then SetDimensions(nil,NewRuler,nil,nil),theme nil, over constructed "
                           "boundaries only: 468 label line-break/shape/font and class/sql_table row-body observations, 8 class "
                           "field/method row-order observations and 128 class-header language/italic/font-size observations. "
                           "Exact node/edge label bytes and dimensions, selected raw styles and metadata; geometry positions, "
                           "boards and whole graphs unqualified.",
                     requests=requests, source=measured_rows(call(go, requests, "measure")))
    elif family in SEEDED:
        seed, file_system = SEEDED[family]
        requests = seeded_requests(seed)
        replay = [dict(request, fs=file_system) for request in requests] if file_system else requests
        value = dict(requests=requests, source=call(go, replay, "compile"))
    else:
        sys.exit(f"unknown family {family}")
    assert len(value["source"]) == len(value["requests"])
    panics = sum(bool(row.get("panic")) for row in value["source"] if isinstance(row, dict))
    assert not panics, f"{panics} upstream panics: review before accepting"
    write(fixture(f"public-{family}-v090.json"), value)
    print(f"public-{family}: {len(requests)} requests captured")


if __name__ == "__main__":
    main()
