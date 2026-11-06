#!/usr/bin/env python3
"""Manifest ownership admission: original assertions and full error strings.

Original Go assertions stay active. Native replay receives input manifests only.
Captures use the shared streaming 1 GiB / 128 MiB file / 20 GiB free guard.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TEST = "TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims"


def load_cli():
    spec = importlib.util.spec_from_file_location(
        "cli", ROOT / "scripts/cli-scoreboard.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def compare(capture, actual):
    calls = capture["calls"]
    assert capture["originalCount"] == 3 and len(calls) == 16
    assert len(actual) == len(calls), "missing/extra native responses"
    return [
        {
            "case": str(i),
            "test": TEST if i < 3 else "finite-ownership-admission",
            "outcome": "match" if result == {"error": call["error"]} else "mismatch",
            "expected": {"error": call["error"]},
            "actual": result,
        }
        for i, (call, result) in enumerate(zip(calls, actual, strict=True))
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cli = load_cli()
    capture = args.capture.resolve()
    source = references.verify_source()
    hook = ROOT / "tools/board-manifest-scoreboard/hook_test.go"
    source_file = source / "d2cli/board_output_test.go"
    sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    binding = {
        "pin": references.SOURCE_COMMIT,
        "hook": sha(hook),
        "test": sha(source_file),
        "implementation": sha(source / "d2cli/board_output_publish.go"),
    }
    env = dict(
        os.environ,
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "tmp").mkdir()
        env.update(TMPDIR=str(capture / "tmp"), D2RS_MANIFEST_CAPTURE=str(capture))
        path = references.TRACE_CHECKOUT / "d2cli/board_output_test.go"
        helper = path.parent / "d2rs_manifest_test.go"
        original = source_file.read_bytes()
        assert path.read_bytes() == original and not helper.exists()
        try:
            path.write_text(
                original.decode().replace(
                    "validateBoardOutputManifest(", "d2rsValidateManifest("
                )
            )
            helper.write_bytes(hook.read_bytes())
            result = cli.bounded_run(
                [
                    "go",
                    "test",
                    "./d2cli",
                    "-run",
                    "^TestD2rsManifestCapture$",
                    "-json",
                    "-count=1",
                ],
                env,
                capture,
                capture / "go.jsonl",
                600,
            )
            assert result == {"exit": 0, "failure": None}, result
        finally:
            path.write_bytes(original)
            helper.unlink(missing_ok=True)
        events = cli.events(capture / "go.jsonl")
        assert events.get("TestD2rsManifestCapture/" + TEST) == "pass", events
        binding["calls"] = sha(capture / "calls.json")
        (capture / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    else:
        saved = json.loads((capture / "binding.json").read_text())
        assert saved == {**binding, "calls": sha(capture / "calls.json")}
    observed = json.loads((capture / "calls.json").read_text())
    work = args.output.resolve().with_suffix("")
    work.mkdir(exist_ok=False)
    (work / "tmp").mkdir()
    request = [{"manifest": call["manifest"]} for call in observed["calls"]]
    (work / "request.json").write_text(json.dumps(request) + "\n")
    env.update(
        TMPDIR=str(work / "tmp"),
        D2RS_MANIFEST_REQUEST=str(work / "request.json"),
        D2RS_MANIFEST_RESPONSE=str(work / "actual.json"),
    )
    cli.TRACE = ROOT
    runner = args.rust.resolve()
    runner_hash = sha(runner)
    result = cli.bounded_run(
        [
            str(runner),
            "--exact",
            "board_output::manifest_api_tests::replay_manifest_calls",
            "--ignored",
            "--nocapture",
        ],
        env,
        work,
        work / "native.log",
        60,
    )
    assert result == {"exit": 0, "failure": None}, result
    assert sha(runner) == runner_hash
    rows = compare(observed, json.loads((work / "actual.json").read_text()))
    # Credit the original function only when all three original calls match.
    original = {
        "test": TEST,
        "outcome": "match"
        if all(r["outcome"] == "match" for r in rows[:3])
        else "mismatch",
    }
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "package": "d2cli",
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "bindings": binding,
        "rows": [original],
        "calls": rows,
        "matches": sum(r["outcome"] == "match" for r in rows),
        "total": len(rows),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {"matches": report["matches"], "total": len(rows), "rows": [original]}
        )
    )


if __name__ == "__main__":
    main()
