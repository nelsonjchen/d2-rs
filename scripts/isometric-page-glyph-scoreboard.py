#!/usr/bin/env python3
"""Observe the original physical Page glyph-removal contract, assertions intact.

Input is the complete pre-removal source scene root. The whole resulting root
and outer stroke identity compare. No raster, font or pointer-layout claim.
Capture limits: 1 GiB total / 128 MiB file / 20 GiB free; finite deadlines.
"""
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "geometry", ROOT / "scripts/isometric-geometry-scoreboard.py"
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
TEST = "TestNativePhysicalPageReplacesFlatGlyphAndRetainsSourcePerimeter"


def scoreboard():
    score = base(
        tests={TEST: 1},
        files=["native_page_test.go"],
        sources=["native_page.go"],
        template=ROOT / "tools/isometric-page-glyph-scoreboard/hook_test.go",
        helper_templates=[
            ROOT / "tools/isometric-solid-scoreboard/hook_test.go",
            ROOT / "tools/isometric-content-scoreboard/scenejson.go",
        ],
        call_replacements=[("nativeRemovePageGlyph(", "pageObserveRemoveGlyph(")],
    )
    score.OPS = {TEST: {"remove-page-glyph": 1}}
    score.RUNNER_TEST = "replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "original_go_assertions": "all retained and required to pass",
        "compared": [
            "complete source root before and after glyph removal",
            "outer source stroke identity preservation",
        ],
        "unqualified": [
            "source scene construction (the original Go-built root is the replay input)",
            "Go pointer addresses and allocation layout",
        ],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
