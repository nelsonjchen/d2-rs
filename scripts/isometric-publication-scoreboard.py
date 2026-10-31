#!/usr/bin/env python3
"""Original isometric publication/atomic/failure-preservation assertions.

Both implementations use the same temporary paths. No error/path/byte
normalization. Streaming limits: 1 GiB total, 128 MiB/file, 20 GiB free;
600-second test deadline and 30 seconds per native invocation.
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
    "TestIsometricMultiboardPublicationIsTransactional",
    "TestIsometricImageAtomicReplacement",
    "TestIsometricSVGFailurePreservesOutput",
    "TestIsometricPagedFailurePreservesOutput",
]
COUNTS = dict(zip(TESTS, [3, 3, 6, 8]))


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
    assert set(expected) == set(TESTS), "incomplete original functions"
    assert not set(actual) - set(expected), "extra native functions"
    rows = []
    for name in TESTS:
        calls = expected[name]
        assert len(calls) == COUNTS[name], "incomplete original call inventory"
        names = sorted({c["name"] for c in calls})
        for leaf in names:
            indices = [c["call"] for c in calls if c["name"] == leaf]
            assert indices == list(range(len(indices))), "unordered/duplicate calls"
        assert all(c["name"] == name or c["name"].startswith(name + "/") for c in calls)
        assert all(c["before"] and c["after"] for c in calls)
        if name == TESTS[1]:
            assert [c["canceled"] for c in calls] == [True, False, False]
            assert [c["error"] for c in calls] == ["context canceled", "", ""]
        else:
            assert all(
                isinstance(c["source"], str)
                and isinstance(c["args"], list)
                and c["stdout"] == ""
                for c in calls
            )
            assert [c["failed"] for c in calls] == (
                [False, True, False] if name == TESTS[0] else [True] * COUNTS[name]
            )
        rows.append(
            {
                "test": name,
                "calls": len(calls),
                "outcome": "match" if actual.get(name) == calls else "mismatch",
            }
        )
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source-version",
        action="store_true",
        help="Use the unstamped pinned Go version; expose metadata differences",
    )
    parser.add_argument("--rust", required=True, type=Path)
    parser.add_argument("--native-test", required=True, type=Path)
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
    native_hash = sha(args.native_test.resolve())
    files = [
        "isometric_image_test.go",
        "isometric_integration_test.go",
        "isometric_svg_test.go",
        "isometric_paged_test.go",
    ]
    originals = {name: (source / "d2cli" / name).read_bytes() for name in files}
    targets = {name: references.TRACE_CHECKOUT / "d2cli" / name for name in files}
    helper = references.TRACE_CHECKOUT / "d2cli/d2rs_publication_test.go"
    hook = ROOT / "tools/isometric-publication-scoreboard/hook_test.go"
    assert (
        all(targets[n].read_bytes() == v for n, v in originals.items())
        and not helper.exists()
    )
    env = dict(
        os.environ,
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
        D2RS_PUBLICATION_WORK=str(work),
    )
    env.pop("D2RS_PUBLICATION_BINARY", None)
    env.pop("D2RS_PUBLICATION_NATIVE_TEST", None)
    runs, responses, events = {}, {}, {}
    cli = load_cli()
    try:
        for name, original in originals.items():
            transformed = (
                original.decode()
                .replace("func imageCLI(", "func d2rsOriginalImageCLI(")
                .replace("t.TempDir()", "d2rsPublicationTempDir(t)")
            )
            if name == "isometric_image_test.go":
                assert transformed.count("atomicIsometricImage(ctx, path,") == 1
                transformed = transformed.replace(
                    "atomicIsometricImage(ctx, path,",
                    "d2rsPublicationAtomic(t, ctx, path,",
                ).replace(
                    "atomicIsometricImage(context.Background(),",
                    "d2rsPublicationAtomic(t, context.Background(),",
                )
            targets[name].write_text(transformed)
        helper.write_bytes(hook.read_bytes())
        for mode in ["go", "rust"]:
            output = capture / mode
            output.mkdir()
            (output / "calls").mkdir()
            (output / "tmp").mkdir()
            env["TMPDIR"] = str(output / "tmp")
            env["D2RS_PUBLICATION_CAPTURE"] = str(output / "calls")
            if mode == "rust":
                env["D2RS_PUBLICATION_BINARY"] = str(runner)
                env["D2RS_PUBLICATION_NATIVE_TEST"] = str(args.native_test.resolve())
            runs[mode] = cli.bounded_run(
                [
                    "go",
                    "test",
                    "./d2cli",
                    *(
                        []
                        if args.source_version
                        else [
                            "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0"
                        ]
                    ),
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
                if not p.name.startswith("atomic-")
            ]
            responses[mode] = {}
            for call in sorted(calls, key=lambda c: (c["name"], c["call"])):
                responses[mode].setdefault(call["name"].split("/")[0], []).append(call)
            events[mode] = cli.events(output / "events.jsonl")
    finally:
        for name, original in originals.items():
            targets[name].write_bytes(original)
        helper.unlink(missing_ok=True)
    assert runs["go"] == {"exit": 0, "failure": None}, runs
    assert all(events["go"].get(t) == "pass" for t in TESTS), events["go"]
    rows = compare(responses["go"], responses["rust"])
    for row in rows:
        row["go_assertions"] = events["go"].get(row["test"])
        row["native_assertions"] = events["rust"].get(row["test"])
        if runs["rust"]["failure"] is not None or row["native_assertions"] != "pass":
            row["outcome"] = "mismatch"
    assert sha(runner) == runner_hash
    assert sha(args.native_test.resolve()) == native_hash
    assert not list(work.iterdir()), "test cleanup left output paths"
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "package": "d2cli",
        "capture": str(capture),
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "sources_sha256": {
            n: hashlib.sha256(v).hexdigest() for n, v in originals.items()
        },
        "native_test_sha256": sha(args.native_test.resolve()),
        "hook_sha256": sha(hook),
        "go_version_stamp": (
            "pinned source (no ldflags override)"
            if args.source_version
            else "v0.9.0 (explicit release ldflags; complete artifacts compared)"
        ),
        "controls": {
            "total_bytes": 1 << 30,
            "file_bytes": 128 << 20,
            "minimum_free_bytes": 20 << 30,
            "deadline_seconds": 600,
            "comparison": "original failure/success contracts, complete stdout, before/after node inventories/modes/links/bytes at shared paths; atomic errors exact; no normalization",
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
