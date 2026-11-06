#!/usr/bin/env python3
"""Replay public image API calls observed from pinned original Go tests."""

import argparse
import base64
import collections
import importlib.util
import io
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import references

spec = importlib.util.spec_from_file_location(
    "iso", ROOT / "scripts/isometric-scoreboard.py"
)
iso = importlib.util.module_from_spec(spec)
spec.loader.exec_module(iso)
TESTS = [
    "TestPagePreservesImageAndProjectsTypedLinks",
    "TestPageMarkdownURLsStayDistinctFromBoardTargets",
    "TestPageLinkBudgetsAndCancellation",
    "TestPageRejectsMetadataThatWritersCannotPreserve",
    "TestPagePreservesInlineMarkdownOnEdgeCaptions",
    "TestNativeInteractiveExportsRejectUnsafeLinks",
]


SVG_TESTS = [
    "TestNativeSVGVectorGeometryAndText",
    "TestNativeSVGAdmissionAndCancellation",
    "TestNativeSVGLargeRichTextAndPaperStayWithinBudget",
    "TestD2rsSVGDeadlineBoards",
]

PNG_TESTS = [
    "TestAdmissionBeforeRendering",
    "TestContentFramingFitsFinalGeometryAndKeepsFixedAPI",
    "TestD2rsPNGDeadlineBoards",
]

PAINT_TESTS = [
    "TestNativeAuthoredGradientPaintRoles",
    "TestNativeContainerGradientRetainsStopsAndOpacity",
    "TestNativeSVGPreservesMagnifiedAuthoredIconTranslation",
    "TestSurfaceIconNativeRenderPreservesFootprint",
    "TestD2rsContainerGradientAdmissionBoard",
]

PANEL_TESTS = ["TestD2rsLegendAssetBoards"]
COMPOSITION_TESTS = [
    "TestNativeBuiltInLegendKeepsEveryVisibleEntry",
    "TestNativeSequencePreservesSourceCompositionAndAuthoredAnimation",
    "TestNativeRootLabelDescriptionAndFrameAreVisible",
]
FONT_TESTS = ["TestD2rsConfiguredFontAdmissionBoards"]
LINK_TESTS = [
    "TestNativeLinkDestinationDistinguishesBoardTargetsAndFiles",
    "TestOutsideMarkdownPageLinksFollowFinalSurfaces",
]
STILL_TESTS = ["TestNativeDeterministicCapture", "TestSVGVisibilityLargeSources"]
RICH_BOARD_TESTS = [
    "TestRichMarkdownPreservesFormattingAndLongContent",
    "TestRichCodeUsesSyntaxAndDarkPalette",
    "TestRichStructuredRowsAndOwnership",
    "TestRichOpacityBudgetsAndCancellation",
    "TestNativeLatexIsTypesetAndLabelBackingPreserved",
    "TestNativeSQLRowsRetainOriginalCoordinates",
]


