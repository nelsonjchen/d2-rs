#!/usr/bin/env python3
"""Generators for d2lib-compiled crates/d2-rs-render probe fixtures
(tools/fixtures/renderexport-render-probes.toml).

    render_probe_fixtures.py <family>

Each family builds its case list (hand-written D2 sources from
tools/fixtures/seeds/render-probes.json), runs the
tools/fixtures/hooks/render-compile-probe hook (d2lib.Compile with TALA seed 1
plus the requested SVG/scene/predicate extras) against $D2RS_UPSTREAM, and
writes the fixture layout its Rust reader expects. Run through
`uv run --no-project python scripts/regen-fixtures.py --only 'render-probe-*'`
(docs/FIXTURES.md).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402
from render_oracle_fixtures import go_test_hook  # noqa: E402

REPO = Path(__file__).resolve().parents[1]
FIXTURES = "crates/d2-rs-render/tests/fixtures/"
HOOK = "tools/fixtures/hooks/render-compile-probe/d2rs_compile_probe_test.go"
SEEDS = json.loads((REPO / "tools/fixtures/seeds/render-probes.json").read_text())


def run(cases: list[dict], test: str = "TestD2RSCompileProbe") -> list[dict]:
    work = fixture_recipe.workdir()
    request, raw = work / f"{test}-cases.json", work / f"{test}-rows.json"
    request.write_text(json.dumps(cases, ensure_ascii=False))
    go_test_hook("d2lib", HOOK, test, {"D2RS_RECIPE_INPUT": str(request), "D2RS_RECIPE_OUTPUT": str(raw)})
    rows = json.loads(raw.read_text())
    assert len(rows) == len(cases), (len(rows), len(cases))
    return rows


def write(name: str, value, indent=None) -> None:
    kwargs = dict(indent=indent) if indent else dict(separators=(",", ":"))
    fixture_recipe.output(FIXTURES + name).write_text(json.dumps(value, ensure_ascii=False, **kwargs) + "\n")


def upstream_text(path: str) -> str:
    return (Path(os.environ["D2RS_UPSTREAM"]) / path).read_text()


def seed_cases(family: str) -> list[dict]:
    out = []
    for case in SEEDS[family]:
        case = dict(case)
        if "upstream_source" in case:
            case["source"] = upstream_text(case.pop("upstream_source"))
        out.append(case)
    return out


# --- families ---------------------------------------------------------------

def sequence_note_references():
    cases = seed_cases("sequence-note-references")
    out = []
    for c, r in zip(cases, run([dict(c, extras=["references"]) for c in cases])):
        refs = r["references"]
        row = {"name": c["name"], "source": c["source"]}
        if "error" in refs:
            row["error"] = refs["error"]
        else:
            row["objects"] = refs["objects"]
        out.append(row)
    write("sequence-note-references-source-v090.json", out, indent=2)


def sequence_groups():
    """sequence-group-reference: group predicates and fills."""
    cases = seed_cases("sequence-groups")
    rows = {c["name"]: r for c, r in zip(cases, run([dict(c, extras=["compile", "predicates"]) for c in cases]))}
    by_name = {c["name"]: c for c in cases}
    reference = []
    for name in SEEDS["sequence-group-reference"]:
        r = rows[name]
        reference.append({"name": name, "source": by_name[name]["source"],
                          "groups": [p["id"] for p in r["predicates"] if p["group"]],
                          "group_fills": {p["id"]: p["fill"] for p in r["predicates"] if p["group"]}})
    write("sequence-group-reference-source-v090.json", reference, indent=2)


def target_empty_config_bytes():
    """Hash-input bytes (d2target.Diagram.Bytes) of non-renderable empty targets."""
    cases = seed_cases("target-empty-config-bytes")
    out = []
    for c, r in zip(cases, run([dict(c, layout="dagre", extras=["compile", "bytes"]) for c in cases])):
        assert "bytes" in r, (c["name"], r.get("compile_error"))
        out.append({"name": c["name"], "source": c["source"], "bytes": r["bytes"]})
    write("target-empty-config-bytes-source-v090.json", out, indent=2)


FAMILIES = {
    "target-empty-config-bytes": target_empty_config_bytes,
    "sequence-note-references": sequence_note_references,
    "sequence-groups": sequence_groups,
}

if __name__ == "__main__":
    FAMILIES[sys.argv[1]]()
