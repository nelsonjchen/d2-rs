#!/usr/bin/env python3
"""Original registered-font rendering and direct ThemeCSS admission contracts.

Complete documents/stylesheets and errors compare, with input-only font
registration operations. Capture limits: 1 GiB / 128 MiB per file / 20 GiB
free, streaming checks and 300-second deadline from svg-admission-scoreboard.
"""
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("svg_admission", Path(__file__).with_name("svg-admission-scoreboard.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.TEST_FILES = ["d2renderers/d2svg/" + name for name in ["render_target_validation_test.go", "paint_validation_test.go"]]
base.TESTS = ["TestRenderAcceptsRegisteredCustomFontFamily", "TestThemeCSSRejectsUnsupportedOverrides"]
base.COUNTS = dict(zip(base.TESTS, [1, 5]))
base.HOOK = base.ROOT / "tools/svg-font-scoreboard/hook_test.go"

def transform(text):
    text = text.replace("d2svg.Render(", "d2rsRender(t, ")
    text = text.replace("d2svg.ThemeCSS(", "d2rsThemeCSS(t, ")
    text = text.replace("d2fonts.AddFontFamily(", "d2rsAddFontFamily(t, ")
    return text

base.transform_tests = transform
original_source = base.source

def source():
    result = original_source()
    pinned = base.references.verify_source()
    result["font_registry"] = base.sha(pinned / "d2renderers/d2fonts/d2fonts_common.go")
    result["target_validation"] = base.sha(pinned / "d2target/render_validation.go")
    return result

base.source = source
original_verify = base.verify

def validate_case(case):
    request, response = case["request"], case["got"]
    assert case["test"] == case["name"].rsplit("#", 1)[0]
    assert set(response) == {"output", "error"} and isinstance(response["error"], str)
    if request["api"] == "ThemeCSS":
        assert isinstance(response["output"], str)
        if response["error"]:
            assert response["output"] == ""
    else:
        assert request["api"] == "Render"
        validator = base.module("svg-isometric-scoreboard")
        validator.validate_response({"api": "Render"}, response)
        assert isinstance(request["families"], list) and request["families"]
        for family in request["families"]:
            assert set(family) == {"name", "regular", "italic", "bold", "semibold"}
            assert isinstance(family["name"], str)

def verify(capture):
    cases = original_verify(capture)
    for case in cases:
        validate_case(case)
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
    base.main()
