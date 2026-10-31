"""Request generators for the request-driven d2svgimport/d2scenebuild hooks
(tools/fixtures/mathtext-svg.toml).

    fixture-gen-svg-requests.py color|length|transform|xml-budget|document-budget|gif-config

writes $D2RS_RECIPE_OUTPUT, the JSON request list read through
$D2RS_RECIPE_INPUT by the matching tools/fixtures/hooks/*/d2rs_<mode>_test.go.
The sweeps are those of the v0.9.0 captures' prepare scripts; their seeded
random sections use Python's `random` with the original seeds, so the
requests are identical to the v0.9.0 ones.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def color() -> list:
    values = list(json.loads((ROOT / "crates/d2-rs-math/src/css_named_colors.json").read_text()))
    values += ["transparent", "TRANSPARENT", " Red ", "#abc", "#abcd", "#123456", "#12345678", "abc", "bad", "none",
               "currentColor", "url(#x)", "#ggg", "rgb(1,2,3)", "rgb(1 2 3 / 50%)", "rgb(100% 0% 0%)", "rgb(1\t2\t3)",
               "rgb(1,,2,,,3)", "rgb(NaN 0 0)", "rgb(inf 0 0)", "rgb(0xff 0 0)", "rgb(0x1p4 0 0)", "rgb(1_0 0 0)",
               "rgba(1 2 3 / bad)", "", "x" * 4097]
    for name in ["rgb", "rgba", "hsl", "hsla", "hwb", "hwba", "hsv", "hsva", "lab", "lch", "oklab", "oklch"]:
        for fields in ["0 0 0", "1 1 1", "50% 50% 50%", "30 50% 50%", "180deg 1 0.5", "NaN 0 0", "1e308 1e308 1e308",
                       "-100 -100 -100", "bad 1 1", "1 2", "1 2 3 4 5"]:
            for alpha in ["", " / .5", " / 50%"]:
                values.append(f"{name}({fields}{alpha})")
    rng = random.Random(724)
    for _ in range(250):
        name = rng.choice(["rgb", "hsl", "hwb", "hsv", "lab", "lch", "oklab", "oklch"])
        fields = [str(rng.randrange(-100, 400) / 10) for _ in range(3)]
        values.append(name + "(" + " ".join(fields) + " / " + str(rng.randrange(0, 100) / 100) + ")")
    # Non-finite channels and alpha, then huge lightness for the Lab-family spaces.
    for name in ["rgb", "hsl", "hwb", "hsv", "lab", "lch", "oklab", "oklch"]:
        for fields in ["NaN 1 30", "1 NaN 30", "1 30 NaN", "inf 1 30", "1 inf 30", "1 30 inf", "-inf -inf -inf"]:
            values.append(f"{name}({fields} / NaN)")
    for name in ["oklab", "oklch", "lab", "lch"]:
        for lightness in ["1e10", "1e20", "1e100", "-1e10", "-1e20", "-1e100"]:
            values.append(f"{name}({lightness} 1 30)")
    return values


def length() -> list:
    values = ["", " ", "0", "-0", "+0", "1", "-1", ".5", "1.", "1e3", "1e-999", "1e999", "1e", "1e+", "1_000", "0x1p0", "NaN",
              "Inf", "1%", "1 2", "1 ", "​1", "﻿1", "\u00851 ", "1" * 512, "1" * 513, " " * 513 + "1",
              "\U0001f642" * 129]
    for unit in ["px", "in", "cm", "mm", "pt", "pc", "q", "ex", "em", "PX", "CM", "Q", "EX", "%"]:
        for n in ["0", "-0", "1", "-1", ".1", "1e308", "1e-308"]:
            values += [n + unit, n + " " + unit, " " + n + unit + " "]
    rows = []
    for value in values:
        for kind in ["number", "ex", "interval"]:
            rows.append({"value": value, "kind": kind, "allow_units": False})
        for allow in [False, True]:
            rows.append({"value": value, "kind": "length", "allow_units": allow})
    return rows


def transform() -> list:
    values = ["", " ", "translate(1)", "scale(2 -3)", "matrix(1 2 3 4 5 6)", "rotate(30 1 2)", "skewX(45)", "skewY(-45)",
              "rotate(-0)", "scale(-0)", "translate(0 -0)", "translate(1)scale(2)", "translate(1),", "translate(1),, scale(2)",
              "translate(,1)", "translate(1,)", "translate(1,,2)", "translate(1-2)", "translate(1.2.3)", "translate()",
              "translate(1 2 3)", "matrix(1 2 3 4 5 6 7)", "unknown()", "Translate(1)", "rotate(1 2)", "skewX(1 2)",
              "scale(1e999)", "scale(1e-999)", "scale(1e)", "scale(.)", "scale(1", "scale", "2", "scale(1) translate(1)",
              "scale(1e308) scale(1e308)", "matrix(1 2 3 4 5 6) rotate(30) translate(7 -8)"]
    for name in ["rotate", "skewX", "skewY"]:
        for angle in ["0", "-0", "1", "-1", "30", "45", "90", "180", "270", "360", "540", "-540", "1e308", "-1e308", "1e-308",
                      "89.999999", "90.000001"]:
            values.append(f"{name}({angle})")
    rng = random.Random(716)
    for _ in range(150):
        parts = []
        for _ in range(rng.randrange(1, 8)):
            name = rng.choice(["scale", "translate", "rotate", "skewX", "skewY"])
            n = rng.choice([1, 2]) if name in ["scale", "translate"] else rng.choice([1, 3]) if name == "rotate" else 1
            parts.append(name + "(" + ",".join(str(rng.randrange(-1000, 1000) / 10) for _ in range(n)) + ")")
        values.append(" ".join(parts))
    rows = []
    for value in values:
        for maximum in [1, 2, 1000]:
            rows.append({"value": value, "max_bytes": 10000, "max_functions": maximum})
    for value in values[:36]:
        rows.append({"value": value, "max_bytes": max(1, len(value.encode()) - 1), "max_functions": 1000})
    rows += [{"value": "scale(1)", "max_bytes": 0, "max_functions": 1}, {"value": "scale(1)", "max_bytes": 1, "max_functions": 0}]
    return rows


def xml_budget() -> list:
    samples = ['<svg/>', '<svg><g><path d="M0 0L1 1"/></g></svg>', '<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"/>',
               '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><use xlink:href="#a"/><path id="a" d="M0 0"/></svg>',
               '<svg data-a="µ&amp;\U0001f642" role="img"><g opacity="1"/></svg>', '<svg><title>ignored</title><g data-a="a&#10;b&#9;c"/></svg>',
               '﻿<svg/>', '<svg data-a="a\r\nb\rc\td"/>',
               '<svg xmlns="http://www.w3.org/2000/svg" xmlns:p="http://www.w3.org/1999/xlink"><use p:href="#a"/><g id="a"/></svg>']
    rows = []
    for xml in samples:
        rows.append({"xml": xml, "limits": [10000] * 9})
        for slot in range(5):
            for maximum in [1, 2, 5, 10, 50, len(xml.encode())]:
                limits = [10000] * 9
                limits[slot] = maximum
                rows.append({"xml": xml, "limits": limits})
    for slot in range(9):
        limits = [10000] * 9
        limits[slot] = 0
        rows.append({"xml": "<svg/>", "limits": limits})
    return rows


def document_budget() -> list:
    rows = []

    def add(m=None, u=None, limits=None, c=None, e=0, q=0):
        rows.append({"maximum": m or [10] * 8, "used": u or [0] * 8, "limits": limits or [100] * 7, "parsed": c or [1] * 8,
                     "emitted_elements": e, "emitted_paths": q})
    add()
    add(u=[9] * 8)
    add(u=[10] * 8)
    add(limits=[3] * 7)
    for i in range(8):
        for v in [0, 1, 9, 10, 11]:
            m = [10] * 8
            m[i] = v
            add(m=m)
        for v in [9, 10, 11]:
            u = [0] * 8
            u[i] = v
            add(u=u)
        for v in [9, 10, 11]:
            c = [1] * 8
            c[i] = v
            add(c=c)
    add(e=11)
    add(q=11)
    add(e=10, q=10)
    return rows


def gif_config() -> list:
    """GIF byte inputs: header/palette skeletons, then prefixes, byte mutations and
    trailers of one complete 2x3 GIF89a with a two-entry palette."""
    inputs = []
    for signature in [b"GIF87a", b"GIF89a", b"GIF88a", b"gif89a", b'\x00\xffA\n"\\']:
        for width, height in [(2, 3), (0, 3), (2, 0), (0, 0), (0xFFFF, 0xFFFF)]:
            for packed in [0x00, 0x80, 0x81, 0x87]:
                palette = bytes(3 << ((packed & 7) + 1)) if packed & 0x80 else b""
                inputs.append(signature + width.to_bytes(2, "little") + height.to_bytes(2, "little")
                              + bytes([packed, 0xFF, 0x00]) + palette + b"\x2c" + bytes(9) + b"\x02\x00\x3b")
    base = b"GIF89a\x02\x00\x03\x00\x80\x00\x00\x00\x01\x02\x03\x04\x05\x2c" + bytes(9) + b"\x02\x00\x3b"
    inputs += [base[:n] for n in range(len(base) + 1)]
    for i in range(len(base)):
        for value in (0x00, 0xFF):
            inputs.append(base[:i] + bytes([value]) + base[i + 1:])
    inputs += [base + b"x", base[:-1] + b"\x2c" + bytes(9) + b"\x02\x00\x3b", base[:-1] + b"\x21\xf9\x02ab\x00\x3b",
               base[:13] + base[19:]]
    return [list(b) for b in inputs]


MODES = {"color": color, "length": length, "transform": transform, "xml-budget": xml_budget,
         "document-budget": document_budget, "gif-config": gif_config}


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in MODES:
        sys.exit(f"usage: {sys.argv[0]} {'|'.join(MODES)}")
    rows = MODES[sys.argv[1]]()
    Path(fixture_recipe._env("D2RS_RECIPE_OUTPUT")).write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
