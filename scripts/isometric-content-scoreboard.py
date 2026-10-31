#!/usr/bin/env python3
"""Combined original-test observation of native content/retained-surface contracts.

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
    "readability_test.go": [
        "TestNativeReadingAxisAndForeshortening",
        "TestNativeRouteStylingPreservesAuthoredPaint",
        "TestNativeSlopedTubeAndCasing",
        "TestNativeCasingDoesNotRevealTransparentStroke",
        "TestNativeDashPreservesBridgeAndCornerVertices",
    ],
    "presentation_quality_test.go": [
        "TestMatteLightingPreservesTopColorAndRevealsSidewalls",
        "TestPrintedRouteInkKeepsColorAndPlaneThroughTurns",
        "TestReliefSymbolsStayInsideAuthoredContourBounds",
    ],
    "surface_content_test.go": [
        "TestMarkdownSurfaceUsesRaisedPaperWithinSourceAllocation",
        "TestLaneSubdivisionPreservesCaptionPrintSize",
    ],
    "output_density_test.go": ["TestOutputDensityNativeFaceKeepsSourceArea"],
    "surface_fidelity_test.go": [
        "TestNativePlanarDocumentFrameIsDeterministicAndBounded",
        "TestNativeSurfaceEmbeddedAssetsOnlyAndTheme",
    ],
    "vector_surface_test.go": [
        "TestNativeVectorCaptureIsScopedAndCancelable",
        "TestNativeVectorBorderAperturePreservesSourceOpening",
        "TestNativeVectorInitialAnimationPreservesStaticFrame",
        "TestNativeVectorSplitPaintCoverageAppliesOpacityOnce",
    ],
    "vector_surface_mask_test.go": ["TestNativeVectorCompensationMaskHasWhiteRGB"],
    "vector_surface_svg_precision_test.go": [
        "TestNativeSVGMatrixPreservesAuthoredPrecision"
    ],
    "vector_surface_cap_test.go": [
        "TestNativeVectorSolidCapClosesOnlyOpaqueSourceMargin"
    ],
}
CALLS = {
    "nativeSurfaceSVG": "contentSVG",
    "nativeVectorAperture": "contentAperture",
    "nativeVectorSolidCap": "contentCap",
    "rasterNativeSurfaceDocument": "contentRaster",
    "nativeSVGMatrix": "contentMatrix",
    "nativeViewDirection": "contentView",
    "nativeRouteTint": "contentTint",
    "nativeRouteSection": "contentSection",
    "nativeSurfaceDocument": "contentDocument",
}
METHODS = {
    "node": "Node",
    "edges": "Edges",
    "routeInk": "Ink",
    "tube": "Tube",
    "routeCasing": "Casing",
    "nativeFace": "Face",
    "shade": "Shade",
    "nativeFaceOpacity": "Opacity",
}

EXPECTED_OPERATIONS = {
    "TestLaneSubdivisionPreservesCaptionPrintSize": {"edges": 2},
    "TestMarkdownSurfaceUsesRaisedPaperWithinSourceAllocation": {"node": 1},
    "TestMatteLightingPreservesTopColorAndRevealsSidewalls": {"shade": 6},
    "TestNativeCasingDoesNotRevealTransparentStroke": {"edges": 1},
    "TestNativeDashPreservesBridgeAndCornerVertices": {"section": 1},
    "TestNativePlanarDocumentFrameIsDeterministicAndBounded": {"raster_doc": 4},
    "TestNativeReadingAxisAndForeshortening": {"view": 1},
    "TestNativeRouteStylingPreservesAuthoredPaint": {"tint": 4},
    "TestNativeSVGMatrixPreservesAuthoredPrecision": {"matrix": 7},
    "TestNativeSlopedTubeAndCasing": {"casing": 1, "tube": 1},
    "TestNativeSurfaceEmbeddedAssetsOnlyAndTheme": {"document": 2, "raster_doc": 1},
    "TestNativeVectorBorderAperturePreservesSourceOpening": {
        "aperture": 1,
        "raster_doc": 1,
        "svg": 2,
    },
    "TestNativeVectorCaptureIsScopedAndCancelable": {"svg": 2},
    "TestNativeVectorCompensationMaskHasWhiteRGB": {"face_opacity": 1, "svg": 2},
    "TestNativeVectorInitialAnimationPreservesStaticFrame": {"raster_doc": 1, "svg": 2},
    "TestNativeVectorSolidCapClosesOnlyOpaqueSourceMargin": {"svg": 3, "cap": 2},
    "TestNativeVectorSplitPaintCoverageAppliesOpacityOnce": {
        "face_opacity": 3,
        "cap": 3,
        "svg": 3,
    },
    "TestOutputDensityNativeFaceKeepsSourceArea": {"face": 3},
    "TestPrintedRouteInkKeepsColorAndPlaneThroughTurns": {"ink": 1, "shade": 18},
    "TestReliefSymbolsStayInsideAuthoredContourBounds": {"node": 3},
}


def load_original(path):
    # Go encodes a float's negative zero as -0, which Python otherwise reads
    # as integer zero. Preserve that authored sign through request transport.
    return json.loads(
        path.read_text(), parse_int=lambda text: -0.0 if text == "-0" else int(text)
    )


def scoreboard():
    spec = spec_from_file_location(
        "content_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    score.TESTS = dict.fromkeys(
        [name for names in TEST_FILES.values() for name in names], None
    )
    package = "d2renderers/d2isometric/d2isometricimg"
    templates = [
        score.ROOT / "tools/isometric-content-scoreboard" / name
        for name in ["hook_test.go", "scenejson.go"]
    ]

    def source():
        pinned = score.references.verify_source()
        names = list(TEST_FILES) + [
            "vector_surface.go",
            "vector_surface_svg.go",
            "surface_document.go",
            "native_shapes.go",
            "lighting.go",
            "route_style.go",
            "geometry_routes.go",
        ]
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
            assert (
                collections.Counter(o["request"]["op"] for o in c["operations"])
                == EXPECTED_OPERATIONS[c["name"]]
            ), c["name"]
            for o in c["operations"]:
                assert o["request"]["op"] in set(CALLS.values()) | {
                    "svg",
                    "cap",
                    "raster_doc",
                    "matrix",
                    "view",
                    "tint",
                    "section",
                    "document",
                    "node",
                    "edges",
                    "ink",
                    "tube",
                    "casing",
                    "face",
                    "aperture",
                    "face_opacity",
                    "shade",
                }
                assert isinstance(o["got"]["error"], str)
                validate_images(o["got"])

    score.validate_cases = validate_cases
    original_hashes = score.hashes

    def hashes(capture):
        values = original_hashes(capture)
        values.update(
            {
                str(p.relative_to(capture)): score.sha(p)
                for p in sorted((capture / "assets").glob("*"))
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
        return cases

    score.verify = verify

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {folder / name: (folder / name).read_bytes() for name in TEST_FILES}
        hooks = {folder / ("d2rs_content_" + p.name): p for p in templates}
        assert all(not p.exists() for p in hooks)
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            # Observe the shared vector-fragment helper as well as all selected
            # direct calls, while preserving every original assertion and subtest.
            for a, b in CALLS.items():
                text = text.replace(a + "(", b + "(")
            for name in TEST_FILES[path.name]:
                match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
                assert match, name
                body = match[0].replace("{\n", "{\n\tcontentStart(t)\n", 1)
                for a, b in METHODS.items():
                    body = re.sub(
                        r"\b(\w+)\." + a + r"\(", r"content" + b + r"(\1, ", body
                    )
                body = body.replace("contentOpacity(b,", "contentOpacity(&b,").replace(
                    "contentOpacity(builder,", "contentOpacity(&builder,"
                )
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


def validate_images(value):
    import base64

    if isinstance(value, dict):
        if "pixels" in value or "stride" in value:
            assert set(value) == {"width", "height", "stride", "pixels"}
            w, h = value["width"], value["height"]
            assert (
                isinstance(w, int)
                and isinstance(h, int)
                and 0 < w <= 8192
                and 0 < h <= 8192
            )
            assert value["stride"] == 4 * w
            p = base64.b64decode(value["pixels"], validate=True)
            assert len(p) == w * h * 4
            assert all(max(p[i : i + 3]) <= p[i + 3] for i in range(0, len(p), 4))
        for v in value.values():
            validate_images(v)
    elif isinstance(value, list):
        for v in value:
            validate_images(v)


def validate_actual(directory, cases):
    expected_files = {str(i) + ".json" for i in range(len(cases))}
    assert (
        cases
        and directory.is_dir()
        and {p.name for p in directory.iterdir()} == expected_files
    )
    values = [
        json.loads((directory / (str(i) + ".json")).read_text())
        for i in range(len(cases))
    ]
    assert all(
        isinstance(value, list) and len(value) == len(case["operations"])
        for value, case in zip(values, cases, strict=True)
    )
    for value in values:
        for got in value:
            validate_images(got)
    return values


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
        D2RS_CONTENT_REQUEST=str(request.resolve()),
        D2RS_CONTENT_OUTPUT=str(actual.resolve()),
    )
    run = score.module("cli-scoreboard").bounded_run(
        [
            str(args.rust.resolve()),
            "--exact",
            "img::content_api_tests::replay_original_content_calls",
            "--ignored",
            "--nocapture",
        ],
        env,
        work,
        work / "test.log",
        180,
    )
    output = None
    if run == {"exit": 0, "failure": None}:
        output = validate_actual(actual, cases)
    rows = []
    actual_hashes = (
        {str(p.relative_to(work)): score.sha(p) for p in sorted(actual.glob("*.json"))}
        if output is not None
        else {}
    )
    for i, c in enumerate(cases):
        expected = [o["got"] for o in c["operations"]]
        got = output[i] if output is not None else None
        outcome = score.outcome(expected, got)
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
        "comparison_contract": {
            "compared": [
                "complete observed mesh geometry/materials/textures and painter results",
                "complete retained SVG fragments, frame pixels, source documents and errors",
            ],
            "original_go_assertions": "all retained and required to pass",
            "unqualified": [
                "unobserved Go registry isolation and allocation internals",
                "private source-vector reimport tolerance checks remain original Go assertions",
                "packet internals beyond emitted mesh remain original Go assertions",
            ],
        },
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
