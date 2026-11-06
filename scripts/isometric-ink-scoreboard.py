#!/usr/bin/env python3
"""Original classic-outline cancellation and input admission contracts."""

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
            "TestClassicInkCancellationAndWorkLimit": 2,
            "TestNativeSolidInkFitCancellation": 1,
        },
        files=["native_edges_test.go", "native_face_ink_test.go"],
        sources=["native_edges.go", "native_solids.go"],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-ink-scoreboard/hook_test.go",
        call_replacements=[
            ("classicInkSegments(", "d2rsInkSegments("),
            ("b.solidInkFit(", "d2rsInkFit(b, "),
        ],
    )
    score.OPS = {
        name: {"fit" if "Fit" in name else "segments": n}
        for name, n in score.TESTS.items()
    }
    score.RUNNER_TEST = "img::native_edges::api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete admission error",
            "returned segment count and ink-fit factor",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "segment and supporting-facet geometry on successful extraction",
            "allocation counts and asynchronous cancellation timing",
        ],
        "inputs": "complete consumed triangle fields and raw texture storage; oversized zero slice encoded by exact count",
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
