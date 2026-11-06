#!/usr/bin/env python3
"""Original solid, relief and route-caption mesh contracts."""

import argparse
import collections
import importlib.util
import json
import os
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
TESTS = {
    "TestNativeSolidsKeepGeometryAcrossPaintStyles": {"solid": 22},
    "TestNativeSolidsHaveBoundedFootprintsAndOutwardSmoothNormals": {"solid": 15},
    "TestNativeSolidCrownRetainsAuthoredMaterialTone": {"paint": 2},
    "TestNativeSolidQueueLabelIsSupportedByContinuousCrown": {"solid": 2},
    "TestNativeSolidPatternsDashesAndDoubleBordersRemainVisible": {"solid": 17},
    "TestNativeSolidLabelsAndIconsKeepCompiledPrintAreas": {"solid": 11},
    "TestNativeSolidOpacityCopiesAndTextureAdmission": {"solid": 20, "node": 2},
    "TestNativeSolidIconPrintIsPhysicallySupported": {"solid": 6},
    "TestReliefSymbolsKeepSourceContourAcrossStyles": {"relief": 15},
    "TestReliefSymbolDepthIgnoresLegacySculptureHeight": {"relief": 108},
    "TestReliefSymbolCaptionsAndIconsStayOnSourcePrintPlane": {"relief": 12},
    "TestReliefC4PersonKeepsBothContoursAndEmptyShoulders": {"relief": 1},
    "TestReliefTransparentSymbolsKeepSourceAlpha": {"relief": 6},
    "TestReliefSymbolSourcePortsMeetWallsWithoutRouteExtensions": {"relief": 3},
    "TestNativePageFoldIsPhysicalAndKeepsSourceFootprint": {"canonical": 2},
    "TestConnectionCaptionBackgroundRequiresExplicitFill": {"edges": 6},
    "TestRouteCaptionKnockoutPreservesProjectedDashesAndAlpha": {"knockout": 3},
    "TestRouteCaptionKnockoutPreservesTriangleMetadata": {"knockout": 1},
    "TestConnectionCaptionKnockoutPreservesEarlierGeometryAndMarkers": {"edges": 2},
    "TestConnectionCaptionGapRevealsUnderlyingSurface": {"edges": 4},
}


def scoreboard():
    score = base(
        tests={n: sum(ops.values()) for n, ops in TESTS.items()},
        files=[
            "native_solids_test.go",
            "native_symbol_relief_test.go",
            "native_page_test.go",
            "route_caption_mask_test.go",
        ],
        sources=[
            "native_solids.go",
            "native_shapes.go",
            "native_page.go",
            "route_caption_mask.go",
            "native_edges.go",
        ],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-solid-scoreboard/hook_test.go",
        call_replacements=[
            ("solidTestBuild(", "solidObserveBuild("),
            ("reliefTestBuild(", "solidObserveRelief("),
            ("b.solidPaint(", "solidObservePaint(b, "),
            ("b.node(", "solidObserveNode(b, "),
            ("b.canonicalNode(", "solidObserveCanonical(b, "),
            ("b.edges(", "solidObserveEdges(b, "),
            ("with.edges(", "solidObserveEdges(with, "),
            ("without.edges(", "solidObserveEdges(without, "),
            ("b.routeCaptionKnockout(", "solidObserveKnockout(b, "),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::native_solids::solid_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "all ordered vertices, normals, UVs and triangle metadata",
            "material values and shared material/group/owner identities",
            "texture dimensions and SHA-256 of every premultiplied RGBA pixel",
            "source nodes and edges, builder errors and texture budget state",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "Go allocations and pointer addresses",
            "uncaptured caller/raster assertions",
            "vector surface document bodies (presence only)",
            "internal painter allocation/cache state",
        ],
    }
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
    digest = score.sha(args.rust)
    output = []
    runs = []
    for i, case in enumerate(cases):
        request = work / f"request-{i}.json"
        actual = work / f"actual-{i}.json"
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
                    ]
                }
            )
            + "\n"
        )
        run = score.module("cli-scoreboard").bounded_run(
            [str(args.rust.resolve()), "--exact", score.RUNNER_TEST, "--nocapture"],
            dict(
                os.environ,
                D2RS_GEOMETRY_REQUEST=str(request.resolve()),
                D2RS_GEOMETRY_OUTPUT=str(actual.resolve()),
            ),
            work,
            work / f"run-{i}.log",
            60,
        )
        runs.append(run)
        sizes = [p.stat().st_size for p in work.rglob("*") if p.is_file()]
        if max(sizes) > 128 << 20 or sum(sizes) > 1 << 30:
            run["failure"] = "budget-exceeded"
        values = (
            json.loads(actual.read_text())
            if run == {"exit": 0, "failure": None} and actual.is_file()
            else None
        )
        assert values is None or len(values) == 1
        output.append(values[0] if values is not None else None)
        if run.get("failure") is not None:
            output.extend([None] * (len(cases) - len(output)))
            break
    run = (
        {"exit": 0, "failure": None}
        if all(r == {"exit": 0, "failure": None} for r in runs)
        else {"shards": runs}
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
