#!/usr/bin/env python3
"""Replay original scene layout and sequence-identity tests, without repacking.

Uses the scene observer's complete scene/source/error comparison and bounded
capture. All original hierarchy and sequence assertions remain active.
"""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "scene_scoreboard", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestContainerHierarchyPreservesCompiledLayout": 1,
        "TestDefaultOptionsPreserveCompiledLayout": 2,
        "TestContainerHierarchyFloatingAndAbsentRoutes": 2,
        "TestContainerHierarchyZeroDimensionsAndLargeCoordinates": 2,
        "TestSequenceRolesPreserveQuotedAuthoredTimeline": 3,
        "TestSequenceLifelinesRequireExactSyntheticIdentity": 5,
        "TestSequenceRealMixedActorSubgraphsAndSemanticChildren": 4,
    }
    score.OPS = {name: {"build": count} for name, count in score.TESTS.items()}
    score.TEST_FILES = ["hierarchy_test.go", "sequence_test.go"]
    score.SOURCE_FILES = score.TEST_FILES + [
        "scene_test.go",
        "scene.go",
        "hierarchy.go",
        "layout.go",
        "sequence.go",
    ]
    return score


if __name__ == "__main__":
    scoreboard().main()
