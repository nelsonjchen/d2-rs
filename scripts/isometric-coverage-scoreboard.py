#!/usr/bin/env python3
"""Original ground-receiver and SVG coverage contracts."""

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
    "TestMarkdownCardsDoNotChangeOtherTextSurfaces": {'markdown': 5},
    "TestGroundReceiversPreserveMeshAndShadowProjection": {'ground': 1},
    "TestGroundReceiversKeepIndependentRootShadowsStable": {'ground': 2},
    "TestHierarchyShadowReceiversFollowIndependentVisibleRoots": {'receivers': 2},
    "TestGroundReceiversKeepUnchangedSlice": {'ground': 3},
    "TestGroundReceiverSnapshotOwnsReceiverPointers": {'snapshot': 1},
    "TestGroundReceiverValidationRejectsInvalidPlanes": {'validate': 10, 'snapshot': 5},
    "TestSVGPaintCoverKeepsExactSilhouetteAndOneSourceOpacity": {'prepare': 1, 'cover': 2},
    "TestSVGPaintCoversKeepDifferentMaterialsAndDecalsSeparate": {'prepare': 1},
    "TestSVGTexturedCoverPreservesOpacityAndTransparentExterior": {'prepare': 1, 'cover': 1},
    "TestSVGTexturedCoversExcludeUnfilledMarginsAndDecals": {'prepare': 3},
    "TestSVGCoverExpansionIsBoundedForBothWindingsAndAcuteCorners": {'expand': 3},
    "TestSVGVisibilityIgnoresProjectionFMAResiduals": {'visibility': 2},
    "TestSVGLinksPreserveEqualAreaOrderAfterProjectionRounding": {'links': 1},
    "TestRasterCameraRoundsScaledCoordinates": {'project': 1},
    "TestSVGShadowOpaqueCapAndWallHaveOneStrength": {'shadow': 2},
    "TestSVGShadowRetainsTransparentTextureHole": {'shadow': 1},
    "TestSVGSurfaceResourcesAreSharedAcrossPaintPasses": {'definition': 1002},
    "TestNativeSVGVisibilityRemovesHiddenFaces": {'scene': 2},
    "TestSVGVisibilityClassRowCapCoversItsRearWall": {'visibility': 1},
}


def scoreboard():
    root = Path(__file__).resolve().parents[1]
    score = base(
        tests={name: sum(ops.values()) for name, ops in TESTS.items()},
        files=[
            "markdown_cards_test.go", "ground_receivers_test.go",
            "svg_seams_test.go", "svg_precision_test.go",
            "svg_shadow_coverage_test.go", "svg_surface_resources_test.go",
            "svg_test.go", "svg_visibility_native_test.go",
        ],
        sources=[
            "markdown_cards.go", "ground_receivers.go", "svg.go", "svg_seams.go",
            "svg_visibility.go", "svg_surface_resources.go", "raster.go",
        ],
        template=root / "tools/isometric-coverage-scoreboard/hook_test.go",
        helper_templates=[
            root / "tools/isometric-coverage-scoreboard/meshjson.go",
            root / "tools/isometric-content-scoreboard/scenejson.go",
        ],
        call_replacements=[
            ("nativeMarkdownCard(", "coverageMarkdown("),
            ("rasterGroundTriangles(", "coverageGround("),
            ("hierarchyShadowReceivers(", "coverageReceivers("),
            ("rasterSnapshot(", "coverageSnapshot("),
            ("rasterValidate(", "coverageValidate("),
            ("svgPreparePaintCovers(", "coveragePrepare("),
            ("writeSVGPaintCover(", "coverageWrite("),
            ("svgExpandCoverPolygon(", "coverageExpand("),
            ("svgVisibleFaces(", "coverageVisibility("),
            ("writeSVGLinks(", "coverageLinks("),
            ("if projected := camera.project(", "if projected := coverageProject(camera, "),
            ("rasterSVGShadowCoverage(", "coverageShadow("),
            ("nativeSceneSVG(", "coverageScene("),
            ("writer.surfaceDefinition(", "coverageDefinition(writer, "),
            ("other.surfaceDefinition(", "coverageDefinition(other, "),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::svg::coverage_api_tests::replay_original_calls"
    original_hashes = score.hashes

    def hashes(capture):
        result = original_hashes(capture)
        result.update({
            str(path.relative_to(capture)): score.sha(path)
            for path in sorted((capture / "assets").glob("*"))
        })
        return result

    score.hashes = hashes
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete ordered geometry, material/group/owner identities, receiver values and texture pixel hashes",
            "all input retained vector document trees/assets replayed into the native SVG writer",
            "complete SVG bytes, cover grouping, visibility fragments, projection coordinates and errors",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "Go slice-allocation and receiver-pointer identities (native receivers are owned scalar values)",
            "uncaptured caller raster assertions; SVG shadow bytes compare before caller rasterization",
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
