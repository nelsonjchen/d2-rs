"""Input generator for recipe mathtext-txt-uniseg (tools/fixtures/mathtext-txt.toml).

Writes $D2RS_RECIPE_OUTPUT, a JSON list of strings for tools/mathtext-uniseg-oracle,
taken from the github.com/rivo/uniseg module pinned by that oracle's go.mod:

1. every Unicode LineBreakTest and GraphemeBreakTest case embedded in
   uniseg's linebreak_test.go and graphemebreak_test.go;
2. "a" + c + "b" for every first and last code point of a range in uniseg's
   line-break, grapheme and East Asian width property tables (surrogates
   excluded), in code point order;
3. 2050 deterministic pseudo-random strings (lengths 0-14) over those code points;
4. a few hand-written mixed-script strings.
"""
from __future__ import annotations

import random
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mathtext_gen  # noqa: E402

MODULE = "github.com/rivo/uniseg"
ORACLE_GO_MOD = mathtext_gen.ROOT / "tools/mathtext-uniseg-oracle/go.mod"
RANDOM_SEED = 20261008
RANDOM_STRINGS = 2050
HANDWRITTEN = ["a­b", "\U0001f469‍\U0001f469‍\U0001f466", "\r\n", "你好，世界",
               "left right"]


def go_string(literal: str) -> str:
    """Decode a Go interpreted string literal body made of \\uXXXX/\\UXXXXXXXX escapes and ASCII."""
    def unescape(m: re.Match) -> str:
        return chr(int(m.group(1) or m.group(2), 16))
    if re.search(r"\\[^uU]", literal):
        raise SystemExit(f"unexpected escape in {literal!r}")
    return re.sub(r"\\u([0-9a-fA-F]{4})|\\U([0-9a-fA-F]{8})", unescape, literal)


def test_cases(source: Path, var: str) -> list[str]:
    text = source.read_text()
    body = text[text.index(f"var {var} = "):]
    body = body[: body.index("\n}\n")]
    return [go_string(m) for m in re.findall(r'original: "((?:[^"\\]|\\.)*)"', body)]


def range_endpoints(module: Path) -> list[int]:
    points: set[int] = set()
    for name in ("lineproperties.go", "graphemeproperties.go", "eastasianwidth.go"):
        for lo, hi in re.findall(r"\{0x([0-9A-Fa-f]+), 0x([0-9A-Fa-f]+),", (module / name).read_text()):
            points.update((int(lo, 16), int(hi, 16)))
    return sorted(p for p in points if not 0xD800 <= p <= 0xDFFF)


def main() -> None:
    module = mathtext_gen.module_dir(MODULE, mathtext_gen.required_version(ORACLE_GO_MOD, MODULE))
    inputs = test_cases(module / "linebreak_test.go", "lineBreakTestCases")
    inputs += test_cases(module / "graphemebreak_test.go", "graphemeBreakTestCases")
    points = range_endpoints(module)
    inputs += [f"a{chr(p)}b" for p in points]
    rng = random.Random(RANDOM_SEED)
    inputs.append("")
    for _ in range(RANDOM_STRINGS - 1):
        inputs.append("".join(chr(rng.choice(points)) for _ in range(rng.randint(1, 14))))
    inputs += HANDWRITTEN
    mathtext_gen.write_output(inputs)


if __name__ == "__main__":
    main()
