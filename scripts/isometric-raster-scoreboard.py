#!/usr/bin/env python3
"""Original-test observation of native raster admission, precision and antialiasing.

Limits: 1 GiB total, 128 MiB/file, 20 GiB free; finite Go/native deadlines.
"""

import argparse
import collections
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

TEST_FILES = {
    "raster_test.go": [
        "TestNativeRasterAdmissionAndCancellation",
        "TestNativeRasterRejectsOverflowingTextureStorage",
        "TestNativeRasterScanlineCoverage",
    ],
    "raster_aa_test.go": [
        "TestNativeAACoverageAcrossFormerSizeCutoff",
        "TestNativeStripReplayMatchesCachedDepthAndAlpha",
        "TestNativeAAMaximumCanvas",
        "TestNativeAACaptureCameraAboveCutoff",
        "TestNativeDeferredSnapshotRetainsTexturePrecision",
        "TestNativeGroundFootprintPreservesEveryShadowSample",
    ],
    "raster_multiply_test.go": ["TestRasterMultiplyPremultipliedBackdrop"],
}
CALLS = {
    "NewRaster": "rasterObserveNew",
    "newRaster": "rasterObserveConstruct",
    "rasterTexture": "rasterObserveSample",
    "rasterSnapshot": "rasterObserveSnapshot",
    "rasterMultiplyOver": "rasterObserveMultiply",
    "rasterRowSpan": "rasterObserveSpan",
}
EXPECTED_OPERATIONS = {
    "TestNativeRasterAdmissionAndCancellation": {"render": 11, "charge": 1},
    "TestNativeRasterRejectsOverflowingTextureStorage": {"render": 4},
    "TestNativeRasterScanlineCoverage": {"span": 15113},
    "TestNativeAACoverageAcrossFormerSizeCutoff": {"render": 2},
    "TestNativeStripReplayMatchesCachedDepthAndAlpha": {"render": 6},
    "TestNativeAAMaximumCanvas": {"render": 2},
    "TestNativeAACaptureCameraAboveCutoff": {"render": 1},
    "TestNativeDeferredSnapshotRetainsTexturePrecision": {"snapshot": 1, "sample": 7},
    "TestNativeGroundFootprintPreservesEveryShadowSample": {"render": 1},
    "TestRasterMultiplyPremultipliedBackdrop": {"multiply": 4},
}


def load_original(path):
    # Go encodes a float's negative zero as -0, which Python otherwise reads
    # as integer zero. Preserve that authored sign through request transport.
    return json.loads(
        path.read_text(), parse_int=lambda text: -0.0 if text == "-0" else int(text)
    )


def scoreboard():
    spec = spec_from_file_location(
        "raster_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = dict.fromkeys(
        [name for names in TEST_FILES.values() for name in names], None
    )
    package = "d2renderers/d2isometric/d2isometricimg"
    # Reuse the exact existing comparator without re-importing its module for
    # every one of the source test's 15,113 scanline observations.
    comparison = score.module("language-scoreboard").differences

    def outcome(expected, got):
        if got is None:
            return "rust-failed"
        if isinstance(got, dict) and "unsupported" in got:
            return "unsupported"
        return "mismatch" if comparison(expected, got) else "match"

    score.outcome = outcome
    templates = [
        score.ROOT / "tools/isometric-raster-scoreboard" / name
        for name in ["hook_test.go"]
    ]

    def source():
        pinned = score.references.verify_source()
        names = list(TEST_FILES) + ["raster.go", "lighting.go", "framing.go"]
        return {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + p: score.sha(pinned / package / p) for p in names
            },
            "hooks": {p.name: score.sha(p) for p in templates},
        }

    score.source = source

    def validate_cases(cases):
        assert len(cases) == len(score.TESTS) and {c["name"] for c in cases} == set(
            score.TESTS
        )
        for c in cases:
            counts = collections.Counter(o["request"]["op"] for o in c["operations"])
            expected = EXPECTED_OPERATIONS[c["name"]]
            assert set(counts) == set(expected), (c["name"], counts)
            assert counts == expected, (c["name"], counts)
            assert all(isinstance(o["got"]["error"], str) for o in c["operations"])

    score.validate_cases = validate_cases
    original_hashes = score.hashes

    def hashes(capture):
        values = original_hashes(capture)
        values.update(
            {
                str(p.relative_to(capture)): score.sha(p)
                for p in sorted((capture / "images").glob("*.rgba"))
                if p.is_file()
            }
        )
        return values

    score.hashes = hashes

    original_verify = score.verify

    def verify(capture):
        original_verify(capture)
        cases = [load_original(p) for p in sorted((capture / "go").glob("*.json"))]
        validate_cases(cases)
        validate_image_store(
            capture / "images", [o["got"] for c in cases for o in c["operations"]]
        )
        return cases

    score.verify = verify

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {folder / name: (folder / name).read_bytes() for name in TEST_FILES}
        hooks = {folder / ("d2rs_raster_" + p.name): p for p in templates}
        assert all(not p.exists() for p in hooks)
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            for name in TEST_FILES[path.name]:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                assert match, name
                body = match[0].replace("{\n", "{\n\trasterObserveStart(t)\n", 1)
                for a, b in CALLS.items():
                    body = re.sub(r"\b" + a + r"\(", b + "(", body)
                body = re.sub(r"\b(\w+)\.Frame\(", r"rasterObserveFrame(\1, ", body)
                body = re.sub(r"\b(\w+)\.charge\(", r"rasterObserveCharge(&\1, ", body)
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
            for path, text in replacements.items():
                path.write_text(text)
            for path, template in hooks.items():
                path.write_bytes(template.read_bytes())
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
            for path, data in originals.items():
                path.write_bytes(data)
            for path in hooks:
                path.unlink(missing_ok=True)
        (capture / "run.json").write_text(json.dumps(result) + "\n")
        assert result == {"exit": 0, "failure": None}, result
        (capture / "hashes.json").write_text(
            json.dumps(hashes(capture), indent=2) + "\n"
        )
        score.verify(capture)

    score.capture_original = capture_original
    return score


