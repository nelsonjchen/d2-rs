#!/usr/bin/env python3
"""Observe original direct SVG isometric tests, then replay the native document API.

Complete SVG bytes, ordered board arrays, and complete errors compare.
All original assertions remain active. Inf inputs retain their IEEE bits.
Shared capture bounds: 1 GiB total / 128 MiB file / 20 GiB free / 300 seconds.
"""
import base64
import importlib.util
import json
import xml.etree.ElementTree as ET
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "svg_admission", Path(__file__).with_name("svg-admission-scoreboard.py")
)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.TEST_FILES = ["d2renderers/d2svg/isometric_test.go"]
base.TESTS = [
    "TestRenderIsometricModeAndDocumentOptions",
    "TestRenderIsometricRejectsIncompatibleModes",
    "TestRenderIsometricMultiboardIsStaticSVG",
    "TestRenderIsometricRequiresResolverForExternalImages",
]
base.COUNTS = dict(zip(base.TESTS, [3, 8, 1, 1]))
base.HOOK = base.ROOT / "tools/svg-isometric-scoreboard/hook_test.go"
base.transform_tests = lambda text: text.replace(
    "Render(d,", "d2rsRender(t, d,"
).replace("Render(diagram,", "d2rsRender(t, diagram,").replace(
    "RenderMultiboard(d,", "d2rsRenderMultiboard(t, d,"
).replace("RenderIsometric(context.", "d2rsRenderIsometric(t, context.")
original_source = base.source


def source():
    result = original_source()
    pinned = base.references.verify_source()
    result["implementation"]["isometric.go"] = base.sha(pinned / "d2renderers/d2svg/isometric.go")
    return result


base.source = source
original_verify = base.verify


def validate_response(request, response):
    assert set(response) == {"output", "error"}
    assert isinstance(response["error"], str)
    if response["error"]:
        assert response["output"] is None
        return
    output = response["output"]
    outputs = output if request["api"] == "RenderMultiboard" else [output]
    assert isinstance(outputs, list) and outputs
    for encoded in outputs:
        assert isinstance(encoded, str)
        decoded = base64.b64decode(encoded, validate=True)
        assert decoded and ET.fromstring(decoded).tag == "{http://www.w3.org/2000/svg}svg"


def verify(capture):
    cases = original_verify(capture)
    for case in cases:
        assert case["test"] == case["name"].rsplit("#", 1)[0]
        assert case["request"]["api"] in {"Render", "RenderIsometric", "RenderMultiboard"}
        validate_response(case["request"], case["got"])
    return cases


base.verify = verify
original_score = base.score


def score(capture, runner, output):
    original_score(capture, runner, output)
    report = json.loads(output.read_text())
    report["package"] = "d2renderers/d2svg"
    output.write_text(json.dumps(report, indent=2) + "\n")


base.score = score

if __name__ == "__main__":
    # Reuse the standard capture/score CLI without its suite-specific selection.
    base.main()
