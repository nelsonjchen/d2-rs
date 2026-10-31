#!/usr/bin/env python3
"""Original upstream test-call observations for crates/d2-rs-cli and crates/d2-rs-lib.

usage: cli_go_api_fixtures.py <family>

Each family runs pinned upstream Go tests with their API calls routed through
an env-gated observer hook. The rewritten tests and hooks reach the runner's
patched upstream tree ($D2RS_UPSTREAM) only as `go -overlay` entries
(scripts/upstream_overlay.py); the original assertions stay active, so a
failing upstream test fails the recipe.

  board-manifest     d2cli TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims plus the
                     tools/board-manifest-scoreboard admission grid
  board-path         d2cli board name, collision and append-path tests (tools/board-path-scoreboard)
  renderer-contract  scripts/renderer-contract-scoreboard.py capture
  library-config     scripts/library-api-scoreboard.py capture, configuration calls only
  library-source     scripts/library-source-scoreboard.py capture
  master-urlenc      scripts/language-remaining-master-scoreboard.py capture, lib/urlenc calls only

Run through `uv run --no-project python scripts/regen-fixtures.py --only <recipe>`
(docs/FIXTURES.md).
"""
from __future__ import annotations

import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402
import upstream_overlay  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
PIN = json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]


def load(name: str):
    path = ROOT / "scripts" / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def go_test(package: str, tests, env: dict, log: Path):
    """`go test` in the upstream tree; the redirected Popen adds the live overlay."""
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    environment = dict(os.environ, **env)
    environment.pop("TA", None)
    environment.pop("TESTDATA_ACCEPT", None)
    with log.open("wb") as stream:
        process = subprocess.run(["go", "test", "./" + package, "-run", "^(" + "|".join(tests) + ")$", "-json",
                                  "-count=1", "-parallel=1"], cwd=upstream, env=environment, stdout=stream,
                                 stderr=subprocess.STDOUT, timeout=900)
    events = {}
    for line in log.read_text().splitlines():
        if line.startswith("{"):
            event = json.loads(line)
            if event.get("Test") and event.get("Action") in ("pass", "fail", "skip"):
                events[event["Test"]] = event["Action"]
    if process.returncode != 0 or any(v == "fail" for v in events.values()):
        sys.exit(f"go test ./{package} failed; see {log}:\n" + log.read_text()[-4000:])
    return events


def canonical_tempdirs(value, tmp: Path):
    """Replace t.TempDir() paths (random suffixes) under the capture's TMPDIR
    with fixed, nonexistent /tmp/d2rs-oracle/<test> paths, consistently in
    requests and results."""
    pattern = re.compile(re.escape(str(tmp)) + r"/([A-Za-z_]+?)[0-9]+/")
    if isinstance(value, str):
        return pattern.sub(r"/tmp/d2rs-oracle/\1/", value)
    if isinstance(value, list):
        return [canonical_tempdirs(v, tmp) for v in value]
    if isinstance(value, dict):
        return {k: canonical_tempdirs(v, tmp) for k, v in value.items()}
    return value


def write(logical: str, value, **kwargs):
    fixture_recipe.output(logical).write_text(json.dumps(value, **kwargs) + "\n")


def board_manifest(work: Path, upstream: Path):
    test = upstream / "d2cli/board_output_test.go"
    original = test.read_text()
    test.write_text(original.replace("validateBoardOutputManifest(", "d2rsValidateManifest("))
    (upstream / "d2cli/d2rs_manifest_test.go").write_bytes((ROOT / "tools/board-manifest-scoreboard/hook_test.go").read_bytes())
    capture = work / "capture"
    (capture / "tmp").mkdir(parents=True)
    events = go_test("d2cli", ["TestD2rsManifestCapture"], {"D2RS_MANIFEST_CAPTURE": str(capture),
                                                           "TMPDIR": str(capture / "tmp")}, work / "go.jsonl")
    if events.get("TestD2rsManifestCapture/TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims") != "pass":
        sys.exit(f"original manifest test did not pass: {events}")
    calls = json.loads((capture / "calls.json").read_text())
    if calls["originalCount"] != 3 or len(calls["calls"]) != 16:
        sys.exit(f"expected 3 original and 16 total manifest calls: {calls['originalCount']} {len(calls['calls'])}")
    write("crates/d2-rs-cli/tests/fixtures/board-manifest-api.json", calls, separators=(",", ":"), ensure_ascii=False)