def images(value):
    if isinstance(value, dict):
        if "sha256" in value:
            assert set(value) == {"width", "height", "stride", "sha256"}
            w, h = value["width"], value["height"]
            assert (
                isinstance(w, int)
                and isinstance(h, int)
                and 0 < w <= 8192
                and 0 < h <= 8192
            )
            assert value["stride"] == 4 * w and re.fullmatch(
                "[0-9a-f]{64}", value["sha256"]
            )
            yield value
        else:
            for v in value.values():
                yield from images(v)
    elif isinstance(value, list):
        for v in value:
            yield from images(v)


def validate_image_store(directory, values):
    import hashlib

    bindings = {}
    for v in images(values):
        key = v["sha256"] + ".rgba"
        expected = v["width"] * v["height"] * 4
        assert key not in bindings or bindings[key] == expected
        bindings[key] = expected
    assert bindings and directory.is_dir()
    assert {p.name for p in directory.iterdir()} == set(bindings)
    for name, size in bindings.items():
        p = directory / name
        assert p.is_file() and p.stat().st_size == size
        digest = hashlib.sha256()
        with p.open("rb") as stream:
            while chunk := stream.read(1 << 20):
                digest.update(chunk)
        assert digest.hexdigest() + ".rgba" == name
    return bindings


def validate_actual(directory, cases):
    expected = {str(i) + ".json" for i in range(len(cases))} | {"images"}
    assert (
        cases
        and directory.is_dir()
        and {p.name for p in directory.iterdir()} == expected
    )
    values = [
        json.loads((directory / (str(i) + ".json")).read_text())
        for i in range(len(cases))
    ]
    assert all(
        isinstance(v, list) and len(v) == len(c["operations"])
        for v, c in zip(values, cases, strict=True)
    )
    validate_image_store(directory / "images", values)
    return values


def case_outcome(score, expected, got):
    if got is None:
        return "rust-failed"
    if len(expected) != len(got):
        return "mismatch"
    outcomes = [score.outcome(a, b) for a, b in zip(expected, got, strict=True)]
    # An unsupported Go-only operation must never hide another call's error.
    if "mismatch" in outcomes:
        return "mismatch"
    return "unsupported" if "unsupported" in outcomes else "match"


def main():
    score = scoreboard()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        assert not capture.exists()
        score.capture_original(capture)
    cases = score.verify(capture)
    if not args.output:
        print("verified original functions", len(cases))
        return
    assert args.rust and args.rust.is_file() and not args.output.exists()
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, actual = work / "request.json", work / "actual"
    request.write_text(
        json.dumps(
            {
                "cases": [
                    {
                        "name": c["name"],
                        "operations": [o["request"] for o in c["operations"]],
                    }
                    for c in cases
                ]
            }
        )
    )
    digest = score.sha(args.rust)
    env = dict(
        os.environ,
        D2RS_RASTER_REQUEST=str(request.resolve()),
        D2RS_RASTER_OUTPUT=str(actual.resolve()),
    )
    run = score.module("cli-scoreboard").bounded_run(
        [
            str(args.rust.resolve()),
            "--exact",
            "img::raster::api_tests::replay_original_raster_calls",
            "--ignored",
            "--nocapture",
        ],
        env,
        work,
        work / "test.log",
        600,
    )
    output = None
    if run == {"exit": 0, "failure": None}:
        output = validate_actual(actual, cases)
    rows = []
    actual_hashes = (
        {
            str(p.relative_to(work)): score.sha(p)
            for p in sorted(actual.rglob("*"))
            if p.is_file()
        }
        if output is not None
        else {}
    )
    for i, c in enumerate(cases):
        expected = [o["got"] for o in c["operations"]]
        got = output[i] if output is not None else None
        outcome = case_outcome(score, expected, got)
        row = {
            "name": c["name"],
            "outcome": outcome,
            "operations": len(c["operations"]),
        }
        if outcome == "mismatch":
            row["differences"] = score.module("language-scoreboard").differences(
                expected, got
            )
        rows.append(row)
    assert score.sha(args.rust) == digest
    report = {
        "source_commit": score.references.SOURCE_COMMIT,
        "package": "d2renderers/d2isometric/d2isometricimg",
        "captured_functions": len(cases),
        "observed_operations": sum(r["operations"] for r in rows),
        "runner_sha256": digest,
        "capture_hashes": score.hashes(capture),
        "run": run,
        "actual_hashes": actual_hashes,
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
        "comparison_contract": getattr(
            score,
            "comparison_contract",
            {
                "compared": [
                    "complete observed raster pixels and image dimensions, texture samples, snapshot meshes, scanline spans and compositor results",
                    "complete errors; raw expected and actual image stores require complete nonempty inventories and verified pixel byte hashes",
                ],
                "original_go_assertions": "all retained and required to pass",
                "unqualified": [
                    "Go cache layout, forced-strip internals and allocation identities remain original Go assertions",
                    "Go typed nil images have no native image input representation",
                    "capture-camera internals beyond the observed legacy-camera render remain original Go assertions",
                ],
            },
        ),
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
