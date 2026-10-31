#!/usr/bin/env python3
"""Observe original low-level raster calls with complete premultiplied frames.

The original Go assertions remain active. Every frame has a complete bounded
pixel inventory; errors and ordered constructor/frame calls compare exactly.
Captures use 1 GiB total / 128 MiB file / 20 GiB free guards and deadlines.
"""

import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "raster_api_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestNativeRasterDepthAndStaticOwnership": 8,
        "TestNativeRasterSharedEdgeHasSingleSampleOwner": 2,
        "TestNativeRasterPremultipliedLabelEdges": 2,
        "TestNativeRasterTranslucentBackfaces": 2,
        "TestNativeRasterMultiplyGroupsAndRaisedGeometry": 6,
        "TestNativeRasterMultiplyOpacityAndTexture": 6,
        "TestNativeRasterNormalMaterialOverUnchanged": 2,
    }
    score.OPS = {
        name: {"new": n, **({"frame": count - n} if count > n else {})}
        for (name, count), n in zip(
            score.TESTS.items(), [2, 2, 1, 1, 2, 3, 1], strict=True
        )
    }
    score.COMPARISON_CONTRACT = {
        "compared": [
            "all original NewRaster and Frame calls in seven functions",
            "complete ordered premultiplied RGBA frame bytes and errors",
            "static camera, dynamic occlusion and output ownership across calls",
        ],
        "unqualified": [
            "private texture-sampler scalar assertion (original Go assertion retained)",
            "cache performance, concurrent Frame calls and arbitrary material mutation",
            "polymorphic or malformed Go image storage, private compositor and scanline APIs",
        ],
        "original_go_assertions": "all retained and required to pass",
    }
    package = "d2renderers/d2isometric/d2isometricimg"
    test_files = ["raster_test.go", "raster_multiply_test.go"]
    source_files = test_files + ["raster.go", "lighting.go"]
    template = score.ROOT / "tools/isometric-raster-api-scoreboard/hook_test.go"

    def source():
        pinned = score.references.verify_source()
        return {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + p: score.sha(pinned / package / p) for p in source_files
            },
            "hook": score.sha(template),
        }

    score.source = source
    base_validate = score.validate_cases

    def validate_cases(cases):
        import base64

        base_validate(cases)
        for case in cases:
            ids = set()
            for operation in case["operations"]:
                request, got = operation["request"], operation["got"]
                assert request["op"] in {"new", "frame"}
                if request["op"] == "new":
                    assert request["id"] not in ids and request["id"] > 0
                    ids.add(request["id"])
                else:
                    assert request["id"] in ids
                frame = got["frame"]
                assert got["error"] == "" and frame is not None
                assert 0 < frame["width"] <= 4096 and 0 < frame["height"] <= 4096
                assert frame["stride"] == frame["width"] * 4
                pixels = base64.b64decode(frame["pixels"], validate=True)
                assert len(pixels) == frame["stride"] * frame["height"]

    score.validate_cases = validate_cases

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {folder / p: (folder / p).read_bytes() for p in test_files}
        hook = folder / "d2rs_raster_api_test.go"
        assert not hook.exists()
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            for name in score.TESTS:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                if not match:
                    continue
                body = match[0].replace("{\n", "{\n\td2rsRasterStart(t)\n", 1)
                body = body.replace("NewRaster(", "d2rsRasterNew(")
                body = re.sub(r"\b(\w+)\.Frame\(", r"d2rsRasterFrame(\1, ", body)
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
        )
        try:
            for path, text in replacements.items():
                path.write_text(text)
            hook.write_bytes(template.read_bytes())
            result = score.module("cli-scoreboard").bounded_run(
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
            for path, original in originals.items():
                path.write_bytes(original)
            hook.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(result) + "\n")
        assert result == {"exit": 0, "failure": None}, result
        (capture / "hashes.json").write_text(
            json.dumps(score.hashes(capture), indent=2) + "\n"
        )
        score.verify(capture)

    score.capture_original = capture_original
    return score


if __name__ == "__main__":
    scoreboard().main()