def capture(path, suite="page"):
    tests = {
        "page": TESTS,
        "svg": SVG_TESTS,
        "png": PNG_TESTS,
        "paint": PAINT_TESTS,
        "panels": PANEL_TESTS,
        "composition": COMPOSITION_TESTS,
        "fonts": FONT_TESTS,
        "links": LINK_TESTS,
        "still": STILL_TESTS,
        "rich-boards": RICH_BOARD_TESTS,
    }[suite]
    assert (
        references.command("git", "rev-parse", "HEAD", cwd=references.TRACE_CHECKOUT)
        == references.SOURCE_COMMIT
    )
    package = references.TRACE_CHECKOUT / "d2renderers/d2isometric/d2isometricimg"
    hook = package / "d2rs_api_hook_test.go"
    assert not hook.exists()
    witness = {
        "svg": "svg_context_test.go",
        "png": "png_context_test.go",
        "paint": "gradient_admission_test.go",
        "panels": "panel_assets_test.go",
        "composition": "composition_test.go",
        "fonts": "font_admission_test.go",
        "links": "link_boards_test.go",
        "rich-boards": "rich_boards_test.go",
    }.get(suite)
    contract_hook = package / "d2rs_api_context_test.go"
    assert not contract_hook.exists()
    edits = []
    symbols = [("page.go", "RenderPage"), ("render.go", "Render")]
    if suite == "composition":
        symbols.append(("source_panels_test.go", "nativeFixtureScene"))
    for file, symbol in symbols:
        source = package / file
        original = source.read_bytes()
        assert (
            original
            == (
                iso.UPSTREAM / "d2renderers/d2isometric/d2isometricimg" / file
            ).read_bytes()
        )
        needle = f"func {symbol}(".encode()
        assert original.count(needle) == 1
        edits.append(
            (
                source,
                original,
                original.replace(needle, f"func d2rsOriginal{symbol}(".encode()),
            )
        )
    if suite == "links":
        for file, needle, replacement in [
            (
                "page_test.go",
                'nativeLinkDestination(tc.link, "details")',
                'd2rsLinkDestinationBoard(t, tc.link, "details")',
            ),
            (
                "animation_page_regression_test.go",
                "d2isometric.BuildScene(d, &d2isometric.RenderOpts{})",
                "d2rsOutsideLinkBoard(t, d, &d2isometric.RenderOpts{})",
            ),
        ]:
            source = package / file
            original = source.read_bytes()
            assert (
                original
                == (
                    iso.UPSTREAM / "d2renderers/d2isometric/d2isometricimg" / file
                ).read_bytes()
            )
            assert original.count(needle.encode()) == 1
            edits.append(
                (
                    source,
                    original,
                    original.replace(needle.encode(), replacement.encode()),
                )
            )
    if suite == "rich-boards":
        source = package / "rich_labels.go"
        original = source.read_bytes()
        assert (
            original
            == (
                iso.UPSTREAM / "d2renderers/d2isometric/d2isometricimg/rich_labels.go"
            ).read_bytes()
        )
        needle = b"doc, err := d2scenebuild.Build(ctx, diagram,"
        assert original.count(needle) == 1
        edits.append(
            (
                source,
                original,
                original.replace(
                    needle, b"doc, err := d2rsRichBoardBuild(ctx, diagram,"
                ),
            )
        )
    path.mkdir(parents=True)
    env = dict(
        os.environ,
        D2RS_ISOMETRIC_API=str(path / "calls.jsonl"),
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    try:
        if witness:
            contract_hook.write_bytes(Path(__file__).with_name(witness).read_bytes())
        hook.write_bytes(Path(__file__).with_name("api_hook_test.go").read_bytes())
        for source, _, changed in edits:
            source.write_bytes(changed)
        with (path / "go-events.jsonl").open("w") as log:
            command = [
                "env",
                *[f"{k}={v}" for k, v in env.items() if os.environ.get(k) != v],
                "go",
                "-C",
                str(references.TRACE_CHECKOUT),
                "test",
                "./d2renderers/d2isometric/d2isometricimg",
                "-json",
                "-count=1",
                "-run",
                "^(" + "|".join(tests) + ")$",
            ]
            code = iso.bounded_native(
                command, log, lambda: iso.direct_guard(path, 1 << 30)
            )
        assert code == 0, "original Go assertions failed"
    finally:
        for source, original, changed in edits:
            assert source.read_bytes() == changed
            source.write_bytes(original)
        hook.unlink()
        if witness:
            contract_hook.unlink()
    events = [
        json.loads(line) for line in (path / "go-events.jsonl").read_text().splitlines()
    ]
    passed = [e["Test"] for e in events if e.get("Action") == "pass" and "Test" in e]
    assert all(test in passed for test in tests)
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert (
        len(calls)
        == {
            "page": 48,
            "svg": 11,
            "png": 18,
            "paint": 34,
            "panels": 27,
            "composition": 15,
            "fonts": 18,
            "links": 54,
            "still": 9,
            "rich-boards": 30,
        }[suite]
    )
    assert {call["test"] for call in calls} == set(tests)
    (path / "manifest.json").write_text(
        json.dumps(
            {
                "pin": references.SOURCE_COMMIT,
                "tests": tests,
                "suite": suite,
                "passed": passed,
                "calls_sha256": references.digest(path / "calls.jsonl"),
                "events_sha256": references.digest(path / "go-events.jsonl"),
                "source_hashes": {
                    str(
                        source.relative_to(references.TRACE_CHECKOUT)
                    ): references.digest(source)
                    for source in [
                        package / "page.go",
                        package / "render.go",
                        package / "page_test.go",
                        package / "svg_test.go",
                        package / "svg_surface_resources_test.go",
                        *(
                            [
                                package / name
                                for name in [
                                    "rich_labels.go",
                                    "rich_labels_test.go",
                                    "surface_fidelity_test.go",
                                ]
                            ]
                            if suite == "rich-boards"
                            else []
                        ),
                        *(
                            [
                                package / "render_test.go",
                                package / "svg_visibility_scale_test.go",
                            ]
                            if suite == "still"
                            else []
                        ),
                        *(
                            [package / "animation_page_regression_test.go"]
                            if suite == "links"
                            else []
                        ),
                        *(
                            [package / "source_panels_test.go"]
                            if suite == "composition"
                            else []
                        ),
                        *(
                            [
                                package / "surface_document.go",
                                package / "surface_fonts.go",
                                package / "source_panels.go",
                            ]
                            if suite == "panels"
                            else []
                        ),
                        *(
                            [package / "render_test.go", package / "framing_test.go"]
                            if suite == "png"
                            else []
                        ),
                        *(
                            [
                                package / name
                                for name in [
                                    "gradient_paints_test.go",
                                    "vector_surface_svg_precision_test.go",
                                    "surface_icons_test.go",
                                ]
                            ]
                            if suite == "paint"
                            else []
                        ),
                    ]
                },
                "witness_hook_sha256": references.digest(
                    Path(__file__).with_name(witness)
                )
                if witness
                else None,
                "witness_hook": witness,
                "hook_sha256": references.digest(
                    Path(__file__).with_name("api_hook_test.go")
                ),
                "budget": {
                    "total": 1 << 30,
                    "file": 128 << 20,
                    "minimum_free": 20 << 30,
                },
            },
            indent=1,
        )
        + "\n"
    )


def compare_calls(calls, got, code):
    if not calls or len(got) != len(calls):
        raise ValueError("native output inventory differs or empty source capture")
    rows = []
    for i, (call, result) in enumerate(zip(calls, got)):
        if not valid_result(
            call["request"]["api"], call["want"], call["request"].get("options")
        ):
            outcome = "go-invalid-output"
        elif not valid_result(
            call["request"]["api"], result, call["request"].get("options")
        ):
            outcome = "rust-invalid-output"
        else:
            outcome = (
                "match"
                if code == 0 and iso.equal_call_value(call["want"], result)
                else "mismatch"
            )
        rows.append(
            {
                "call": i,
                "test": call.get("test"),
                "api": call["request"]["api"],
                "outcome": outcome,
            }
        )
    return rows


def valid_result(api, result, options=None):
    """Require complete nonempty PNGs and page inventories before any match.

    Every full encoded byte is also compared. SVG must parse as a complete,
    nonempty document with the SVG root namespace and positive dimensions.
    """
    from PIL import Image

    if not isinstance(result, dict) or set(result) != {"result", "error"}:
        return False
    if result["error"]:
        return isinstance(result["error"], str) and result["result"] is None
    try:
        page = result["result"]
        encoded = page["PNG"] if api == "page" else page
        data = base64.b64decode(encoded, validate=True)
        if not data or len(data) > (
            64 << 20
            if api == "render" and options and options.get("Format") == "svg"
            else 32 << 20
        ):
            return False
        if api == "render" and options and options.get("Format") == "svg":
            from xml.etree import ElementTree

            root = ElementTree.fromstring(data)
            import math

            return (
                math.isfinite(float(root.attrib["width"]))
                and math.isfinite(float(root.attrib["height"]))
                and root.tag == "{http://www.w3.org/2000/svg}svg"
                and float(root.attrib["width"]) > 0
                and float(root.attrib["height"]) > 0
                and len(root) > 0
            )
        with Image.open(io.BytesIO(data)) as image:
            if image.format != "PNG":
                return False
            image.verify()
        with Image.open(io.BytesIO(data)) as image:
            image.load()
            if api == "page" and (
                set(page) != {"PNG", "Links", "Width", "Height"}
                or image.size != (page["Width"], page["Height"])
                or not isinstance(page["Links"], list)
            ):
                return False
        return True
    except (KeyError, ValueError, TypeError, OSError, SyntaxError):
        return False


def api_guard(capture, outputs):
    failure = iso.direct_guard(capture, 1 << 30)
    sizes = [p.stat().st_size for p in outputs if p.exists()]
    if sizes and max(sizes) > 128 << 20:
        return "native output exceeds file budget"
    if sum(sizes) + iso.size(capture) > 1 << 30:
        return "combined API capture exceeds total budget"
    return failure


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", choices=["api"], default="api")
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument(
        "--suite",
        choices=[
            "page",
            "svg",
            "png",
            "paint",
            "panels",
            "composition",
            "fonts",
            "links",
            "still",
            "rich-boards",
        ],
        default="page",
    )
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    path = args.capture.resolve()
    if not args.reuse:
        capture(path, args.suite)
    manifest = json.loads((path / "manifest.json").read_text())
    assert manifest["pin"] == references.SOURCE_COMMIT
    for key, name in [
        ("calls_sha256", "calls.jsonl"),
        ("events_sha256", "go-events.jsonl"),
    ]:
        assert manifest[key] == references.digest(path / name)
    for source, digest in manifest.get("source_hashes", {}).items():
        assert digest == references.digest(references.TRACE_CHECKOUT / source)
    assert manifest["hook_sha256"] == references.digest(
        Path(__file__).with_name("api_hook_test.go")
    )
    if manifest.get("witness_hook_sha256"):
        assert manifest["witness_hook_sha256"] == references.digest(
            Path(__file__).with_name(
                manifest.get("witness_hook") or "svg_context_test.go"
            )
        )
    calls = [
        json.loads(line) for line in (path / "calls.jsonl").read_text().splitlines()
    ]
    assert calls, "empty API inventory"
    replay = args.output.resolve().with_suffix(".replay.jsonl")
    requests = args.output.resolve().with_suffix(".requests.jsonl")
    assert not replay.exists() and not requests.exists(), "fresh output paths required"
    requests.write_text("".join(json.dumps(call["request"]) + "\n" for call in calls))
    stderr_path = args.output.resolve().with_suffix(".stderr")
    with (
        requests.open() as stdin,
        replay.open("w") as stdout,
        stderr_path.open("w") as stderr_file,
    ):
        import subprocess

        process = subprocess.Popen(
            [str(args.rust.resolve())],
            stdin=stdin,
            stdout=stdout,
            stderr=stderr_file,
            start_new_session=True,
        )
        import signal
        import time

        started = time.monotonic()
        failure = None
        try:
            while process.poll() is None:
                failure = api_guard(path, [requests, replay, stderr_path])
                if failure or time.monotonic() - started > 600:
                    raise RuntimeError(failure or "native timeout")
                time.sleep(0.05)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
        failure = api_guard(path, [requests, replay, stderr_path])
        if failure:
            raise RuntimeError(failure)
        stderr = stderr_path.read_text()
    got = [json.loads(line) for line in replay.read_text().splitlines()]
    rows = compare_calls(calls, got, code)
    report = {
        "pin": references.SOURCE_COMMIT,
        "capture": str(path),
        "suite": manifest.get("suite", "page"),
        "rust": str(args.rust.resolve()),
        "rust_sha256": references.digest(args.rust.resolve()),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
        "returncode": code,
        "failure": None,
        "budgets": manifest["budget"],
        "stderr": stderr,
        "replay_sha256": references.digest(replay),
    }
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(report["summary"]))


if __name__ == "__main__":
    main()
