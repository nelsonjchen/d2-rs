#!/usr/bin/env python3
"""Isometric capture inventory, complete responses and original assertion gates."""
import importlib.util
import unittest
from pathlib import Path


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


original = module("test_wasm_admission_scoreboard")
isometric = module("wasm-isometric-scoreboard")
original.scoreboard = isometric.base


class IsometricCaptureGate(original.CaptureGate):
    expected_count = 12

    def test_compile_projection_is_complete(self):
        for value in [{}, {"diagram": {}}, {"diagram": {}, "renderOptions": None}]:
            with self.subTest(value=value), self.assertRaises(AssertionError):
                isometric.validate_response({"data": value})
        isometric.validate_response({"data": {"diagram": {}, "renderOptions": {}}})


if __name__ == "__main__":
    unittest.main()
