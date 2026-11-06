#!/usr/bin/env python3
"""Original ink, Markdown-card and opacity-composition calls; assertions retained."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
ROOT = Path(__file__).resolve().parents[1]
# Exact operation counts are validated against the original test loops.
TESTS = {
    "TestClassicInkAuthoredPaintDashAndCaptionOrder": {"ink": 5},
    "TestClassicInkTranslucentGroupRetainsCompensatedSourceCap": {"ink": 2},
    "TestClassicInkRasterWeightAndOcclusion": {"ink": 1, "render": 3},
    "TestClassicInkSmoothContourHasContinuousRasterCoverage": {
        "segments": 1,
        "ink": 1,
        "render": 1,
    },
    "TestClassicInkMultipleRearCopyCannotBleedAcrossPrimaryCap": {
        "ink": 3,
        "render": 2,
    },
    "TestClassicInkBranchCornersHaveContinuousCoverage": {"ink": 6, "render": 12},
    "TestClassicInkBranchJoinsAreBoundedAndRespectPaintedEnds": {
        "junctions": 15,
        "render": 6,
    },
    "TestClassicInkBranchJoinKeepsStraightStrokeWidth": {"ink": 1, "render": 1},
    "TestClassicInkCurvedSilhouettesKeepTheirInsideHalf": {"node": 18, "segments": 18},
    "TestClassicInkSupportCannotClimbOntoAnOppositeCap": {"segments": 1},
    "TestMarkdownCardsKeepSourceFootprintsAndAttachToSupport": {
        "node": 2,
        "hierarchy": 2,
    },
    "TestMarkdownCardsRaiseExistingDocumentIconsAndLinksTogether": {
        "content": 2,
        "hierarchy": 2,
    },
    "TestMarkdownCardsKeepAuthoredPaintModifiersAndOpacity": {"hierarchy": 3},
    "TestMarkdownCardHeaderAvoidsRaisedPaper": {
        "hierarchy": 1,
        "header": 1,
        "render-nodes": 1,
        "placement": 1,
    },
    "TestMarkdownCardHeaderUsesClearSourceBottomMargin": {
        "hierarchy": 1,
        "render-nodes": 3,
        "placement": 3,
    },
    "TestOpacityGroupComposesFacesAndInkBeforeFade": {"render": 4},
    "TestOpacityGroupOccludesInternallyAndRespectsOtherDepth": {"render": 1},
    "TestOpacityGroupFrameReplaysDepthAndStrips": {"render": 8},
    "TestOpacityGroupFadesBackgroundDecalsAndKeepsForeground": {"render": 2},
    "TestOpacityGroupInterleavesExternalTransparentPaint": {"render": 12},
}


def scoreboard():
    score = base(
        tests={n: sum(ops.values()) for n, ops in TESTS.items()},
        files=[
            "native_edges_test.go",
            "native_ink_joins_test.go",
            "native_ink_support_test.go",
            "markdown_cards_test.go",
            "opacity_groups_test.go",
        ],
        sources=[
            "native_edges.go",
            "native_ink_joins.go",
            "markdown_cards.go",
            "opacity_groups.go",
            "hierarchy_headers.go",
            "raster.go",
        ],
        template=ROOT / "tools/isometric-composition-scoreboard/hook_test.go",
        helper_templates=[
            ROOT / "tools/isometric-solid-scoreboard/hook_test.go",
            ROOT / "tools/isometric-shape-scoreboard/hook_test.go",
            ROOT / "tools/isometric-hierarchy-scoreboard/hook_test.go",
            ROOT / "tools/isometric-composition-scoreboard/raster_test.go",
        ],
        call_replacements=[
            ("b.classicInkEdges(", "compositionEdges(b, "),
            ("b.classicInkJunctions(", "compositionJunctions(b, "),
            ("reversed.classicInkJunctions(", "compositionJunctions(reversed, "),
            ("classicInkSegments(", "compositionSegments("),
            ("b.node(", 'compositionNode(b, "node", '),
            ("b.hierarchyNode(", 'compositionNode(b, "hierarchy", '),
            ("after.hierarchyNode(", 'compositionNode(after, "hierarchy", '),
            ("before.canonicalNodeContent(", "compositionContent(before, "),
            ("hierarchyBoardHeaderSurface(", "hierObserveHeader("),
            ("hierarchyBoardHeaderPlacement(", "hierObservePlacement("),
            ("hierarchyRenderNodes(", "hierObserveRenderNodes("),
            ("newRaster(", "rasterObserveConstruct("),
            ("r.Frame(", "rasterObserveFrame(r, "),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_ARGUMENTS = ["--ignored"]
    score.RUNNER_TEST = (
        "img::native_edges::composition_api_tests::replay_original_calls"
    )
    return pixel_score(score)


def pixel_score(score):
    original_hashes = score.hashes

    def hashes(capture):
        result = original_hashes(capture)
        result.update(
            {
                str(p.relative_to(capture)): score.sha(p)
                for p in sorted((capture / "images").glob("*.rgba"))
            }
        )
        return result

    score.hashes = hashes
    original_verify = score.verify

    def verify(capture):
        cases = original_verify(capture)
        # Reuse the strict complete pixel inventory gate; absent/corrupt images fail.
        raster = geometry.spec_from_file_location(
            "raster", ROOT / "scripts/isometric-raster-scoreboard.py"
        )
        module = geometry.module_from_spec(raster)
        raster.loader.exec_module(module)
        module.validate_image_store(
            capture / "images",
            [
                op["got"]
                for c in cases
                for op in c["operations"]
                if op["request"]["op"] in getattr(score, "PIXEL_OPS", {"render"})
            ],
        )
        return cases

    score.verify = verify
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete ordered ink mesh, support facets and material/group/owner sharing",
            "Markdown-card geometry, texture hashes, source fields, links and placement",
            "all raster output pixel hashes and complete original raw pixel inventories",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "Go allocation/pointer addresses",
            "opacity raster calls use original supplied Go meshes; mesh construction and caller cached-state assertions are separate",
            "native stateless raster frame replay does not establish original Go cached allocation layout",
        ],
    }
    return score


def main(score_factory=None):
    import argparse, collections, json, os

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    score = (score_factory or scoreboard)()
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists()
        score.capture_original(capture)
    cases = score.verify(capture)
    if args.output is None:
        print("verified original functions", len(cases))
        return
    assert args.rust and args.rust.is_file() and not args.output.exists()
    spec = geometry.spec_from_file_location(
        "raster", ROOT / "scripts/isometric-raster-scoreboard.py"
    )
    raster = geometry.module_from_spec(spec)
    spec.loader.exec_module(raster)
    work = args.output.resolve().parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    digest = score.sha(args.rust)
    rows = []
    runs = []
    for i, c in enumerate(cases):
        shard = work / str(i)
        shard.mkdir()
        request = shard / "request.json"
        output = shard / "actual.json"
        request.write_text(
            json.dumps(
                {
                    "cases": [
                        {
                            "name": c["name"],
                            "operations": [
                                {"request": o["request"]} for o in c["operations"]
                            ],
                        }
                    ]
                }
            )
            + "\n"
        )
        run = score.module("cli-scoreboard").bounded_run(
            [
                str(args.rust.resolve()),
                "--exact",
                score.RUNNER_TEST,
                "--ignored",
                "--nocapture",
            ],
            dict(
                os.environ,
                D2RS_GEOMETRY_REQUEST=str(request),
                D2RS_GEOMETRY_OUTPUT=str(output),
            ),
            work,
            shard / "run.log",
            120,
        )
        runs.append(run)
        got = (
            json.loads(output.read_text())[0]
            if run == {"exit": 0, "failure": None} and output.is_file()
            else None
        )
        expected = [o["got"] for o in c["operations"]]
        result = (
            "rust-failed" if got is None else raster.case_outcome(score, expected, got)
        )
        if got is not None and any(
            o["request"]["op"] in getattr(score, "PIXEL_OPS", {"render"}) for o in c["operations"]
        ):
            raster.validate_image_store(
                shard / "images",
                [
                    v
                    for o, v in zip(c["operations"], got, strict=True)
                    if o["request"]["op"] in getattr(score, "PIXEL_OPS", {"render"})
                ],
            )
        row = {"name": c["name"], "outcome": result, "operations": len(expected)}
        if result == "mismatch":
            row["differences"] = score.module("language-scoreboard").differences(
                expected, got
            )[:30]
        rows.append(row)
        print(row, flush=True)
    assert score.sha(args.rust) == digest
    report = {
        "source_commit": score.references.SOURCE_COMMIT,
        "package": "d2renderers/d2isometric/d2isometricimg",
        "captured_functions": len(cases),
        "observed_operations": sum(r["operations"] for r in rows),
        "runner_sha256": digest,
        "capture_hashes": score.hashes(capture),
        "runs": runs,
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
        "comparison_contract": score.COMPARISON_CONTRACT,
        "actual_hashes": {
            str(p.relative_to(work)): score.sha(p)
            for p in sorted(work.rglob("*"))
            if p.is_file()
        },
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
