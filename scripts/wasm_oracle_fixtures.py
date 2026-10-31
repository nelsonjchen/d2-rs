#!/usr/bin/env python3
"""Generators for the d2-rs-wasm d2wasm-test fixtures (tools/fixtures/renderexport-d2js.toml).

    scoreboard --script scripts/<wasm scoreboard>.py --output <logical>
        Run the scoreboard's original-test capture (upstream d2js/d2wasm test
        bodies with call replacements plus its hook, GOOS=js GOARCH=wasm)
        against $D2RS_UPSTREAM through a go overlay (scripts/upstream_overlay.py),
        verify it with the scoreboard's own verify(), and write the case list.
    legacy --output <logical>
        Re-evaluate the retained regression inputs (successful root-structured
        targets, raw ASCII render requests, two completions) at the pin with
        tools/wasm-admission-scoreboard/legacy_test.go, as
        .reference/codex-runs/run70-refresh-legacy.py did. The inputs come from
        the `d2js-public` and `d2js-target` artifacts in $D2RS_FIXTURES_DIR.

Run through scripts/regen-fixtures.py (docs/FIXTURES.md).
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import fixture_recipe  # noqa: E402
import upstream_overlay  # noqa: E402


def load(script: Path):
    spec = importlib.util.spec_from_file_location(script.stem.replace("-", "_"), script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def scoreboard(args):
    work = fixture_recipe.workdir()
    upstream_overlay.redirect(work / "overlay")
    score = load(ROOT / args.script)
    score = getattr(score, "base", score)  # wasm-isometric configures the admission module
    capture = work / "capture"
    score.capture_original(capture)
    cases = score.verify(capture)
    fixture_recipe.output(args.output).write_text(json.dumps(cases, indent=2) + "\n")


def legacy(args):
    work = fixture_recipe.workdir()
    upstream = upstream_overlay.redirect(work / "overlay")
    fixtures = Path(os.environ["D2RS_FIXTURES_DIR"])
    wasm = load(ROOT / "scripts/wasm-admission-scoreboard.py")
    cli = load(ROOT / "scripts/cli-scoreboard.py")
    capture = work / "capture"
    (capture / "tmp").mkdir(parents=True)
    rows, bindings = [], {}
    for logical in ["crates/d2-rs-wasm/tests/fixtures/root-structured-source-v090.json",
                    "crates/d2-rs-wasm/tests/fixtures/raw-target-render-v090.json"]:
        path = fixtures / logical
        bindings[logical] = hashlib.sha256(path.read_bytes()).hexdigest()
        data = json.loads(path.read_text())
        for c in data["cases"]:
            if logical.endswith("root-structured-source-v090.json"):
                if "error" not in c["response"]:
                    d = c["response"]["data"]
                    rows.append(dict(name=c["name"], kind="direct", request=dict(diagram=d["diagram"], options=d["renderOptions"])))
            elif c["options"].get("ascii"):
                rows.append(dict(name=c["name"], kind="ascii", request=dict(diagram=c["diagram"], options=c["options"])))
    for name, source in [("ascii-completion", "node: {shape: }"), ("unicode-completion", '"😀".node: {shape: }')]:
        rows.append(dict(name=name, kind="completion", source=source, line=0, column=len(source.encode("utf-16-le")) // 2 - 1))
    requests = capture / "requests.json"
    requests.write_text(json.dumps(rows, ensure_ascii=False) + "\n")
    hook = ROOT / "tools/wasm-admission-scoreboard/legacy_test.go"
    (upstream / "d2js/d2wasm/d2rs_legacy_refresh_test.go").write_bytes(hook.read_bytes())  # overlay entry
    env = dict(os.environ, GOOS="js", GOARCH="wasm", GOMAXPROCS="1", TMPDIR=str(capture / "tmp"),
               D2RS_WASM_REGRESSION_INPUT=str(requests), D2RS_WASM_REGRESSION_OUTPUT=str(capture / "responses.json"))
    result = cli.bounded_run(["go", "test", "./d2js/d2wasm", "-exec", "bash " + str(wasm.wasm_exec(env)),
                              "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                              "-run", "^TestD2rsRefreshLegacyWASM$", "-json", "-count=1"],
                             env, capture, capture / "go.jsonl", 600)
    if result != dict(exit=0, failure=None):
        sys.exit(f"legacy capture failed: {result}; see {capture / 'go.jsonl'}")
    assert cli.events(capture / "go.jsonl")["TestD2rsRefreshLegacyWASM"] == "pass"
    responses = json.loads((capture / "responses.json").read_text())
    assert [(r["name"], r["kind"]) for r in rows] == [(r["name"], r["kind"]) for r in responses]
    record = {"source_commit": json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"],
              "inputs": bindings, "hook_sha256": hashlib.sha256(hook.read_bytes()).hexdigest(), "rows": responses}
    fixture_recipe.output(args.output).write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    s = sub.add_parser("scoreboard")
    s.add_argument("--script", required=True)
    s.add_argument("--output", required=True)
    s = sub.add_parser("legacy")
    s.add_argument("--output", required=True)
    args = parser.parse_args()
    {"scoreboard": scoreboard, "legacy": legacy}[args.mode](args)


if __name__ == "__main__":
    main()
