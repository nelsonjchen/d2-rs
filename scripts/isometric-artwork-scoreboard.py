#!/usr/bin/env python3
"""Original explicit icon-resolver and physical/structured vector-capture calls.

Every original assertion remains active. Capture/replay uses 1 GiB total,
128 MiB/file, 20 GiB free streaming limits and finite deadlines.
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
    "TestSurfaceIconRasterAndExplicitLocalResolver": {"icon_new": 3, "icon": 5},
    "TestNativeVectorCaptureCoversPhysicalAndStructuredSurfaces": {"capture": 4},
}


def scoreboard():
    score = base(
        tests={n: sum(ops.values()) for n, ops in TESTS.items()},
        files=["surface_icons_test.go", "vector_surface_test.go"],
        sources=[
            "surface_icons.go",
            "render.go",
            "vector_surface.go",
            "vector_surface_svg.go",
        ],
        template=ROOT / "tools/isometric-artwork-scoreboard/hook_test.go",
        helper_templates=[ROOT / "tools/isometric-solid-scoreboard/hook_test.go"],
        call_replacements=[
            ("newSurfaceIconPainter(", "artworkNewIcon("),
            ("p.texture(", "artworkTexture(p, "),
            ("withAssets.texture(", "artworkTexture(withAssets, "),
            ("imageasset.New(", "artworkResolver("),
            ("d2isometric.BuildScene(", "artworkScene("),
            ("newNativeSceneWithOptions(", "artworkCapture("),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::surface_icons::artwork_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete icon texture pixel hashes, dimensions, sharing and errors",
            "actual local-file policy and original resolver limits; complete caller file bytes are inputs",
            "complete ordered meshes, shared material/group/owner/vector identities and texture pixel hashes",
            "all retained surface SVG fragments, complete scene and source target values",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "internal resolver cache allocation counts",
            "unobserved caller assertions",
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
