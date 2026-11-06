#!/usr/bin/env python3
"""Observe both original WASM isometric tests, preserving every Go assertion.

Compile compares the complete diagram and resolved render options consumed by
the original test. Render compares the complete data/error envelope. Graph/FS
projection and the Rust WASM transport are outside this native API observation.
The shared capture runner enforces 1 GiB total / 128 MiB file / 20 GiB free
streaming limits and finite Go/native deadlines.
"""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "wasm_admission_scoreboard", Path(__file__).with_name("wasm-admission-scoreboard.py")
)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.TEST_FILES = ["d2js/d2wasm/isometric_test.go"]
base.TESTS = [
    "TestIsometricWASMCompileAndRender",
    "TestIsometricWASMRejectsIncompatibleFormats",
]
base.COUNTS = dict(zip(base.TESTS, [6, 6]))
base.HOOK = base.ROOT / "tools/wasm-isometric-scoreboard/hook_test.go"
base.GO_TEST_FLAGS = ["-timeout=60s", "-test.testlogfile="]
base.GO_TIMEOUT = 120
original_source = base.source


def source():
    return {**original_source(), "go_test_flags": base.GO_TEST_FLAGS}


base.source = source
base.transform_tests = lambda text: text.replace(
    "Compile(wasmJSONArgument", "d2rsCompile(t, wasmJSONArgument"
).replace("Render(wasmJSONArgument", "d2rsRender(t, wasmJSONArgument")
validate_render_response = base.validate_response


def validate_response(response):
    if isinstance(response, dict) and isinstance(response.get("data"), dict):
        assert set(response) == {"data"}
        assert set(response["data"]) == {"diagram", "renderOptions"}
        assert isinstance(response["data"]["diagram"], dict)
        assert isinstance(response["data"]["renderOptions"], dict)
    else:
        validate_render_response(response)


base.validate_response = validate_response

if __name__ == "__main__":
    base.main()
