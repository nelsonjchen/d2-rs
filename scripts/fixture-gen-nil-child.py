"""Input generator for recipe mathtext-latex-source-nil-child (tools/fixtures/mathtext-latex-wasm.toml).

Writes $D2RS_RECIPE_OUTPUT: the D2 sources of the malformed-math control
sweep (a vdotswithin/shortvdotswithin alignment mark inside each alignment
environment, with a well-formed, undefined or truncated argument, an
optional trailing undefined command and an optional outer style) plus four
error-precedence probes. Each case is one `x: |latex ...|` shape label.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ENVIRONMENTS = [("align*", ""), ("aligned", ""), ("xalignat", "{2}"), ("xxalignat", "{2}")]
COMMANDS = ["vdotswithin", "shortvdotswithin"]
ARGUMENTS = ["=", "\\unknown", "\\frac{a}"]
STYLES = ["", "\\scriptstyle ", "\\small "]
PRECEDENCE = [
    "\\begin{aligned}\\vdotswithin{\\unknown}\\end{aligned}\\unknown",
    "\\unknown\\begin{aligned}\\vdotswithin{\\unknown}\\end{aligned}",
    "\\begin{aligned}\\vdotswithin{\\unknown}\\end{aligned} + \\frac{a}",
    "\\frac{\\begin{aligned}\\vdotswithin{\\unknown}\\end{aligned}}{x}",
]


def source(expression: str) -> str:
    return f"x: |latex\n{expression}\n|\n"


def main() -> None:
    cases = []
    for environment, environment_argument in ENVIRONMENTS:
        for command in COMMANDS:
            for argument in ARGUMENTS:
                for outer_error in (False, True):
                    for style in STYLES:
                        body = f"\\{command}{{{argument}}}" + (" \\unknown" if outer_error else "")
                        expression = f"{style}\\begin{{{environment}}}{environment_argument}{body}\\end{{{environment}}}"
                        cases.append({
                            "name": f"math-control-{len(cases)}",
                            "source": source(expression),
                            "expression": expression,
                            "environment": environment,
                            "command": command,
                            "argument": argument,
                            "outer_error": outer_error,
                            "style": style,
                        })
    for index, expression in enumerate(PRECEDENCE):
        cases.append({"name": f"precedence-{index}", "source": source(expression), "expression": expression})
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(cases, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
