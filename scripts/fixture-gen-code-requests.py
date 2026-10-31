"""Request generators for the d2scenebuild code-block hooks (tools/fixtures/mathtext-txt.toml).

    fixture-gen-code-requests.py glyph|token-budget

writes $D2RS_RECIPE_OUTPUT, the JSON request list read by
tools/fixtures/hooks/mathtext-txt-code-scenebuild/d2rs_code_*_test.go:

- glyph: Source Code Pro regular/bold/italic x six sizes x 17 texts (kerning,
  ligature, emoji, control, joiner and CJK samples), then three texts under a
  cancelled and an expired context.
- token-budget: maximum scene nodes x token value sequences x cancellation
  modes, the last sequence repeated past the node limit.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

TEXTS = ["", "AV", "a α", "fi", "x\U0001f642", "café", "\t", "\x00", "\n", "a\r", "a b", "漢字", "a‍",
         " ", "\U0001f4be", "−", "á"]


def glyph() -> list:
    rows = [{"font": font, "size": size, "text": text, "cancel": "none"}
            for font in ["regular", "bold", "italic"] for size in [1, 16, 23, 32, 16.15, 0.03125] for text in TEXTS]
    rows += [{"font": "regular", "size": 16, "text": text, "cancel": cancel}
             for cancel in ["before", "deadline_before"] for text in ["", "AV", "\U0001f642"]]
    return rows


def token_budget() -> list:
    def unit(text, repeat=1):
        return {"unit": text, "repeat": repeat}
    cases = [([], 1), ([unit("")], 1), ([unit("a")], 1), ([unit("\n")], 1), ([unit("a\nb")], 1), ([unit("\n", 2)], 1),
             ([unit("a"), unit("b")], 1), ([unit("a\n"), unit("")], 1), ([unit("α\n", 10)], 1),
             ([unit("\n", 100000)], 1), ([unit("a")], 100001)]
    return [{"name": f"{maximum}-{i}-{cancel}", "max_nodes": maximum, "values": values, "sequence_repeat": repeat,
             "cancel": cancel}
            for maximum in [0, 1, 2, 3, 5, 100001] for i, (values, repeat) in enumerate(cases)
            for cancel in ["none", "before", "deadline_before", "after_first"]]


MODES = {"glyph": glyph, "token-budget": token_budget}


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in MODES:
        sys.exit(f"usage: {sys.argv[0]} {'|'.join(MODES)}")
    rows = MODES[sys.argv[1]]()
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
