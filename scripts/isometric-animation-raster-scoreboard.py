#!/usr/bin/env python3
"""Observe complete raster calls in original animation and shadow-framing tests.

All Go assertions remain active. Replays receive caller geometry/cameras, never
expected pixels. Limits: 1 GiB total, 128 MiB/file, 20 GiB free, 600s deadlines.
"""

import importlib.util
import json
import os
import re
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "raster_api", Path(__file__).with_name("isometric-raster-scoreboard.py")
)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
ORIGINAL_SCOREBOARD = base.scoreboard
TEST_FILES = {
    "gif_loop_test.go": [
        "TestNativeGIFAuthoredNodeLoopAndPacketSeam",
        "TestNativeGIFPanelLoopDoesNotMutateSourceTiming",
    ],
    "animation_page_regression_test.go": [
        "TestAnimationFramingKeepsTinyNodeAndTrafficGeometry"
    ],
    "surface_depth_test.go": [
        "TestSurfaceDecalSurvivesLargeTexturedSubstrate",
        "TestAuthoredShapeMotionUsesFixedCameraAndExactOneSecondCycle",
    ],
    "shadow_framing_test.go": [
        "TestFramingContainsRenderedSolidGroundShadow",
        "TestCaptureAndTimelineContainAnimatedGroundShadows",
    ],
}
# Counts are from the pinned original Go call inventory, including repeated GIF passes.
EXPECTED_OPERATIONS = {
    "TestNativeGIFAuthoredNodeLoopAndPacketSeam": 205,
    "TestNativeGIFPanelLoopDoesNotMutateSourceTiming": 5,
    "TestAnimationFramingKeepsTinyNodeAndTrafficGeometry": 16,
    "TestSurfaceDecalSurvivesLargeTexturedSubstrate": 2,
    "TestAuthoredShapeMotionUsesFixedCameraAndExactOneSecondCycle": 8,
    "TestFramingContainsRenderedSolidGroundShadow": 4,
    "TestCaptureAndTimelineContainAnimatedGroundShadows": 10,
}


