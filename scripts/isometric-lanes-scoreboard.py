#!/usr/bin/env python3
"""Capture original route-lane calls and integrated packet paths, all assertions active."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard


def scoreboard():
    score = base(
        tests={
            "TestNativeLanesDistinctStableFlatAndOwned": 3,
            "TestNativeLanesLocalPartialOverlap": 2,
            "TestNativeLanesAvoidModulesHeadersAndNeighbors": 4,
            "TestNativeLanesLeaveNonoverlapsAndInvisibleRoutes": 7,
            "TestNativeLanesPipelineUsesSamePacketGeometry": 2,
            "TestNativeLanesAdmissionAndCancellation": 5,
            "TestNativeLanesContinueThroughSharedBends": 4,
            "TestNativeLanesDoNotEnterEndpointOnLaterRun": 1,
        },
        files=["route_lanes_test.go"],
        sources=["route_lanes.go", "route_bridges.go", "geometry_routes.go"],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-lanes-scoreboard/hook_test.go",
        call_replacements=[
            ("nativeLaneRoutes(", "d2rsGeometryLanes("),
            ("nativeResolveLaneRoutes(", "d2rsGeometryLanesLimit("),
            ("b.edges(", "d2rsGeometryPackets(b, "),
        ],
    )
    score.OPS = {name: {"lane": n} for name, n in score.TESTS.items()}
    score.OPS["TestNativeLanesPipelineUsesSamePacketGeometry"] = {
        "lane": 1,
        "pipeline": 1,
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
