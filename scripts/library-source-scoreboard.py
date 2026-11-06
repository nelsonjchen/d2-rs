#!/usr/bin/env python3
"""Capture the six remaining original master d2lib source-boundary tests.

Reuse the library scoreboard's complete inventory, byte budgets, timeouts,
original-assertion verification and exact JSON comparison.
"""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "library_api", Path(__file__).with_name("library-api-scoreboard.py")
)
library = importlib.util.module_from_spec(spec)
spec.loader.exec_module(library)
library.TEST_FILES = ["d2lib/d2_test.go"]
library.TESTS = {
    "TestParseAndCompileHonorCanceledContext": 2,
    "TestCompilePropagatesVariableExpansionLimit": 1,
    "TestCompilePropagatesGlobExpansionLimit": 1,
    "TestCompilePropagatesEdgeExpansionLimit": 1,
    "TestCompilePropagatesEdgeExpansionWorkLimit": 1,
    "TestNilFSDeniesImports": 1,
}


def instrument(text):
    # Restrict replacements to these six functions; preserve unrelated tests.
    for test in library.TESTS:
        start = text.index("func " + test + "(")
        end = text.find("\nfunc ", start + 1)
        if end < 0:
            end = len(text)
        body = text[start:end]
        body = body.replace("Parse(ctx,", "d2rsSourceParse(t, ctx,")
        body = body.replace("Compile(", "d2rsSourceCompile(t, ")
        # Diagnostic strings are assertions too, and stay byte-identical.
        body = body.replace('"d2rsSourceCompile(t, ', '"Compile(')
        text = text[:start] + body + text[end:]
    return text


library.instrument = instrument
library.HOOK = "tools/library-source-scoreboard/hook_test.go"

if __name__ == "__main__":
    library.main()
