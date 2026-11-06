#!/usr/bin/env python3
"""Observe twelve original header and crossing tests, preserving every assertion.

The Rust runner is a unit-test executable: private index/predicate code is tested
without enabling production diagnostics.
"""

import argparse
import collections
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard(
    *,
    tests=None,
    files=None,
    sources=None,
    template=None,
    call_replacements=None,
    helper_templates=(),
    helper_functions=(),
):
    spec = spec_from_file_location(
        "geometry_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestHierarchyHeaderSourcePositionsAndOwnership": 6,
        "TestHierarchyHeaderAvoidsNestedRaisedSilhouette": 2,
        "TestHierarchyHeaderVerticalStripDeterminismAndBlockedFallback": 3,
        "TestHierarchyHeaderFallsBackToClearBottomStrip": 1,
        "TestHierarchyHeaderTinySurfaceAndIconPrintArea": 2,
        "TestHierarchyHeaderCompiledWorkspaceMargin": 1,
        "TestNativeBridgeCrossingClearanceAndOwnership": 3,
        "TestNativeBridgeOnlyTrueCrossings": 7,
        "TestNativeBridgeVisibilityAndDash": 5,
        "TestNativeBridgeStableOrderAndNearbyPlateau": 4,
        "TestNativeBridgeMultiwayAndRemoteSharedSource": 2,
        "TestNativeBridgeValidationCancellationAndBudget": 7,
    }
    if tests is not None:
        score.TESTS = tests
    score.OPS = {
        name: {"header" if "Header" in name else "bridge": n}
        for name, n in score.TESTS.items()
    }
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete ordered paths, placement and errors",
            "all supplied coordinates and source fields before/after",
            "original Go assertions retained",
        ],
        "unqualified": ["Go pointer addresses, timing and allocation counts"],
        "original_go_assertions": "all retained and required to pass",
    }
    package = "d2renderers/d2isometric/d2isometricimg"
    test_files = [
        "hierarchy_headers_test.go",
        "route_bridges_test.go",
    ]
    if files is not None:
        test_files = files
    source_files = test_files + [
        "hierarchy_headers.go",
        "route_bridges.go",
        "geometry_routes.go",
    ]
    if sources is not None:
        source_files = test_files + sources
    template = (
        template or score.ROOT / "tools/isometric-geometry-scoreboard/hook_test.go"
    )

    def source():
        pinned = score.references.verify_source()
        result = {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + p: score.sha(pinned / package / p) for p in source_files
            },
            "hook": score.sha(template),
        }
        if helper_templates:
            result["helpers"] = {
                str(p.relative_to(score.ROOT)): score.sha(p) for p in helper_templates
            }
        return result

    score.source = source

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {folder / p: (folder / p).read_bytes() for p in test_files}
        hook = folder / "d2rs_geometry_api_test.go"
        assert not hook.exists()
        helpers = {
            folder / f"d2rs_geometry_helper_{i}_test.go": p
            for i, p in enumerate(helper_templates)
        }
        assert all(not p.exists() for p in helpers)
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            for name in [*score.TESTS, *helper_functions]:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                if not match:
                    continue
                body = match[0]
                if name in score.TESTS:
                    body = body.replace("{\n", "{\n\td2rsGeometryStart(t)\n", 1)
                for old, new in call_replacements or [
                    ("svgVisibleFaces(", "d2rsGeometryVisibility("),
                    ("svgVisibleFacesWithLimits(", "d2rsGeometryVisibilityLimits("),
                    ("svgVisibilityWorkLimit(", "d2rsGeometryWork("),
                    ("hierarchyBoardHeaderSurface(", "d2rsGeometryHeader("),
                    ("hierarchyBoardHeaderPlacement(", "d2rsGeometryPlacement("),
                    ("nativeBridgeRoutes(", "d2rsGeometryBridge("),
                    ("nativeResolveBridgeRoutes(", "d2rsGeometryBridgeLimit("),
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
            for path, helper in helpers.items():
                path.write_bytes(helper.read_bytes())
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
            for path in helpers:
                path.unlink(missing_ok=True)
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
    request.write_text(
        json.dumps(
            {
                "cases": [
                    {
                        "name": case["name"],
                        "operations": [
                            {"request": op["request"]} for op in case["operations"]
                        ],
                    }
                    for case in cases
                ]
            }
        )
        + "\n"
    )
    digest = score.sha(args.rust)
    run = score.module("cli-scoreboard").bounded_run(
        [
            str(args.rust.resolve()),
            "--exact",
            getattr(
                score, "RUNNER_TEST", "img::geometry_api_tests::replay_original_calls"
            ),
            "--nocapture",
        ],
        dict(
            os.environ,
            D2RS_GEOMETRY_REQUEST=str(request.resolve()),
            D2RS_GEOMETRY_OUTPUT=str(actual.resolve()),
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
        operation_outcomes = (
            [score.outcome(e, a) for e, a in zip(expected, got, strict=True)]
            if got is not None
            else ["rust-failed"] * len(expected)
        )
        result = next(
            (
                v
                for v in ["rust-failed", "mismatch", "unsupported"]
                if v in operation_outcomes
            ),
            "match",
        )
        row = {
            "name": case["name"],
            "outcome": result,
            "input_sha256": score.module("imageasset-scoreboard").value_sha(
                [op["request"] for op in case["operations"]]
            ),
            "expected_sha256": score.module("imageasset-scoreboard").value_sha(
                expected
            ),
            "operations": dict(collections.Counter(operation_outcomes)),
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
