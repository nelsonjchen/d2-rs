#!/usr/bin/env python3
"""Run original publication callbacks/assertions through native API transport.

Complete errors, touched state, callback order, node inventories, permissions,
links and bytes compare at identical paths. Streaming guard: 1 GiB total,
128 MiB/file, 20 GiB free; 600-second suites, 60-second native invocations.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TESTS = [
    "TestBoardOutputPublicationRollsBackWholeTransaction",
    "TestBoardOutputPublicationRejectsAncestorSwapWithoutEscapingRoot",
    "TestBoardOutputPublicationPreservesStaleFileEditedDuringPublish",
]


def load_cli():
    spec = importlib.util.spec_from_file_location(
        "cli", ROOT / "scripts/cli-scoreboard.py"
    )
    cli = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cli)
    return cli


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare(expected, actual):
    assert set(expected) == set(TESTS), "incomplete original test inventory"
    assert not (set(actual) - set(expected)), "extra native results"
    rows = []
    for test in TESTS:
        go = expected[test]
        assert go["name"] == test and go["after"] and go["callbacks"]
        assert not any(
            ".d2-board-" in p for p in go["after"] if not p.endswith(".json")
        ), "leftover staging output"
        assert go["touched"] == (test == TESTS[2])
        assert bool(go["error"]) == (test != TESTS[2])
        rows.append(
            {
                "test": test,
                "outcome": "match" if actual.get(test) == go else "mismatch",
                "expected": go,
                "actual": actual.get(test),
            }
        )
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source = references.verify_source()
    capture = args.capture.resolve()
    capture.mkdir(exist_ok=False)
    work = capture / "shared-work"
    work.mkdir()
    runner = args.rust.resolve()
    runner_hash = sha(runner)
    original = (source / "d2cli/board_output_test.go").read_bytes()
    target = references.TRACE_CHECKOUT / "d2cli/board_output_test.go"
    helper = target.with_name("d2rs_transaction_test.go")
    hook = ROOT / "tools/board-transaction-scoreboard/hook_test.go"
    assert target.read_bytes() == original and not helper.exists()
    env = dict(
        os.environ,
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
        D2RS_TRANSACTION_WORK=str(work),
    )
    env.pop("D2RS_TRANSACTION_BINARY", None)
    runs, responses, events = {}, {}, {}
    cli = load_cli()
    try:
        transformed = (
            original.decode()
            .replace("t.TempDir()", "d2rsTransactionTempDir(t)")
            .replace(
                "workspace.publish()", "d2rsTransactionPublish(t, directory, workspace)"
            )
        )
        assert transformed.count("d2rsTransactionPublish(t, directory, workspace)") == 3
        target.write_text(transformed)
        helper.write_bytes(hook.read_bytes())
        for mode in ["go", "rust"]:
            output = capture / mode
            output.mkdir()
            (output / "calls").mkdir()
            (output / "tmp").mkdir()
            env.update(
                TMPDIR=str(output / "tmp"),
                D2RS_TRANSACTION_CAPTURE=str(output / "calls"),
            )
            if mode == "rust":
                env["D2RS_TRANSACTION_BINARY"] = str(runner)
            runs[mode] = cli.bounded_run(
                [
                    "go",
                    "test",
                    "./d2cli",
                    "-run",
                    "^(" + "|".join(TESTS) + ")$",
                    "-json",
                    "-count=1",
                    "-parallel=1",
                ],
                env,
                capture,
                output / "events.jsonl",
                600,
            )
            calls = [
                json.loads(p.read_text())
                for p in (output / "calls").glob("*.json")
                if not p.stem.endswith(("-request", "-response"))
            ]
            assert len(calls) == len({r["name"] for r in calls}), (
                "duplicate observations"
            )
            responses[mode] = {r["name"]: r for r in calls}
            events[mode] = cli.events(output / "events.jsonl")
    finally:
        target.write_bytes(original)
        helper.unlink(missing_ok=True)
    assert runs["go"] == {"exit": 0, "failure": None}, runs
    assert all(events["go"].get(t) == "pass" for t in TESTS), events["go"]
    rows = compare(responses["go"], responses["rust"])
    for row in rows:
        row["go_assertions"] = events["go"].get(row["test"])
        row["native_assertions"] = events["rust"].get(row["test"])
        if runs["rust"]["failure"] or row["native_assertions"] != "pass":
            row["outcome"] = "mismatch"
    assert sha(runner) == runner_hash
    assert not list(work.iterdir()), "test cleanup left output paths"
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "package": "d2cli",
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "source_sha256": hashlib.sha256(original).hexdigest(),
        "hook_sha256": sha(hook),
        "controls": {
            "total_bytes": 1 << 30,
            "file_bytes": 128 << 20,
            "minimum_free_bytes": 20 << 30,
            "deadline_seconds": 600,
            "comparison": "complete errors, touched state, callback order and node inventories/modes/links/bytes; no normalization",
        },
        "runs": runs,
        "rows": rows,
        "matches": sum(r["outcome"] == "match" for r in rows),
        "total": len(rows),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "matches": report["matches"],
                "total": len(rows),
                "runs": runs,
                "rows": [
                    {k: v for k, v in r.items() if k not in {"expected", "actual"}}
                    for r in rows
                ],
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
