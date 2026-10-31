#!/usr/bin/env python3
"""Observe six original SVG fragment/precision tests, preserving every assertion.

The Rust runner is a unit-test executable: private index/predicate code is tested
without expanding the renderer's public API or enabling production diagnostics.
"""

import argparse
import collections
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "precision_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestSVGFragmentIndexBoundsLiveReferences": 5,
        "TestSVGPredicatesRoundCancellationProducts": 3,
        "TestSVGIntersectionRoundsInterpolatedProducts": 1,
        "TestSVGPolygonPathCanonicalizesSharedVertices": 4,
        "TestSVGPolygonPathOmitsSubprecisionCollinearSplits": 21,
        "TestSVGPolygonPathKeepsRealCornersAndTriangles": 2,
    }
    score.OPS = {name: {"path": n} for name, n in score.TESTS.items()}
    score.OPS["TestSVGFragmentIndexBoundsLiveReferences"] = {
        "new": 1,
        "add": 3,
        "remove": 1,
    }
    score.OPS["TestSVGPredicatesRoundCancellationProducts"] = {
        "edge": 1,
        "area": 1,
        "depth": 1,
    }
    score.OPS["TestSVGIntersectionRoundsInterpolatedProducts"] = {"split_y": 1}
    score.COMPARISON_CONTRACT = {
        "compared": [
            "all original assertions and 36 original operation results",
            "complete fragment index, cells, live references, work and error text",
            "IEEE-754 result bits for cancellation predicates",
            "ordered split polygons and unchanged input coordinates",
            "complete SVG path bytes and unchanged source polygons",
        ],
        "unqualified": ["Go allocation addresses and map traversal order"],
        "original_go_assertions": "all retained and required to pass",
    }
    package = "d2renderers/d2isometric/d2isometricimg"
    test_files = [
        "svg_fragments_test.go",
        "svg_precision_test.go",
        "svg_polygon_precision_test.go",
    ]
    source_files = test_files + ["svg_fragments.go", "svg_visibility.go", "svg.go"]
    template = score.ROOT / "tools/isometric-precision-scoreboard/hook_test.go"

    def source():
        pinned = score.references.verify_source()
        return {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + p: score.sha(pinned / package / p) for p in source_files
            },
            "hook": score.sha(template),
        }

    score.source = source

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {folder / p: (folder / p).read_bytes() for p in test_files}
        hook = folder / "d2rs_precision_api_test.go"
        assert not hook.exists()
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            for name in score.TESTS:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                if not match:
                    continue
                body = match[0].replace("{\n", "{\n\td2rsPrecisionStart(t)\n", 1)
                for old, new in [
                    ("svgNewVisibleFragmentSet(", "d2rsPrecisionNew("),
                    ("s.add(small)", "d2rsPrecisionAdd(s, small)"),
                    (
                        "s.vertices -= len(small)",
                        "s.vertices -= len(small)\n\td2rsPrecisionRemoved(s, id)",
                    ),
                    ("svgPolygonPath(", "d2rsPrecisionPath(t, "),
                    ("svgEdgeDistance(", "d2rsPrecisionEdge("),
                    ("svgPolygonArea(", "d2rsPrecisionArea("),
                    (
                        "plane.at(svgPoint{x: b})",
                        "d2rsPrecisionDepth(plane, svgPoint{x: b})",
                    ),
                    ("svgSplitPolygon(", "d2rsPrecisionSplit("),
                ]:
                    body = body.replace(old, new)
                text = text[: match.start()] + body + text[match.end() :]
            replacements[path] = text
        (capture / "go").mkdir(parents=True)
        (capture / "tmp").mkdir()
        (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
        (capture / "inventory.json").write_text(json.dumps(score.TESTS) + "\n")
        env = dict(
            os.environ,
            TMPDIR=str(capture / "tmp"),
            D2RS_SCENE_API=str(capture / "go"),
            GOMODCACHE=str(score.ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(score.ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        try:
            for path, text in replacements.items():
                path.write_text(text)
            hook.write_bytes(template.read_bytes())
            result = score.module("cli-scoreboard").bounded_run(
                [
                    "go",
                    "test",
                    "./" + package,
                    "-run",
                    "^(" + "|".join(score.TESTS) + ")$",
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
            for path, original in originals.items():
                path.write_bytes(original)
            hook.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(result) + "\n")
        assert result == {"exit": 0, "failure": None}, result
        (capture / "hashes.json").write_text(
            json.dumps(score.hashes(capture), indent=2) + "\n"
        )
        score.verify(capture)

    score.capture_original = capture_original
    return score


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    score = scoreboard()
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists()
        score.capture_original(capture)
    cases = score.verify(capture)
    if args.output is None:
        print("verified original functions", len(cases))
        return
    assert args.rust and args.rust.is_file() and not args.output.exists()
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, actual = work / "request.json", work / "actual.json"
    request.write_text(json.dumps({"cases": cases}) + "\n")
    digest = score.sha(args.rust)
    run = score.module("cli-scoreboard").bounded_run(
        [
            str(args.rust.resolve()),
            "--exact",
            "img::svg_visibility::source_contracts::replay_original_calls",
            "--nocapture",
        ],
        dict(
            os.environ,
            D2RS_PRECISION_REQUEST=str(request.resolve()),
            D2RS_PRECISION_OUTPUT=str(actual.resolve()),
        ),
        work,
        work / "run.log",
        60,
    )
    output = (
        json.loads(actual.read_text())
        if run == {"exit": 0, "failure": None} and actual.is_file()
        else None
    )
    rows = []
    assert output is None or len(output) == len(cases)
    for i, case in enumerate(cases):
        expected = [op["got"] for op in case["operations"]]
        got = output[i] if output is not None else None
        result = score.outcome(expected, got)
        row = {
            "name": case["name"],
            "outcome": result,
            "input_sha256": score.module("imageasset-scoreboard").value_sha(
                [op["request"] for op in case["operations"]]
            ),
            "expected_sha256": score.module("imageasset-scoreboard").value_sha(
                expected
            ),
        }
        if result == "mismatch":
            row["differences"] = score.module("language-scoreboard").differences(
                expected, got
            )
        rows.append(row)
    assert score.sha(args.rust) == digest
    report = {
        "source_commit": score.references.SOURCE_COMMIT,
        "package": "d2renderers/d2isometric/d2isometricimg",
        "captured_functions": len(cases),
        "observed_operations": sum(score.TESTS.values()),
        "comparison_contract": score.COMPARISON_CONTRACT,
        "runner_sha256": digest,
        "capture_hashes": score.hashes(capture),
        "run": run,
        "summary": dict(collections.Counter(row["outcome"] for row in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
