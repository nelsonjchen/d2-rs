#!/usr/bin/env python3
"""Observe the six original paint-order tests, preserving assertions and inputs.

Complete permutations, all unit/batch fields authored by these tests, identity
relations, stats, cancellation and errors compare. Allocation addresses and
unused svgPaintBatch rendering fields are outside this adapter's contract.
"""

import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "order_observer_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = {
        "TestSVGPaintOrderLargePlanesCannotCoverNearLabels": 3,
        "TestSVGPaintOrderIndexesFragmentedSurfacesWithinBudget": 1,
        "TestSVGPaintOrderUsesPolygonOverlapAndCoplanarSourceOrder": 3,
        "TestSVGPaintOrderPreservesAtomicGroupsAndDecalOrder": 1,
        "TestSVGPaintOrderCyclesRetainExternalDepthConstraints": 2,
        "TestSVGPaintOrderCancellationAndWorkBudget": 2,
    }
    score.OPS = {name: {"order": n} for name, n in score.TESTS.items()}
    score.COMPARISON_CONTRACT = {
        "compared": [
            "all twelve original calls and all original Go assertions",
            "complete painter permutation and unit/batch equality relations",
            "all test-authored unit/batch values, polygon coordinates and decal order",
            "complete errors, cancellation and statistics consumed by original tests",
            "explicit fragmented-surface work allowance",
        ],
        "unqualified": [
            "Go allocation addresses",
            "unused batch rendering fields",
            "partial statistics discarded by original error-only calls",
        ],
        "original_go_assertions": "all retained and required to pass",
    }
    package = "d2renderers/d2isometric/d2isometricimg"
    source_files = [
        "svg_order_test.go",
        "svg_order.go",
        "svg.go",
        "svg_visibility.go",
        "svg_visibility_tree.go",
        "svg_fragments.go",
    ]
    template = score.ROOT / "tools/isometric-order-scoreboard/hook_test.go"

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
        path = folder / "svg_order_test.go"
        original = path.read_bytes()
        assert original == (pinned / package / path.name).read_bytes()
        hook = folder / "d2rs_order_api_test.go"
        assert not hook.exists()
        text = original.decode()
        for name in score.TESTS:
            match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
            assert match
            body = match[0].replace("{\n", "{\n\td2rsOrderStart(t)\n", 1)
            body = body.replace("svgOrderPaintUnits(", "d2rsOrderObserve(")
            body = body.replace("svgOrderPaintUnitsWithLimits(", "d2rsOrderWithLimits(")
            # Observe precisely the error-only contract at this original call;
            # its ignored statistics are not available in Rust's Result API.
            body = body.replace(
                "if _, err := d2rsOrderWithLimits(",
                "if _, err := d2rsOrderDiscardStats(",
            )
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
