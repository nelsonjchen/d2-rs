#!/usr/bin/env python3
"""Original configured-font and explicit-fallback calls; exact bytes and state.

Streaming limits: 1 GiB total, 128 MiB/file, 20 GiB free; finite deadlines.
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
    "TestNativePlainAndRichUseConfiguredAndExplicitFontFamilies": {
        "text": 3,
        "rich": 2,
    },
    "TestNativeTextUsesExplicitFallbackResolver": {"text": 1},
}


def scoreboard():
    score = base(
        tests={k: sum(v.values()) for k, v in TESTS.items()},
        files=["surface_fidelity_test.go"],
        sources=[
            "text.go",
            "surface_fonts.go",
            "surface_document.go",
            "rich_labels.go",
        ],
        template=ROOT / "tools/isometric-font-scoreboard/hook_test.go",
        helper_templates=[ROOT / "tools/isometric-content-scoreboard/scenejson.go"],
        call_replacements=[
            ("p.texture(", "fontTexture(p, "),
            ("richLabelDocument(", "fontRich("),
            ("p.configureFallbackFonts(", "fontConfigureFallback(p, "),
        ],
    )
    score.OPS = TESTS
    original_hashes = score.hashes

    def hashes(capture):
        return original_hashes(capture) | {
            str(p.relative_to(capture)): score.sha(p)
            for p in sorted((capture / "assets").glob("*"))
            if p.is_file()
        }

    score.hashes = hashes
    original_validate = score.validate_cases

    def validate(cases):
        original_validate(cases)
        for c in cases:
            for op in c["operations"]:
                got = op["got"]
                assert isinstance(got["error"], str)
                if op["request"]["op"] == "text":
                    assert set(got) == {
                        "error",
                        "frame",
                        "layout",
                        "faces",
                        "requested",
                        "requests",
                    }
                    frame = got["frame"]
                    assert set(frame) == {"width", "height", "stride", "pixels"}
                    w, h = frame["width"], frame["height"]
                    assert 0 < w <= 4096 and 0 < h <= 4096 and frame["stride"] == w * 4
                    raw = base64.b64decode(frame["pixels"], validate=True)
                    assert len(raw) == 4 * w * h
                    assert isinstance(got["requested"], bool) and isinstance(
                        got["requests"], list
                    )
                else:
                    assert (
                        set(got) == {"error", "document"}
                        and got["document"]["Root"] is not None
                    )

    score.validate_cases = validate
    score.RUNNER_TEST = "img::text::font_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete texture bytes, layouts, loaded-face keys, errors and source documents",
            "explicit fallback requests, returned font bytes and caller-observed resolver state",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": ["Go pointer addresses and allocation counts"],
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