def scoreboard():
    score = ORIGINAL_SCOREBOARD()
    score.TESTS = dict.fromkeys(n for names in TEST_FILES.values() for n in names)
    package = "d2renderers/d2isometric/d2isometricimg"
    template = score.ROOT / "tools/isometric-animation-raster-scoreboard/hook_test.go"

    def source():
        pinned = score.references.verify_source()
        files = list(TEST_FILES) + [
            "raster.go",
            "render.go",
            "gif_loop.go",
            "framing.go",
        ]
        return {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + p: score.sha(pinned / package / p) for p in files
            },
            "hooks": {str(template.relative_to(score.ROOT)): score.sha(template)},
        }

    score.source = source
    score.comparison_contract = {
        "compared": [
            "complete RGBA frame bytes, dimensions, strides and errors from every observed newRaster/Raster.Frame call",
            "unchanged caller geometry, texture bytes and explicit view, shadow and ground cameras are request inputs",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "caller motion, loop adaptation and framing/camera construction are not replayed by this raster tier",
            "GIF palette/encoding and Go allocation/cache identities remain original Go assertions",
        ],
    }

    def validate(cases):
        assert len(cases) == len(score.TESTS) and {c["name"] for c in cases} == set(
            score.TESTS
        )
        for c in cases:
            assert c["operations"], c["name"]
            count = EXPECTED_OPERATIONS[c["name"]]
            assert len(c["operations"]) == count, (
                c["name"],
                len(c["operations"]),
                count,
            )
            assert all(
                o["request"]["op"] == "render" and isinstance(o["got"]["error"], str)
                for o in c["operations"]
            )
            assert all("ground_camera" in o["request"] for o in c["operations"])

    score.validate_cases = validate

    # The base verifier closes over its own validator; preserve its hash/image checks,
    # but substitute the exact new suite inventory.
    def verify(capture):
        assert json.loads((capture / "source.json").read_text()) == source()
        assert json.loads((capture / "inventory.json").read_text()) == score.TESTS
        assert json.loads((capture / "hashes.json").read_text()) == score.hashes(
            capture
        )
        assert json.loads((capture / "run.json").read_text()) == {
            "exit": 0,
            "failure": None,
        }
        events = score.module("cli-scoreboard").events(capture / "go.jsonl")
        assert (
            all(events.get(n) == "pass" for n in score.TESTS)
            and "fail" not in events.values()
        )
        cases = [base.load_original(p) for p in sorted((capture / "go").glob("*.json"))]
        validate(cases)
        base.validate_image_store(
            capture / "images", [o["got"] for c in cases for o in c["operations"]]
        )
        validate_textures(capture, cases, score.sha)
        sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
        assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
        return cases

    original_hashes = score.hashes

    def hashes(capture):
        values = original_hashes(capture)
        values.update(
            {
                str(p.relative_to(capture)): score.sha(p)
                for p in sorted((capture / "textures").glob("*.pixels"))
            }
        )
        return values

    score.hashes = hashes
    score.verify = verify

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        paths = [folder / p for p in TEST_FILES] + [folder / "raster.go"]
        originals = {p: p.read_bytes() for p in paths}
        hook = folder / "d2rs_animation_raster_hook_test.go"
        assert not hook.exists()
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            if path.name == "raster.go":
                for before, after in [
                    ("func newRaster(", "func d2rsOriginalNewRaster("),
                    ("func (r *Raster) Frame(", "func (r *Raster) d2rsOriginalFrame("),
                ]:
                    assert text.count(before) == 1
                    text = text.replace(before, after)
            else:
                for name in TEST_FILES[path.name]:
                    match = re.search(
                        r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text
                    )
                    assert match, name
                    body = match[0].replace("{\n", "{\n\trasterObserveStart(t)\n", 1)
                    text = text[: match.start()] + body + text[match.end() :]
            replacements[path] = text
        (capture / "go").mkdir(parents=True)
        (capture / "tmp").mkdir()
        (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
        (capture / "inventory.json").write_text(json.dumps(score.TESTS) + "\n")
        env = dict(
            os.environ,
            TMPDIR=str(capture / "tmp"),
            D2RS_SCENE_API=str(capture / "go"),
            GOMODCACHE=str(score.ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(score.ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
            GOMAXPROCS="2",
        )
        try:
            for p, text in replacements.items():
                p.write_text(text)
            hook.write_bytes(template.read_bytes())
            run = score.module("cli-scoreboard").bounded_run(
                [
                    "go",
                    "test",
                    "./" + package,
                    "-run",
                    "^(" + "|".join(score.TESTS) + ")$",
                    "-json",
                    "-count=1",
                    "-parallel=1",
                ],
                env,
                capture,
                capture / "go.jsonl",
                600,
            )
        finally:
            for p, data in originals.items():
                p.write_bytes(data)
            hook.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(run) + "\n")
        assert run == {"exit": 0, "failure": None}, run
        (capture / "hashes.json").write_text(
            json.dumps(score.hashes(capture), indent=2) + "\n"
        )
        verify(capture)

    score.capture_original = capture_original
    return score


def validate_textures(capture, cases, sha):
    referenced = set()

    def visit(v):
        if isinstance(v, dict):
            if "pixels_file" in v:
                p = Path(v["pixels_file"])
                assert p.parent == capture / "textures" and p.suffix == ".pixels"
                assert p.is_file() and not p.is_symlink()
                if p not in referenced:
                    assert sha(p) == p.stem
                rect = v["rect"]
                assert len(rect) == 4 and all(type(n) is int for n in rect)
                w, h = rect[2] - rect[0], rect[3] - rect[1]
                assert v["kind"] in {"rgba", "nrgba", "alpha"}
                channels = 1 if v["kind"] == "alpha" else 4
                assert w > 0 and h > 0 and v["stride"] >= w * channels
                assert p.stat().st_size >= (h - 1) * v["stride"] + w * channels
                referenced.add(p)
            for child in v.values():
                visit(child)
        elif isinstance(v, list):
            for child in v:
                visit(child)

    visit(cases)
    assert referenced and referenced == set((capture / "textures").iterdir())


if __name__ == "__main__":
    base.scoreboard = scoreboard
    base.main()
