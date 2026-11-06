"""Input generator for the mathtext-txt-chroma-* token recipes (tools/fixtures/mathtext-txt.toml).

    fixture-gen-chroma.py LOGICAL_OUTPUT

Writes $D2RS_RECIPE_OUTPUT, a list of [language, source] pairs for
tools/mathtext-chroma-oracle, from the entry for LOGICAL_OUTPUT in
tools/fixtures/inputs/mathtext-txt/chroma-tokens.json: the hand-written head
cases, then for each random spec `count` deterministic sources concatenating
1..max_pieces lexer-relevant pieces (keywords, literals, comment and string
delimiters, combining and joiner characters, line endings), then the tail
cases (aliases, fenced blocks).
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mathtext_gen  # noqa: E402

SEEDS = mathtext_gen.ROOT / "tools/fixtures/inputs/mathtext-txt/chroma-tokens.json"
RANDOM_SEED = 20261013


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: fixture-gen-chroma.py LOGICAL_OUTPUT")
    spec = json.loads(SEEDS.read_text())[sys.argv[1]]
    rng = random.Random(f"{RANDOM_SEED}:{sys.argv[1]}")
    cases = list(spec["head"])
    for part in spec["random"]:
        for _ in range(part["count"]):
            n = rng.randint(1, part["max_pieces"])
            cases.append([part["language"], "".join(rng.choice(part["pieces"]) for _ in range(n))])
    cases += spec["tail"]
    mathtext_gen.write_output(cases)


if __name__ == "__main__":
    main()
