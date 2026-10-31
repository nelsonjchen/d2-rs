#!/usr/bin/env python3
"""Original local imgbundler policy calls against native filesystem APIs.

Five original test functions, all original assertions retained. Compare complete
output and error strings. Rebuild actual files/symlinks from captured inputs;
never replace policy decisions with mocked bytes. Sparse zero fixtures are
verified by SHA256. Streaming limits: 1 GiB total, 128 MiB/file, 20 GiB free.
"""

import argparse
import base64
import collections
import hashlib
import json
import os
import re
import shutil
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
FILES = ["lib/imgbundler/local_policy_test.go"]
TESTS = {
    "TestBundleLocalDeniesByDefault": 1,
    "TestBundleLocalRootedPolicy": 3,
    "TestBundleLocalRootedPolicyRejectsSymlinkEscape": 1,
    "TestBundleLocalCacheDoesNotCrossRootReplacement": 2,
    "TestBundleLocalCapsLocalFileBytes": 1,
}


def module(name):
    spec = spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    value = module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source():
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    paths = FILES + [
        "lib/imgbundler/imgbundler.go",
        "lib/localfile/localfile.go",
        "lib/localfile/open_unix.go",
    ]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: sha(pinned / p) for p in paths},
        "hook": sha(ROOT / "tools/imgbundler-policy-scoreboard/hook_test.go"),
    }


def hashes(capture):
    paths = [
        capture / n for n in ["source.json", "inventory.json", "go.jsonl", "run.json"]
    ]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def expected():
    return {f"{t}#{i}" for t, count in TESTS.items() for i in range(count)}


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    assert len(cases) == len(expected()) and {c["name"] for c in cases} == expected()
    assert all(events.get(t) == "pass" for t in TESTS), events
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    original = (pinned / FILES[0]).read_bytes()
    path = references.TRACE_CHECKOUT / FILES[0]
    helper = references.TRACE_CHECKOUT / "lib/imgbundler/d2rs_imgbundler_policy_test.go"
    assert not helper.exists() and path.read_bytes() == original
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "fs").mkdir()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS, indent=2) + "\n")
    try:
        parts = re.split(r"(?=^func Test)", original.decode(), flags=re.MULTILINE)
        for i, part in enumerate(parts):
            if any(part.startswith("func " + t + "(") for t in TESTS):
                part = part.replace(
                    "BundleLocalWithPolicy(", "d2rsBundleLocalWithPolicy(t, "
                )
                parts[i] = part.replace("BundleLocal(", "d2rsBundleLocal(t, ")
        path.write_text("".join(parts))
        helper.write_bytes(
            (ROOT / "tools/imgbundler-policy-scoreboard/hook_test.go").read_bytes()
        )
        env = dict(
            os.environ,
            D2RS_IMGBUNDLER_POLICY_DUMP=str(capture / "go"),
            TMPDIR=str(capture / "fs"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./lib/imgbundler",
                "-run",
                "^(" + "|".join(sorted(TESTS)) + ")$",
                "-json",
                "-count=1",
                "-parallel=1",
            ],
            env,
            capture,
            capture / "go.jsonl",
            600,
        )
    finally:
        path.write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def materialize(capture, request):
    fixture = Path(request["fixture"])
    # Only recreate the test-owned temporary directory, and refuse preexisting
    # data. Every output writer gets fresh files; content stores are untouched.
    assert (
        fixture.is_absolute()
        and ".." not in fixture.parts
        and fixture.is_relative_to(capture / "fs")
    )
    assert (
        fixture != capture / "fs" and not fixture.exists() and not fixture.is_symlink()
    )
    fixture.mkdir(parents=True)
    for name, entry in sorted(
        request["entries"].items(), key=lambda item: len(Path(item[0]).parts)
    ):
        path = fixture / name
        assert not Path(name).is_absolute() and ".." not in Path(name).parts
        path.parent.mkdir(parents=True, exist_ok=True)
        if entry.get("directory"):
            path.mkdir(exist_ok=True)
        elif "link" in entry:
            path.symlink_to(entry["link"])
        elif "zero_bytes" in entry:
            assert 0 <= entry["zero_bytes"] <= 128 << 20
            with path.open("xb") as stream:
                stream.truncate(entry["zero_bytes"])
            assert sha(path) == entry["sha256"]
        else:
            path.write_bytes(base64.b64decode(entry["body"], validate=True))
    return fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument(
        "--batch",
        action="store_true",
        help="Replay consecutive document invocations in one native process",
    )
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    if not args.output:
        return
    assert args.rust
    cases = verify(capture)
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    cli = module("cli-scoreboard")
    runner_hash = sha(args.rust)
    rows = []
    cases.sort(key=lambda c: c["name"])
    batch_actual = None
    if args.batch:
        request, log = work / "batch.request.json", work / "batch.log"
        request.write_text(json.dumps([c["request"] for c in cases]))
        # Check each complete input before native materialization. Native creates
        # and removes the same real files consecutively, preserving cache state.
        for case in cases:
            fixture = materialize(capture, case["request"])
            shutil.rmtree(fixture)
        run = cli.bounded_run(
            [str(args.rust.resolve()), str(request.resolve())],
            dict(os.environ, D2RS_FIXTURE_ROOT=str(capture / "fs")),
            work,
            log,
            60,
        )
        assert run == {"exit": 0, "failure": None}, run
        batch_actual = json.loads(log.read_text())
        assert isinstance(batch_actual, list) and len(batch_actual) == len(cases)
    for index, case in enumerate(cases):
        key = hashlib.sha256(case["name"].encode()).hexdigest()
        request, log = work / (key + ".request.json"), work / (key + ".log")
        request.write_text(json.dumps(case["request"]))
        if args.batch:
            run = {"exit": 0, "failure": None}
            log.write_text(json.dumps(batch_actual[index]) + "\n")
        else:
            fixture = materialize(capture, case["request"])
            try:
                run = cli.bounded_run(
                    [str(args.rust.resolve()), str(request.resolve())],
                    os.environ.copy(),
                    work,
                    log,
                    30,
                )
            finally:
                shutil.rmtree(fixture)
        row = {"name": case["name"], "test": case["test"]}
        if run["failure"] or run["exit"]:
            row["outcome"] = run["failure"] or "rust-failed"
        else:
            actual = json.loads(log.read_text())
            row["outcome"] = (
                "unsupported"
                if "unsupported" in actual
                else "match"
                if actual == case["got"]
                else "mismatch"
            )
            if row["outcome"] == "mismatch":
                row["differences"] = module("language-scoreboard").differences(
                    case["got"], actual
                )
        rows.append(row)
    assert sha(args.rust) == runner_hash
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner_sha256": runner_hash,
        "capture_hashes": hashes(capture),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "captured_functions": len(TESTS),
        "same_native_process": args.batch,
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
