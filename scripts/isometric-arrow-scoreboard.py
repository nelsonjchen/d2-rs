#!/usr/bin/env python3
"""Original endpoint visibility calls, including warmed-cache cancellation."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
TESTS = {
    "TestArrowVisibilityRealFixtureAndBackendParity": 4,
    "TestArrowVisibilityPreservesFullMarkerAndSymmetricEndpoints": 22,
    "TestArrowVisibilityWalksShortBendAndKeepsTraffic": 1,
    "TestArrowVisibilityIgnoresLowerAndTransparentFill": 10,
    "TestArrowVisibilityExhaustedRoutesAndNoHeads": 6,
    "TestArrowVisibilityKeepsConcaveOpening": 2,
    "TestArrowVisibilityUsesExistingDepthBias": 2,
    "TestArrowVisibilityBoundsCachesWorkAndCancellation": 4,
}


def validate_query_geometry(cases):
    for case in cases:
        for op in case["operations"]:
            r = op["request"]
            if r["op"] == "arrow":
                assert isinstance(op["got"]["state"]["error"], str)
                for owner in r["builder"]["owners"].values():
                    for cache in owner["obstacles"]:
                        for poly in cache["polygons"]:
                            assert len(poly["points"]) >= 3
                            assert all(set(p) == {"x", "z"} for p in poly["points"])
                            assert set(poly["lo"]) == set(poly["hi"]) == {"x", "z"}
            else:
                assert all(set(p) == {"x", "z"} for p in op["got"]["points"])


def scoreboard():
    score = base(
        tests=TESTS,
        files=["arrow_visibility_test.go"],
        sources=["arrow_visibility.go", "geometry_routes.go"],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-arrow-scoreboard/hook_test.go",
        call_replacements=[
            ("arrowTestRoute(", "d2rsArrowRoute("),
            ("nativeArrowOccluder(", "d2rsArrowOccluder("),
        ],
    )
    score.OPS = {
        name: {"occluder" if "DepthBias" in name else "arrow": n}
        for name, n in TESTS.items()
    }
    original_verify = score.verify

    def verify(capture):
        cases = original_verify(capture)
        validate_query_geometry(cases)
        return cases

    score.verify = verify
    score.RUNNER_TEST = "img::arrow_visibility::api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete route and unchanged source path",
            "cumulative work, cache counts and errors",
            "receiver polygons and marker geometry supplied as inputs",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "Go pointer identities",
            "unobserved marker overlap predicates and pipeline packets remain original Go assertions",
            "material fields unused by visibility are outside this projection",
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
