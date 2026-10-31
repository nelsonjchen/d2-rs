#!/usr/bin/env python3
"""Original shadow-alpha, SVG-mask and COLRv1-gradient tests, without normalization.

Shared streaming gates: 1 GiB total, 128 MiB/file, 20 GiB free;
600-second Go / 60-second native deadlines. Original assertions stay active.
"""

import base64
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
    "TestShadowRespectsTextureAlphaAndAuthoredOpacity": {"shadow": 1, "validate": 3},
    "TestNativeVectorMaskTypeUsesPresentationAttribute": {"svg": 2, "paint": 1},
    "TestNativeVectorColorGradientKeepsLinearLight": {"paint": 1},
}


def scoreboard():
    score = base(
        tests={name: sum(ops.values()) for name, ops in TESTS.items()},
        files=[
            "surface_depth_test.go",
            "vector_surface_mask_test.go",
            "vector_surface_test.go",
        ],
        sources=["raster.go", "vector_surface_svg.go", "vector_surface_color.go"],
        template=ROOT / "tools/isometric-fidelity-scoreboard/hook_test.go",
        helper_templates=[ROOT / "tools/isometric-content-scoreboard/scenejson.go"],
        call_replacements=[
            ("rasterBuildShadow(", "fidelityShadow("),
            ("rasterValidate(", "fidelityValidate("),
            ("nativeSurfaceSVG(", "fidelitySVG("),
            ("w.colorPaint(", "fidelityPaint(w, "),
            ("writer.colorPaint(", "fidelityPaint(writer, "),
        ],
    )
    score.OPS = TESTS
    original_validate = score.validate_cases

    def validate(cases):
        original_validate(cases)
        for case in cases:
            for op in case["operations"]:
                req, got = op["request"], op["got"]
                if req["op"] == "shadow":
                    assert set(got) == {"shadow", "error", "remaining"}
                    assert got["error"] == ""
                    shadow = got["shadow"]
                    assert set(shadow) == {"camera", "depth", "opacity", "bias"}
                    count = shadow["camera"]["width"] * shadow["camera"]["height"]
                    assert count == 2048 * 2048
                    assert (
                        len(base64.b64decode(shadow["depth"], validate=True))
                        == 4 * count
                    )
                    assert (
                        len(base64.b64decode(shadow["opacity"], validate=True)) == count
                    )
                elif req["op"] == "validate":
                    assert set(got) == {"error"} and isinstance(got["error"], str)
                elif req["op"] == "svg":
                    assert set(got) == {"error", "svg"} and got["svg"]
                else:
                    assert set(got) == {"body", "writer"}
                    assert set(got["writer"]) == {
                        "defs",
                        "next",
                        "nodes",
                        "commands",
                        "transform",
                        "linear",
                        "error",
                    }

    score.validate_cases = validate
    score.RUNNER_TEST = (
        "img::vector_surface_svg::fidelity_api_tests::replay_original_calls"
    )
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete shadow opacity and float32 depth bytes, camera, bias and work budget",
            "complete invalid-opacity errors including a preserved NaN input",
            "complete mask SVG and color-paint body, definitions, resource counters and state",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "Go allocations, pointer addresses and font lookup in these font-free paint plans"
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
