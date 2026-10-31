#!/usr/bin/env python3
"""D2 ELK adapter fixture generator (tools/fixtures/langlibelk-elk-adapter.toml).

usage: check-elk-adapter.py delete-bends|margin-shrink|trace-to-shape

  delete-bends    d2layouts/d2elklayout deleteBends + bend collision index
                  (hook tools/fixtures/hooks/elk-adapter-delete-bends)
  margin-shrink   the margin-shrinking block of d2elklayout.Layout, copied
                  verbatim from the pinned layout.go into the hook template
                  tools/fixtures/hooks/elk-adapter-margin-shrink
  trace-to-shape  exported d2graph.Edge.TraceToShape (tools/elk-trace-shape-oracle)

Inputs come from scripts/elk_cases/adapter.py. Everything builds in the
runner's patched upstream tree for arm64 (scripts/elk_oracle.py). `cargo test
-p d2-rs-elk --features oracle-fixtures` compares.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

from elk_cases import adapter
from elk_oracle import build_upstream, run, write_json
from fixture_recipe import output, workdir
from references import ROOT

FIXTURES = "crates/d2-rs-elk/src/fixtures/"
PACKAGE = "d2layouts/d2elklayout"
HOOKS = ROOT / "tools/fixtures/hooks"
MARGIN_START = "\tobjEdges := make(map[*d2graph.Object][]*d2graph.Edge)\n"
MARGIN_END = "\n\tfor _, edge := range g.Edges {\n\t\tpoints := edge.Route\n"


def run_hook(name, hook: Path, test: str, cases, fixture):
    inputs = workdir() / f"{name}-input.json"
    inputs.write_text(json.dumps(cases, indent=2) + "\n")
    oracle = build_upstream(name, PACKAGE, {f"{PACKAGE}/{name.replace('-', '_')}_test.go": hook}, test=True)
    staged = output(FIXTURES + fixture)
    subprocess.run([*oracle, "-test.run", f"^{test}$", "-test.count=1", "-test.v"], check=True,
                   env={**os.environ, "D2RS_RECIPE_INPUT": str(inputs), "D2RS_RECIPE_OUTPUT": str(staged)})
    if not staged.stat().st_size:
        sys.exit(f"{test} wrote no output")


kind = sys.argv[1]
if kind == "delete-bends":
    run_hook("d2rs-delete-bends", HOOKS / "elk-adapter-delete-bends/d2rs_delete_bends_test.go", "TestD2RSDeleteBends",
             adapter.delete_bends(), "original_delete_bends.json")
elif kind == "margin-shrink":
    layout = (Path(os.environ["D2RS_UPSTREAM"]) / PACKAGE / "layout.go").read_text()
    if layout.count(MARGIN_START) != 1:
        sys.exit("d2elklayout/layout.go: the margin block start marker moved; update this generator")
    start = layout.index(MARGIN_START)
    end = layout.index(MARGIN_END, start)
    template = (HOOKS / "elk-adapter-margin-shrink/d2rs_margin_shrink_test.go.tmpl").read_text()
    marker = "\t// D2RS-UPSTREAM-MARGIN-BLOCK\n"
    assert template.count(marker) == 1
    hook = workdir() / "d2rs_margin_shrink_test.go"
    hook.write_text(template.replace(marker, layout[start:end] + "\n"))
    run_hook("d2rs-margin-shrink", hook, "TestD2RSMarginShrink", adapter.margin_shrink(), "original_margin_shrink.json")
elif kind == "trace-to-shape":
    oracle = build_upstream("d2rs-trace-shape-oracle", "d2rs_trace_shape_oracle",
                           {"d2rs_trace_shape_oracle/main.go": ROOT / "tools/elk-trace-shape-oracle/main.go"})
    cases = adapter.trace_to_shape()
    q = run(oracle, json.dumps(cases).encode())
    if q.returncode:
        sys.exit(q.stderr.decode())
    result = json.loads(q.stdout)
    assert len(result) == len(cases)
    write_json(output(FIXTURES + "original_trace_to_shape.json"), result)
else:
    sys.exit(f"unknown adapter fixture {kind}")
