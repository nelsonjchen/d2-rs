#!/usr/bin/env python3
"""Original GIF palette/writer, mask ownership and shadow-framing observations.

Uses the shared bounded capture/replay gate: 1 GiB total, 128 MiB/file,
20 GiB free, 600-second Go and 60-second native deadlines.
"""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard

TESTS = {
    "TestSharedPaletteAndOpaqueDelta": {
        "palette": 1,
        "index": 3,
        "delta": 3,
        "delay": 100,
    },
    "TestFrameAndWriterLimits": {"write": 2},
    "TestNativeGIFLoopCopiesAnimatedMasks": {"loop": 1},
    "TestShadowFramingIgnoresNonCastingGeometry": {"ground": 4},
}


def scoreboard():
    score = base(
        tests={name: sum(ops.values()) for name, ops in TESTS.items()},
        files=["render_test.go", "gif_loop_test.go", "shadow_framing_test.go"],
        sources=["gif.go", "gif_loop.go", "framing.go"],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-output-scoreboard/hook_test.go",
        call_replacements=[
            ("paletteLookup(", "outputPalette("),
            ("indexFrame(", "outputIndex("),
            ("frameDelta(", "outputDelta("),
            ("frameDelay(", "outputDelay("),
            ("w.Write(", "outputWrite(&w, "),
            ("nativeGIFLoopScene(", "outputLoop("),
            ("extent.groundShadows(", "outputGround(&extent, "),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::gif::output_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "entire 262144-byte RGB lookup, complete indexed frames, delta rectangles/pixels/palettes and all delays",
            "writer count, full error, destination bytes and remaining budget",
            "complete mask document before/after loop adaptation and independent ownership",
            "all projected extent fields for non-casting geometry",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "GIF encoded bytes and decoder internals",
            "Go pointer addresses and cyclic pointer graphs, unavailable in owned Rust nodes",
            "arbitrary io.Writer callbacks (these original calls use bytes.Buffer)",
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
