#!/usr/bin/env python3
"""Observe original BuildScene validation with typed, lossless input mutations.

Nonfinite numbers and invalid UTF-8 must never round-trip through JSON. The
capture stores the valid source and original operation name before mutation.
"""

import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "scene_admission_base",
        Path(__file__).with_name("isometric-scene-scoreboard.py"),
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    mutations = [
        "duplicate node",
        "nonfinite route",
        "nil route point",
        "nonfinite ratio",
        "negative size",
        "missing endpoint",
        "duplicate edge",
        "text budget",
        "node budget",
        "invalid UTF8",
        "ID size",
        "ID depth",
    ]
    score.TESTS = {"TestValidation/" + name.replace(" ", "_"): 1 for name in mutations}
    score.TESTS["TestValidation"] = 1  # the original nil-diagram call
    score.FUNCTION_COUNT = 1
    score.OPS = {name: {"validate": 1} for name in score.TESTS}
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete errors",
            "nil scene results",
            "original typed input mutations",
        ],
        "unqualified": ["nil Rust references", "invalid UTF-8 in native String"],
        "original_go_assertions": "all retained and required to pass",
    }
    original_source = score.source

    def source():
        result = original_source()
        result["hook"] = score.sha(
            score.ROOT / "tools/isometric-admission-scoreboard/hook_test.go"
        )
        return result

    score.source = source
    original_validate = score.validate_cases

    def validate_cases(cases):
        original_validate(cases)
        for case in cases:
            expected = case["name"].partition("/")[2].replace("_", " ") or "nil diagram"
            assert case["operations"][0]["request"]["mutation"] == expected

    score.validate_cases = validate_cases

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / "d2renderers/d2isometric"
        path = folder / "scene_test.go"
        original = path.read_bytes()
        assert (
            original == (pinned / "d2renderers/d2isometric/scene_test.go").read_bytes()
        )
        hook = folder / "d2rs_scene_admission_test.go"
        assert not hook.exists()
        text = original.decode()
        match = re.search(r"(?ms)^func TestValidation\(.*?(?=^func |\Z)", text)
        assert match
        body = match[0].replace("{\n", "{\n\td2rsAdmissionStart(t)\n", 1)
        assert body.count("tt.change(d)") == 1
        body = body.replace("tt.change(d)", "")
        body = body.replace(
            "BuildScene(d, nil)", "d2rsAdmissionBuild(t, d, tt.name, tt.change)"
        )
        body = body.replace(
            "BuildScene(nil, nil)", 'd2rsAdmissionBuild(t, nil, "nil diagram", nil)'
        )
        assert "BuildScene(" not in body
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
            hook.write_bytes(
                (
                    score.ROOT / "tools/isometric-admission-scoreboard/hook_test.go"
                ).read_bytes()
            )
            result = score.module("cli-scoreboard").bounded_run(
                [
                    "go",
                    "test",
                    "./d2renderers/d2isometric",
                    "-run",
                    "^TestValidation$",
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
