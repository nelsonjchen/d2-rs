"""Input generator for recipe mathtext-core-go-float (tools/fixtures/mathtext-core.toml).

Writes $D2RS_RECIPE_OUTPUT, the JSON string list parsed by
tools/fixtures/programs/mathtext-core-go-float: the hand-written syntax
controls (signs, NaN/Inf spellings, whitespace, underscores, hex floats,
subnormal/overflow and rounding boundaries) of the v0.9.0 opacity-float
capture, then 600 random hexadecimal and 400 random decimal floats of the same
shape (long mantissas, boundary exponents). The random section is drawn from a
fixed-seed PRNG, so its concrete values differ from the v0.9.0 capture.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

CONTROLS = ["", "0", "-0", "+0", "NaN", "nan", "NAN", "+NaN", "-NaN", "Inf", "iNf", "Infinity", "-INFINITY", "+Inf", "infi",
            "NaNstuff", " 0.5", "0.5 ", "0.5\t", "1__0", "_1", "1_", "1._0", "1_.0", "1e_2", "1e+_2", "1e2_", "0x1", "0x",
            "0x_1p0", "0x__1p0", "0x1_p0", "0x1p_0", "0x1.0p0", "0x1.p0", "0x.8p0", "0x1e2", "0x1p1_0", "0x1p-1074",
            "0x1p-1075", "0x1.00000000000000001p-1075", "0x1.fffffffffffffp1023", "0x1.fffffffffffff8p1023",
            "0x1.fffffffffffff7ffffffffp1023", "0x0p999999999999999999999999999", "0x1p-9999999999999999999999999",
            "1e309", "1e-4000", "0e99999999999999999999", "1.00000000000000011102230246251565404236316680908203125",
            "0x1.00000000000008p0", "0x1.0000000000000800000001p0", "0x1.00000000000007ffffffffp0", "0.5_0", "1_0e-1"]
HEX_EXPONENTS = [1023, 1100, -1075, -1074, -1022, 0, -1, 1, -1023, 1024, -1100]


def main() -> None:
    rng = random.Random(20261004)
    values = list(CONTROLS)

    def digits(alphabet: str, maximum: int) -> str:
        return "".join(rng.choice(alphabet) for _ in range(rng.randint(0, maximum)))
    for _ in range(600):
        whole, fraction = digits("0123456789abcdef", 96), digits("0123456789abcdef", 98)
        if not whole and not fraction:
            whole = rng.choice("0123456789abcdef")
        exponent = rng.choice(HEX_EXPONENTS) if rng.random() < 0.9 else rng.randint(-1143, 1143)
        values.append(f"{rng.choice(['', '+', '-'])}0x{whole}.{fraction}p{exponent}")
    for _ in range(400):
        whole, fraction = digits("0123456789", 90), digits("0123456789", 90)
        if not whole and not fraction:
            whole = rng.choice("0123456789")
        values.append(f"{rng.choice(['', '+', '-'])}{whole}.{fraction}e{rng.randint(-450, 450)}")
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(values) + "\n")


if __name__ == "__main__":
    main()
