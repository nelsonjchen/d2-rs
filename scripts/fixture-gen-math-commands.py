"""Input generator for recipe mathtext-latex-commands (tools/fixtures/mathtext-latex.toml).

Writes $D2RS_RECIPE_OUTPUT: one {"expression": "\\<name>", "command": "<name>"}
row per static command-name candidate of the mathjax-go TeX parser required by
the patched upstream go.mod, using the candidate rule of
scripts/generate-math-command-registry.py (quoted alphabetic names and
`Name: "\\name"` fields in the non-test files of internal/tex). The d2latex
hook then renders each expression; the Rust test classifies a name as
recognized unless its only diagnostic is "Undefined control sequence".
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402
import mathtext_gen  # noqa: E402

MODULE = "github.com/d2lang/mathjax-go"


def main() -> None:
    upstream = Path(fixture_recipe._env("D2RS_UPSTREAM"))
    tex = mathtext_gen.module_dir(MODULE, mathtext_gen.required_version(upstream / "go.mod", MODULE)) / "internal/tex"
    spec = importlib.util.spec_from_file_location(
        "registry", Path(__file__).resolve().parent / "generate-math-command-registry.py")
    registry = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(registry)
    rows = [{"expression": "\\" + name, "command": name} for name in registry.candidates(tex)]
    Path(os.environ["D2RS_RECIPE_OUTPUT"]).write_text(json.dumps(rows, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
