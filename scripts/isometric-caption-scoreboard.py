#!/usr/bin/env python3
"""Original route-caption placement, budgets and authored-anchor contracts."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard

TESTS = {
    "TestRouteCaptionsSeparateSharedCorridor": {"place": 16},
    "TestRouteCaptionsKeepEndpointIdentityAndReadableAngle": {"place": 4},
    "TestRouteCaptionsAvoidComponentsAndUseOtherLegs": {"place": 1},
    "TestRouteCaptionOrientedOverlap": {"overlap": 2},
    "TestRouteCaptionBudgetsAndDegenerateRoutes": {"place": 5},
    "TestNativeConnectionCaptionPositionAndPercentage": {"surface": 5},
    "TestNativeRouteCaptionMissingMetricsUseBoundedFallback": {"size": 3},
    "TestNativeCaptionOrientationUsesCompleteRouteRuns": {"surface": 4},
    "TestNativeCurvedCaptionsPreserveCompiledPrintMetrics": {"size": 2},
    "TestNativeRouteCaptionPreservesMultilineFont": {"size": 1},
    "TestNativeOverflowCaptionsKeepSourceOrientation": {"size": 4, "surface": 8},
    "TestEndpointClippingPreservesOriginalDashPhaseAndBends": {"dashes": 4},
    "TestNativeRouteRemainsFlatAndBounded": {"rounded": 1},
    "TestClassicInkDepthRoundsCancellationProducts": {"depth": 1},
    "TestClassicInkSupportClipCyclicInput": {"face": 3},
    "TestClassicInkSelectsPhysicalCubeEdges": {"segments": 4},
    "TestClassicInkPreservesDecorativeTopBorders": {"segments": 2},
    "TestClassicInkSmoothSphereContourIgnoresTessellation": {"segments": 1},
    "TestClassicInkCylinderHasRimsWithoutFacetLadder": {"segments": 1},
    "TestClassicInkCylinderAndTorsoRimsJoinVisibleSilhouettes": {"segments": 2},
}


def scoreboard():
    score = base(
        tests={name: sum(ops.values()) for name, ops in TESTS.items()},
        files=[
            "route_labels_test.go",
            "route_caption_style_test.go",
            "route_dash_clip_test.go",
            "geometry_test.go",
            "svg_precision_test.go",
            "native_edges_test.go",
            "native_ink_joins_test.go",
        ],
        sources=[
            "route_labels.go",
            "route_caption_style.go",
            "geometry_routes.go",
            "native_edges.go",
        ],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-caption-scoreboard/hook_test.go",
        call_replacements=[
            ("newRouteCaptionPlacer()", "d2rsCaptionPlacer()"),
            ("captionOverlap(", "d2rsCaptionOverlap("),
            ("nativeConnectionCaptionSurface(", "d2rsCaptionSurface("),
            ("nativeRouteCaptionSize(", "d2rsCaptionSize("),
            ("nativeRouteDashes(", "d2rsCaptionDashes("),
            ("nativeRoundedRoute(", "d2rsCaptionRounded("),
            ("face.depthAt(", "d2rsCaptionDepth(face, "),
            ("classicInkSegments(", "d2rsCaptionSegments("),
            ("b.classicInkFace(", "d2rsCaptionFace(b, "),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::route_labels::api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete label surfaces and acceptance",
            "complete ordered obstacle rectangles and work/reference counts",
            "complete unchanged routes and source connection inputs",
            "caption overlap and compiled print dimensions",
            "complete ordered ink segments, supporting facets, clipped ink faces, rounded routes and dash paths",
        ],
        "inputs": "full placer state; consecutive identical obstacles use exact run-length encoding; nonfinite input dimensions retain their classification",
        "original_go_assertions": "all retained and required to pass",
        "unqualified": ["Go allocations and grid-map layout"],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
