#!/usr/bin/env python3
"""Observe original surface painter, allocation and layout contracts.

All original Go assertions remain active. Complete texture pixels, layouts,
retained SVG fragments and errors compare without normalization. Limits:
1 GiB total, 128 MiB/file, 20 GiB free and finite subprocess deadlines.
"""

import argparse
import collections
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path


def scoreboard():
    spec = spec_from_file_location(
        "surface_base", Path(__file__).with_name("isometric-scene-scoreboard.py")
    )
    score = module_from_spec(spec)
    spec.loader.exec_module(score)
    package = "d2renderers/d2isometric/d2isometricimg"
    score.TESTS = dict.fromkeys(
        [
            "TestNativeTextPreservesSourceFontSizeAndMultiline",
            "TestNativeTextStylesAreDeterministicAndUseBundledFaces",
            "TestNativeTextTextureAndGlyphBudgets",
            "TestNativeTextClippingAndCancellation",
            "TestRichSurfaceFittingDoesNotEnlargeSourceText",
            "TestNativePlainLabelKeepsLongContentAndAuthoredWhitespace",
            "TestNativeIconRoundedCornersAreCachedSeparately",
            "TestNativeTextColorEmojiAndUnavailableGlyphError",
            "TestNativeSurfaceTextureDensityPreservesAggregateBudget",
            "TestSurfaceIconSVGAspectAlphaAndCache",
            "TestSurfaceIconCancellationAndBudgets",
            "TestSurfaceIconRejectsExternalSVGReferences",
            "TestSurfaceIconLayoutStaysOnFace",
            "TestSurfaceIconDimensionsExtremeAspect",
            "TestSurfaceIconReservesCompiledLabelAndWideImage",
            "TestSurfaceIconEmptyStructuredHeaderRetainsRows",
            "TestOutputDensityTextPreservesPhysicalLayout",
            "TestOutputDensityRichLabelsAndIcons",
            "TestRichSurfaceRetainsInlineLinkDocument",
            "TestOutputDensityTextureSharesStayBounded",
            "TestNativeVectorTextRetainsCurvesAndTypography",
            "TestNativeVectorRichTextRetainsCurves",
            "TestNativeVectorColorEmojiStaysVector",
            "TestNativeVectorIconsPreserveArtworkAndFirstRasterFrame",
        ],
        None,
    )
    test_files = [
        "text_test.go",
        "rich_labels_test.go",
        "surface_fidelity_test.go",
        "surface_icons_test.go",
        "output_density_test.go",
        "vector_surface_test.go",
    ]
    definitions = {
        "text.go": ["newTextPainter", "(p *textPainter) texture"],
        "rich_labels.go": [
            "newRichLabelPainter",
            "(p *richLabelPainter) texture",
            "fitRichViewport",
        ],
        "surface_icons.go": [
            "newSurfaceIconPainter",
            "(p *surfaceIconPainter) texture",
            "surfaceIconDimensions",
            "surfaceIconLayout",
        ],
    }
    # Only direct scalar calls in the selected tests are observed. Internal
    # allocation/fit calls remain ordinary production calls, preventing nested
    # event bookkeeping from changing painter call order.
    template = score.ROOT / "tools/isometric-surface-scoreboard/hook.go"

    def source():
        pinned = score.references.verify_source()
        names = (
            test_files
            + list(definitions)
            + ["surface_document.go", "vector_surface.go", "vector_surface_svg.go"]
        )
        return {
            "commit": score.references.SOURCE_COMMIT,
            "files": {
                package + "/" + name: score.sha(pinned / package / name)
                for name in names
            },
            "hook": score.sha(template),
        }

    score.source = source
    score.EXPECTED_OPERATIONS = {
        "TestNativeIconRoundedCornersAreCachedSeparately": {"new_icon": 1, "icon": 3},
        "TestNativePlainLabelKeepsLongContentAndAuthoredWhitespace": {
            "new_text": 1,
            "text": 1,
        },
        "TestNativeSurfaceTextureDensityPreservesAggregateBudget": {"dimensions": 26},
        "TestNativeTextClippingAndCancellation": {
            "new_text": 2,
            "text": 2,
            "fit_text": 3,
        },
        "TestNativeTextColorEmojiAndUnavailableGlyphError": {
            "new_text": 1,
            "new_icon": 2,
            "text": 2,
        },
        "TestNativeTextPreservesSourceFontSizeAndMultiline": {"new_text": 1, "text": 2},
        "TestNativeTextStylesAreDeterministicAndUseBundledFaces": {
            "new_text": 5,
            "new_icon": 10,
            "text": 10,
        },
        "TestNativeTextTextureAndGlyphBudgets": {
            "new_text": 3,
            "text": 8,
            "glyph_work": 1,
        },
        "TestNativeVectorColorEmojiStaysVector": {
            "new_text": 1,
            "new_icon": 1,
            "text": 1,
        },
        "TestNativeVectorIconsPreserveArtworkAndFirstRasterFrame": {
            "new_icon": 1,
            "icon": 2,
        },
        "TestNativeVectorRichTextRetainsCurves": {
            "new_rich": 2,
            "new_icon": 2,
            "rich": 2,
        },
        "TestNativeVectorTextRetainsCurvesAndTypography": {
            "new_text": 3,
            "new_icon": 3,
            "text": 3,
        },
        "TestOutputDensityRichLabelsAndIcons": {
            "new_rich": 2,
            "new_icon": 3,
            "rich": 2,
            "icon": 3,
        },
        "TestOutputDensityTextPreservesPhysicalLayout": {"new_text": 2, "text": 2},
        "TestOutputDensityTextureSharesStayBounded": {"dimensions": 505},
        "TestRichSurfaceFittingDoesNotEnlargeSourceText": {"fit_rich": 2},
        "TestRichSurfaceRetainsInlineLinkDocument": {
            "new_rich": 1,
            "new_icon": 1,
            "rich": 2,
        },
        "TestSurfaceIconCancellationAndBudgets": {
            "new_icon": 8,
            "icon": 4,
            "icon_source_budget": 1,
        },
        "TestSurfaceIconDimensionsExtremeAspect": {"icon_dimensions": 4},
        "TestSurfaceIconEmptyStructuredHeaderRetainsRows": {"icon_layout": 3},
        "TestSurfaceIconLayoutStaysOnFace": {"icon_layout": 2250},
        "TestSurfaceIconRejectsExternalSVGReferences": {"new_icon": 1, "icon": 1},
        "TestSurfaceIconReservesCompiledLabelAndWideImage": {
            "icon_layout": 2,
            "new_text": 1,
            "text": 1,
        },
        "TestSurfaceIconSVGAspectAlphaAndCache": {"new_icon": 1, "icon": 3},
    }

    def validate_cases(cases):
        import base64

        assert {c["name"] for c in cases} == set(score.TESTS) and len(cases) == len(
            score.TESTS
        )
        for case in cases:
            assert (
                collections.Counter(
                    call["request"]["op"] for call in case["operations"]
                )
                == score.EXPECTED_OPERATIONS[case["name"]]
            ), case["name"]
            for call in case["operations"]:
                assert isinstance(call["request"]["op"], str)
                got = call["got"]
                assert isinstance(got["error"], str)
                frame = got.get("frame")
                if frame is not None:
                    assert 0 < frame["width"] <= 4096 and 0 < frame["height"] <= 4096
                    assert frame["stride"] == frame["width"] * 4
                    pixels = base64.b64decode(frame["pixels"], validate=True)
                    assert len(pixels) == frame["stride"] * frame["height"]
                    assert all(
                        max(pixels[i : i + 3]) <= pixels[i + 3]
                        for i in range(0, len(pixels), 4)
                    )

    score.validate_cases = validate_cases

    def capture_original(capture):
        pinned = score.references.verify_source()
        folder = score.references.TRACE_CHECKOUT / package
        originals = {
            folder / name: (folder / name).read_bytes()
            for name in test_files + list(definitions)
        }
        hook = folder / "d2rs_surface_observer.go"
        assert not hook.exists()
        replacements = {}
        for path, original in originals.items():
            assert original == (pinned / package / path.name).read_bytes()
            text = original.decode()
            if path.name in definitions:
                for definition in definitions[path.name]:
                    old = "func " + definition + "("
                    receiver, _, name = definition.rpartition(" ")
                    new = (
                        "func "
                        + (receiver + " " if receiver else "")
                        + "d2rsOriginal"
                        + name[0].upper()
                        + name[1:]
                        + "("
                    )
                    assert text.count(old) == 1, old
                    text = text.replace(old, new)
            else:
                for name in score.TESTS:
                    match = re.search(
                        r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text
                    )
                    if not match:
                        continue
                    body = match[0].replace("{\n", "{\n\td2rsSurfaceStart(t)\n", 1)
                    for function in [
                        "fitNativeLabel",
                        "fitRichViewport",
                        "surfaceTextureDimensionsAtDensity",
                        "surfaceTextureDimensions",
                        "surfaceIconDimensions",
                    ]:
                        body = body.replace(
                            function + "(",
                            "d2rs" + function[0].upper() + function[1:] + "(",
                        )
                    body = body.replace(
                        "p.glyphWork = maxTextGlyphWork",
                        "d2rsTextGlyphWork(p, maxTextGlyphWork)",
                    )
                    body = body.replace(
                        "p.remaining.MaxSourceBytes = len(surfaceTestSVG)*2 - 1",
                        "d2rsIconSourceBudget(p, len(surfaceTestSVG)*2 - 1)",
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
        D2RS_SURFACE_REQUEST=str(request.resolve()),
        D2RS_SURFACE_OUTPUT=str(actual.resolve()),
    )
    run = score.module("cli-scoreboard").bounded_run(
        [
            str(args.rust.resolve()),
            "--exact",
            "img::text::api_tests::replay_original_surface_calls",
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
                "all observed constructor and texture calls, full premultiplied RGBA buffers, layouts, complete errors",
                "optional retained vector SVG, pointer reuse identities, direct allocation and icon-layout calls",
            ],
            "original_go_assertions": "all retained and required to pass",
            "unqualified": [
                "Go font cache internals and allocation addresses",
                "whole renderer/mesh assertions beyond the named painter observations",
                "rasterVectorFragment private reimport comparisons remain original Go assertions",
            ],
        },
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
