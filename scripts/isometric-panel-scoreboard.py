#!/usr/bin/env python3
"""Original source-panel reservation and caption/artwork calls.

All original assertions remain active. Streaming capture limits: 1 GiB total,
128 MiB/file, 20 GiB free; finite subprocess deadlines.
"""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestOutputDensityReservesFollowingSourcePanel": {"panel": 5},
    "TestNativeCanonicalArtworkStaysOnFaceWithOutsideCaption": {"hierarchy": 2},
    "TestPresentationStackedPersonCaptionsRetainSourceClearance": {
        "hierarchy": 2,
        "node": 2,
    },
}


def scoreboard():
    score = base(
        tests={n: sum(ops.values()) for n, ops in TESTS.items()},
        files=[
            "source_panel_density_test.go",
            "surface_fidelity_test.go",
            "presentation_caption_test.go",
        ],
        sources=[
            "source_panels.go",
            "surface_document.go",
            "native_shapes.go",
            "hierarchy_boards.go",
            "native_solids.go",
        ],
        template=ROOT / "tools/isometric-panel-scoreboard/hook_test.go",
        helper_templates=[
            ROOT / "tools/isometric-solid-scoreboard/hook_test.go",
            ROOT / "tools/isometric-content-scoreboard/scenejson.go",
        ],
        call_replacements=[
            ("b.sourcePanel(", "panelObserve(b, "),
            ("b.hierarchyNode(", 'd2rsPanelObserveNode(b, "hierarchy", '),
            ("b.node(", 'd2rsPanelObserveNode(b, "node", '),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::source_panels::panel_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete ordered mesh and shared material/group/owner identities",
            "every texture pixel hash, dimensions, errors and pixel budgets",
            "complete source-panel documents, ranges and source-node values",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "uncaptured caller-side negative-control deformation and camera assertions",
            "internal painter caches and allocation counts",
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
