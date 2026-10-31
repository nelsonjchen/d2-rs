#!/usr/bin/env python3
"""Replay eight original SVG visibility functions with complete fragments/errors.

All Go assertions remain active. Nonfinite input scalars use explicit tags, not
JSON null; identity IDs retain group/owner equality. Limits: 1 GiB total,
128 MiB per file, 20 GiB free, finite native/Go deadlines.
"""

import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "visibility_observer_base",
        Path(__file__).with_name("isometric-scene-scoreboard.py"),
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestSVGVisibilityCutsCrossingDepthPlanes": 1,
        "TestSVGVisibilitySubtractsUnionWithoutOverlappingFragments": 1,
        "TestSVGVisibilitySourceOpacityGroupsAndDecals": 1,
        "TestSVGVisibilityCoplanarPaintOrderAndDepthBias": 3,
        "TestSVGVisibilityContourKeepsItsOwnSubstrateUnderTheInk": 3,
        "TestSVGVisibilityMatchesDepthOracle": 1,
        "TestSVGVisibilityGridAndTranslationStayDeterministic": 2,
        "TestSVGVisibilityBoundsCancellationAndDegenerates": 15,
    }
    score.OPS = {name: {"visibility": n} for name, n in score.TESTS.items()}
    score.OPS["TestSVGVisibilityBoundsCancellationAndDegenerates"] = {
        "visibility": 8,
        "work_limit": 7,
    }
    score.COMPARISON_CONTRACT = {
        "compared": [
            "all original visibility calls and work allowances",
            "complete ordered fragments and errors",
            "canceled contexts and nonfinite typed coordinates",
            "group and owner equality relations",
        ],
        "unqualified": ["Go allocation addresses"],
        "original_go_assertions": "all retained and required to pass",
    }
    package = "d2renderers/d2isometric/d2isometricimg"
    source_files = [
        "svg_visibility_test.go",
        "svg_visibility.go",
        "svg_visibility_tree.go",
        "svg_fragments.go",
    ]
    template = score.ROOT / "tools/isometric-visibility-scoreboard/hook_test.go"

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

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        path = folder / "svg_visibility_test.go"
        original = path.read_bytes()
        assert original == (pinned / package / path.name).read_bytes()
        hook = folder / "d2rs_visibility_api_test.go"
        assert not hook.exists()
        text = original.decode()
        for name in score.TESTS:
            match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
            assert match
            body = match[0].replace("{\n", "{\n\td2rsVisibilityStart(t)\n", 1)
            body = body.replace("svgVisibleFaces(", "d2rsVisibilityObserve(")
            body = body.replace(
                "svgVisibleFacesWithLimits(", "d2rsVisibilityWithLimits("
            )
            body = body.replace("svgVisibilityWorkLimit(", "d2rsVisibilityWorkLimit(")
            text = text[: match.start()] + body + text[match.end() :]
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
