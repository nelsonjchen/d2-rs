#!/usr/bin/env python3
"""Observe original master renderer calls; compare complete native results.

Five original functions, 15 calls. No output normalization. Original assertions
remain active. Streaming limits: 1 GiB total, 128 MiB/file, 20 GiB free;
300-second subprocess deadline, fresh capture and native paths.
"""
import argparse
import collections
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = references.ROOT
TEMPLATE = ROOT / "tools/renderer-contract-scoreboard/observer.go.txt"
SUITES = {
    "animate": {
        "package": "d2renderers/d2animate", "file": "d2animate_test.go",
        "tests": {"TestWrapDefaultsMissingPadding": 1,
                  "TestWrapRejectsMalformedTargetBeforeTraversal": 1},
        "imports": '"github.com/d2lang/d2/d2renderers/d2svg"\n"github.com/d2lang/d2/d2target"',
        "replacements": {"Wrap(": "d2rsWrap(t, "},
        "wrappers": '''func d2rsWrap(t *testing.T, d *d2target.Diagram, frames [][]byte, opts d2svg.RenderOpts, interval int) ([]byte, error) {
    out, err := Wrap(d, frames, opts, interval)
    d2rsRecord(t, map[string]any{"operation":"wrap", "diagram":d, "frames":frames, "opts":opts, "interval":interval}, out, err)
    return out, err
}''',
        "sources": ["d2animate.go", "../d2svg/d2svg.go", "../../d2target/render_validation.go"],
    },
    "scene": {
        "package": "d2renderers/d2scene", "file": "aspect_test.go",
        "tests": {"TestAspectRatioMatrixRoundsViewportCancellation": 5,
                  "TestAspectRatioMatrixPreservesTinySourceOrigins": 1},
        "imports": "", "replacements": {"AspectRatioMatrix(": "d2rsAspect(t, "},
        "wrappers": '''func d2rsAspect(t *testing.T, source, destination Box, aspect AspectRatio) (Matrix, error) {
    out, err := AspectRatioMatrix(source, destination, aspect)
    d2rsRecord(t, map[string]any{"operation":"aspect", "source":source, "destination":destination, "aspect":aspect}, out, err)
    return out, err
}''',
        "sources": ["aspect.go", "geometry.go"],
    },
    "latex": {
        "package": "d2renderers/d2latex", "file": "latex_test.go",
        "tests": {"TestInputLimits": 7}, "imports": "",
        "replacements": {"ValidateInput(": "d2rsValidate(t, ", "Render(": "d2rsRender(t, ", "Measure(": "d2rsMeasure(t, "},
        "wrappers": '''func d2rsValidate(t *testing.T, input string) error {
    err := ValidateInput(input)
    d2rsRecord(t, map[string]any{"operation":"validate", "input":input}, nil, err)
    return err
}
func d2rsRender(t *testing.T, input string) (string, error) {
    out, err := Render(input)
    d2rsRecord(t, map[string]any{"operation":"render", "input":input}, out, err)
    return out, err
}
func d2rsMeasure(t *testing.T, input string) (int, int, error) {
    w, h, err := Measure(input)
    d2rsRecord(t, map[string]any{"operation":"measure", "input":input}, []int{w,h}, err)
    return w, h, err
}''',
        "sources": ["latex_common.go", "limits.go"],
    },
}


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def transform(text, replacements):
    # Go comments and string/rune literals are opaque; assertion text stays exact.
    tokens = re.split(r'("(?:\\.|[^"\\])*"|`[^`]*`|\'(?:\\.|[^\'\\])*\'|//[^\n]*|/\*[\s\S]*?\*/)', text)
    for i in range(0, len(tokens), 2):
        for old, new in replacements.items():
            tokens[i] = re.sub(r"\b" + re.escape(old), new, tokens[i])
    return "".join(tokens)


def source():
    pinned = references.verify_source()
    names = [str(Path(s["package"]) / n) for s in SUITES.values()
             for n in [s["file"], *s["sources"]]]
    return {"commit": references.SOURCE_COMMIT,
            "files": {n: sha(pinned / n) for n in names},
            "observer": sha(TEMPLATE), "script": sha(Path(__file__))}