def board_path(work: Path, upstream: Path):
    score = load("board-path-scoreboard")
    test = upstream / "d2cli/board_output_test.go"
    text = test.read_text()
    assert text.count("got := boardOutputComponent(name)") == 2
    assert text.count("got != boardOutputComponent(name)") == 1
    assert text.count("err := validateBoardOutputPaths(") == 2
    text = text.replace("got := boardOutputComponent(name)", "got := d2rsPathComponent(t, name)")
    text = text.replace("got != boardOutputComponent(name)", "got != d2rsPathComponent(t, name)")
    text = text.replace("err := validateBoardOutputPaths(", "err := d2rsPathValidate(t, ")
    for name, wrapper in [("appendBoardOutputName", "d2rsPathAppendName"),
                          ("appendBoardOutputComponent", "d2rsPathAppendComponent"),
                          ("ensurePathWithin", "d2rsPathWithin")]:
        assert text.count(":= " + name + "(") == 1
        text = text.replace(":= " + name + "(", ":= " + wrapper + "(t, ")
    test.write_text(text)
    (upstream / "d2cli/d2rs_board_paths_test.go").write_bytes((ROOT / "tools/board-path-scoreboard/hook_test.go").read_bytes())
    capture = work / "capture"
    (capture / "tmp").mkdir(parents=True)
    events = go_test("d2cli", score.TESTS, {"D2RS_PATH_CAPTURE": str(capture), "TMPDIR": str(capture / "tmp")},
                     work / "go.jsonl")
    if not all(events.get(t) == "pass" for t in score.TESTS):
        sys.exit(f"original board path tests did not pass: {events}")
    calls = json.loads((capture / "calls.json").read_text())
    score.validate_calls(calls)
    calls = canonical_tempdirs(calls, capture / "tmp")
    write("crates/d2-rs-cli/tests/fixtures/board-path-api.json",
          {"source": "d2cli/board_output_test.go", "pin": PIN, "calls": calls}, indent=2, ensure_ascii=False)


def scoreboard_cases(name: str, work: Path, attribute: str | None = None):
    score = load(name)
    if attribute:
        score = getattr(score, attribute)
    capture = work / "capture"
    if name != "renderer-contract-scoreboard":
        capture.mkdir(parents=True)
    score.capture_original(capture)
    return score.verify(capture)


def renderer_contract(work: Path, upstream: Path):
    cases = scoreboard_cases("renderer-contract-scoreboard", work)
    write("crates/d2-rs-cli/tests/fixtures/renderer-contract-api.json", cases, indent=2, sort_keys=True)


def library_config(work: Path, upstream: Path):
    keep = {"TestApplyConfigsThemeOverridePrecedence", "TestIsometricConfigUsesSketchPrecedence"}
    cases = [c for c in scoreboard_cases("library-api-scoreboard", work) if c["test"].split("/")[0] in keep]
    if collections.Counter(c["test"].split("/")[0] for c in cases) != {"TestApplyConfigsThemeOverridePrecedence": 2,
                                                                       "TestIsometricConfigUsesSketchPrecedence": 8}:
        sys.exit("unexpected configuration call inventory")
    write("crates/d2-rs-lib/tests/fixtures/library-config-api.json", sorted(cases, key=lambda c: c["name"]),
          indent=2, sort_keys=True)


def library_source(work: Path, upstream: Path):
    cases = canonical_tempdirs(scoreboard_cases("library-source-scoreboard", work, "library"), work / "capture" / "tmp")
    write("crates/d2-rs-lib/tests/fixtures/library-source-api.json", sorted(cases, key=lambda c: c["name"]),
          indent=2, sort_keys=True)


def master_urlenc(work: Path, upstream: Path):
    cases = [c for c in scoreboard_cases("language-remaining-master-scoreboard", work)
             if c["request"]["op"] in ("encode", "decode")]
    rows = []
    for case in sorted(cases, key=lambda c: c["name"]):
        request = dict(case["request"])
        if request["op"] == "encode":
            # The oversized script is one repeated character; store it compactly.
            raw = request.pop("input")
            unit = raw[:1]
            if not raw or unit * len(raw) != raw:
                sys.exit("encode input is not a repeated character")
            request.update(repeat=unit, count=len(raw), input_sha256=hashlib.sha256(raw.encode()).hexdigest())
        rows.append({"got": case["got"], "name": case["name"], "request": request, "test": case["test"]})
    write("crates/d2-rs-lib/tests/fixtures/master-urlenc.json", {"source_commit": PIN, "cases": rows},
          indent=2, sort_keys=True)


FAMILIES = {
    "board-manifest": board_manifest, "board-path": board_path, "renderer-contract": renderer_contract,
    "library-config": library_config, "library-source": library_source, "master-urlenc": master_urlenc,
}

if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in FAMILIES:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(FAMILIES)}}}")
    work = fixture_recipe.workdir()
    upstream = upstream_overlay.redirect(work / "overlay")
    FAMILIES[sys.argv[1]](work, upstream)
