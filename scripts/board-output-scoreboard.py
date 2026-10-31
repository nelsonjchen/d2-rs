#!/usr/bin/env python3
"""Original board publication tests through both CLI implementations.

All original assertions run. Compare complete file inventories and bytes,
including ownership manifests; no Unicode/path/hash normalization. Captures
have a 1 GiB total / 128 MiB file / 20 GiB free streaming guard and deadlines.
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
PACKAGE = "d2cli"
TESTS = {
    "TestMultiboardOutputContainsUnsafeNames": 1,
    "TestMultiboardOutputHashesOverlongBoardName": 1,
    "TestMultiboardOutputPreservesPreexistingDirectory": 2,
    "TestMultiboardOutputRemovesOnlyUnmodifiedStaleGeneratedFiles": 2,
    "TestMultiboardOutputHandlesEquivalentPathSpellingChanges": 4,
}


def load_module(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "scripts" / (name + ".py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_files(row):
    files = {
        name: base64.b64decode(data, validate=True)
        for name, data in row["files"].items()
    }
    owned = {
        name: base64.b64decode(data, validate=True)
        for name, data in row["owned"].items()
    }
    assert not row["ownedErrors"], row["ownedErrors"]
    assert files and all(
        not Path(p).is_absolute() and ".." not in Path(p).parts for p in files
    )
    assert not row["error"], row["error"]
    manifests = [p for p in files if p.endswith("/.d2-board-output-manifest-v1.json")]
    assert manifests, row["name"]
    for name in manifests:
        manifest = json.loads(files[name])
        assert manifest["format"] == "d2-board-output" and manifest["version"] == 1
        assert manifest["files"], row["name"]
        for entry in manifest["files"]:
            path = str(Path(name).parent / entry["path"])
            assert owned.get(path), (row["name"], path, "missing/empty owned output")
            assert hashlib.sha256(owned[path]).hexdigest() == entry["sha256"], (
                row["name"],
                path,
                "wrong owned hash",
            )
    return files, owned


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    capture = args.capture.resolve()
    capture.mkdir(exist_ok=False)
    source = references.verify_source()
    path = references.TRACE_CHECKOUT / "d2cli/board_output_test.go"
    original = (source / "d2cli/board_output_test.go").read_bytes()
    helper = path.parent / "d2rs_board_output_test.go"
    assert path.read_bytes() == original and not helper.exists()
    runner = args.rust.resolve()
    runner_hash = sha(runner)
    hook = ROOT / "tools/board-output-scoreboard/hook_test.go"
    runs = {}
    env = dict(
        os.environ,
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    env.pop("D2RS_BOARD_BINARY", None)
    try:
        transformed = original.decode().replace(
            "func runBoardOutputCLIResult(", "func d2rsOriginalBoardOutputCLIResult("
        )
        path.write_text(transformed)
        helper.write_bytes(hook.read_bytes())
        for mode in ["go", "rust"]:
            work = capture / mode
            work.mkdir()
            (work / "calls").mkdir()
            (work / "tmp").mkdir()
            env["TMPDIR"] = str(work / "tmp")
            env["D2RS_BOARD_CAPTURE"] = str(work / "calls")
            if mode == "rust":
                env["D2RS_BOARD_BINARY"] = str(runner)
            runs[mode] = load_module("cli-scoreboard").bounded_run(
                [
                    "go",
                    "test",
                    "./d2cli",
                    "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                    "-run",
                    "^(" + "|".join(TESTS) + ")$",
                    "-json",
                    "-count=1",
                    "-parallel=1",
                ],
                env,
                capture,
                work / "events.jsonl",
                600,
            )
    finally:
        path.write_bytes(original)
        helper.unlink(missing_ok=True)
    assert sha(runner) == runner_hash
    assert runs["go"] == {"exit": 0, "failure": None}, runs["go"]
    responses = {
        mode: {
            r["name"]: r
            for p in (capture / mode / "calls").glob("*.json")
            for r in [json.loads(p.read_text())]
        }
        for mode in runs
    }
    events = {
        mode: load_module("cli-scoreboard").events(capture / mode / "events.jsonl")
        for mode in runs
    }
    rows = []
    for test, count in TESTS.items():
        go = {
            k: v
            for k, v in responses["go"].items()
            if k.split("/")[0].split("#")[0] == test
        }
        native = {
            k: v
            for k, v in responses["rust"].items()
            if k.split("/")[0].split("#")[0] == test
        }
        row = {
            "test": test,
            "go_calls": len(go),
            "native_calls": len(native),
            "go_assertions": events["go"].get(test),
            "native_assertions": events["rust"].get(test),
            "outcome": "mismatch",
        }
        assert len(go) == count and events["go"].get(test) == "pass", row
        for response in go.values():
            validate_files(response)
        try:
            assert runs["rust"]["failure"] is None, runs["rust"]
            assert native.keys() == go.keys() and events["rust"].get(test) == "pass", (
                "original assertions/inventory failed"
            )
            for name, expected in go.items():
                assert validate_files(native[name]) == validate_files(expected), (
                    name,
                    "complete artifacts differ",
                )
            row["outcome"] = "match"
        except AssertionError as error:
            row["difference"] = str(error)
        rows.append(row)
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "package": PACKAGE,
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "source_sha256": hashlib.sha256(original).hexdigest(),
        "hook_sha256": sha(hook),
        "controls": {
            "go_version_stamp": "v0.9.0 (explicit release ldflags)",
            "comparison": "complete file inventories, bytes and ownership hashes; no normalization",
            "total_byte_limit": 1 << 30,
            "file_byte_limit": 128 << 20,
            "minimum_free_bytes": 20 << 30,
            "deadline_seconds": 600,
        },
        "runs": runs,
        "rows": rows,
        "matches": sum(r["outcome"] == "match" for r in rows),
        "total": len(rows),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {k: report[k] for k in ["matches", "total", "runs", "rows"]}, indent=2
        )
    )


if __name__ == "__main__":
    main()