def hashes(capture):
    paths = [p for p in capture.rglob("*") if p.is_file() and p.name != "hashes.json"]
    return {str(p.relative_to(capture)): sha(p) for p in sorted(paths)}


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    cases = []
    for key, suite in SUITES.items():
        root = capture / key
        assert json.loads((root / "run.json").read_text()) == {"exit": 0, "failure": None}
        events = module("cli-scoreboard").events(root / "events.jsonl")
        assert all(events.get(t) == "pass" for t in suite["tests"]), events
        rows = [json.loads(p.read_text()) for p in sorted((root / "calls").glob("*.json"))]
        assert collections.Counter(r["test"].split("/")[0] for r in rows) == suite["tests"]
        assert all(r["package"] == suite["package"] and "request" in r and "got" in r for r in rows)
        assert len({r["name"] for r in rows}) == len(rows)
        assert all(set(r["got"]) == {"output", "error"} for r in rows)
        counts = collections.Counter(r["test"] for r in rows)
        assert {r["name"] for r in rows} == {f"{t}#{i}" for t, n in counts.items() for i in range(n)}
        cases.extend(rows)
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    capture.mkdir(exist_ok=False)
    pinned = references.verify_source()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    for key, suite in SUITES.items():
        root = capture / key
        (root / "calls").mkdir(parents=True)
        test = references.TRACE_CHECKOUT / suite["package"] / suite["file"]
        original = (pinned / suite["package"] / suite["file"]).read_bytes()
        helper = test.parent / "d2rs_renderer_contract_test.go"
        assert test.read_bytes() == original and not helper.exists()
        text = transform(original.decode(), suite["replacements"])
        hook = TEMPLATE.read_text().replace("PACKAGE_PATH", suite["package"])
        hook = hook.replace("PACKAGE", Path(suite["package"]).name)
        hook = hook.replace("EXTRA_IMPORTS", suite["imports"]).replace("WRAPPERS", suite["wrappers"])
        env = dict(os.environ, D2RS_RENDERER_CONTRACT_DUMP=str(root / "calls"),
                   GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
                   GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off")
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        try:
            test.write_text(text)
            helper.write_text(hook)
            run = module("cli-scoreboard").bounded_run([
                "go", "test", "./" + suite["package"], "-json", "-count=1", "-parallel=1",
                "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                "-run", "^(" + "|".join(suite["tests"]) + ")$",
            ], env, capture, root / "events.jsonl", 300)
        finally:
            test.write_bytes(original)
            helper.unlink(missing_ok=True)
        (root / "run.json").write_text(json.dumps(run) + "\n")
        assert run == {"exit": 0, "failure": None}, (key, run)
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def score(capture, runner, output):
    assert not output.exists()
    cases = verify(capture)
    work = output.with_suffix("")
    work.mkdir(exist_ok=False)
    request = work / "requests.json"
    request.write_text(json.dumps([c["request"] for c in cases]))
    runner_hash = sha(runner)
    log = work / "responses.json"
    run = module("cli-scoreboard").bounded_run([str(runner), str(request)], os.environ.copy(), work, log, 300)
    rows = []
    try:
        actual = json.loads(log.read_text()) if run == {"exit": 0, "failure": None} else None
    except (ValueError, UnicodeError):
        actual = {}
    assert sha(runner) == runner_hash
    for i, case in enumerate(cases):
        row = {k: case[k] for k in ["name", "test", "package"]}
        if actual is None:
            row["outcome"] = run["failure"] or "rust-failed"
        elif not isinstance(actual, list) or len(actual) != len(cases):
            row["outcome"] = "invalid-inventory"
        else:
            row["outcome"] = "match" if module("language-scoreboard").same_json(actual[i], case["got"]) else "mismatch"
            if row["outcome"] != "match":
                row.update(expected=case["got"], actual=actual[i])
        rows.append(row)
    report = {"source_commit": references.SOURCE_COMMIT, "capture": str(capture),
              "capture_hashes": hashes(capture), "runner": str(runner), "runner_sha256": runner_hash,
              "summary": dict(collections.Counter(r["outcome"] for r in rows)), "rows": rows}
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not args.reuse:
        capture_original(args.capture.resolve())
    if args.output:
        assert args.rust
        score(args.capture.resolve(), args.rust.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
