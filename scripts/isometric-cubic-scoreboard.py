#!/usr/bin/env python3
"""Observe both public BuildScene calls in the original cubic-route test.

The original direct shared-budget assertion also remains active in Go;
the corresponding native private-helper assertion is a Rust unit test.
"""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "cubic_scene_observer",
        Path(__file__).with_name("isometric-scene-scoreboard.py"),
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {"TestContainerHierarchyCubicRouteIsSampledFaithfully": 2}
    score.OPS = {name: {"build": count} for name, count in score.TESTS.items()}
    score.TEST_FILES = ["hierarchy_test.go"]
    score.SOURCE_FILES = score.TEST_FILES + [
        "scene_test.go",
        "scene.go",
        "hierarchy.go",
        "layout.go",
    ]
    score.COMPARISON_CONTRACT = {
        "compared": ["complete scenes", "unchanged source diagrams", "complete errors"],
        "unqualified": [
            "direct private-helper budget call (native unit test separately)"
        ],
        "original_go_assertions": "all retained and required to pass",
    }
    return score


if __name__ == "__main__":
    scoreboard().main()
