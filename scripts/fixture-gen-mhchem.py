"""Input generator for the mathtext-mhchem-* recipes (tools/fixtures/mathtext-txt.toml).

    fixture-gen-mhchem.py pattern|regexp|texify|expand

Writes $D2RS_RECIPE_OUTPUT, the rows tools/mathtext-mhchem-oracle evaluates:

- pattern: every mhchem pattern name x every input of
  tools/fixtures/inputs/mathtext-txt/mhchem-inputs.json. Names are the regex
  and function keys of mathjax-go's internal/mhchem/data/patterns.json (table
  order), an unknown name, and the helper regexes compiled in patterns.go's
  buildRegexes.
- regexp: every (name, source) regex of the same tables x the "common" inputs.
- texify: texifier buffers covering every buffer type_ with scalar and array
  fields, all 64 chemfive field subsets per dType, arrows with and without
  labels, bond and operator kinds, and malformed buffers; each inner and outer.
- expand: ToTeX (mode, input) rows: the 117 {Mode, input, want} cases of
  internal/mhchem/mhchem_test.go (origin original117), 24 retained
  chemistry/unit controls, every pattern input plus extra unit/arrow/Unicode
  strings in each of ce, pu and tex (exploratory), and invalid modes.

The tables are read from the mathjax-go module pinned by
tools/mathtext-mhchem-oracle/go.mod.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mathtext_gen  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / "tools/mathtext-mhchem-oracle"
SEEDS = ROOT / "tools/fixtures/inputs/mathtext-txt/mhchem-inputs.json"


def mathjax_dir() -> Path:
    module = "github.com/d2lang/mathjax-go"
    return mathtext_gen.module_dir(module, mathtext_gen.required_version(ORACLE / "go.mod", module)) / "internal/mhchem"


def regexes(mhchem: Path) -> tuple[list[tuple[str, str]], list[tuple[str, str]], list[str]]:
    """(table regexes, buildRegexes helpers, function keys), each in source order."""
    tables = json.loads((mhchem / "data/patterns.json").read_text())
    build = (mhchem / "patterns.go").read_text().split("func buildRegexes()", 1)[1].split("\n}\n", 1)[0]
    helpers = re.findall(r"(re[A-Za-z]+) = mustCompile\(`([^`]*)`\)", build)
    if not helpers:
        sys.exit("no buildRegexes helpers found in patterns.go")
    return list(tables["regex"].items()), helpers, tables["functionKeys"]


def texify_rows() -> list[dict]:
    rows: list[dict] = []

    def add(name: str, values: list) -> None:
        for outer in (False, True):
            rows.append({"name": f"{name}-{'outer' if outer else 'inner'}", "outer": outer, "values": values})

    scalar = {"color": "blue", "color1": "red", "color2": ["a", {"p1": "B", "type_": "rm"}],
              "p1": "a b-c^d_e", "p2": "3"}
    array = {"color2": [{"p1": "1", "p2": "2", "type_": "frac"}],
             "p1": ["a", {"p1": "B", "type_": "rm"}], "p2": ["2"]}
    for kind in ["rm", "text", "roman numeral", "state of aggregation", "state of aggregation subscript",
                 "frac", "pu-frac", "tex-math", "frac-ce", "overset", "underset", "underbrace", "color",
                 "color0", "1st-level escape", "space", "tinySkip", "entitySkip", "pu-space-1",
                 "pu-space-2", "1000 separator", "commaDecimal", "comma enumeration L",
                 "comma enumeration M", "comma enumeration S", "hyphen", "addition compound",
                 "electron dot", "KV x", "prime", "cdot", "tight cdot", "times", "circa", "^", "v",
                 "ellipsis", "/", " / "]:
        add(kind, [{**scalar, "type_": kind}])
        add(f"{kind}-array", [{**array, "type_": kind}])
    fields = [("a", ["-2"]), ("b", ["13"]), ("p", ["6"]), ("o", [{"p1": "C", "type_": "rm"}]),
              ("q", ["4"]), ("d", ["2+"])]
    for dtype in ["", "kv", "oxidation"]:
        for mask in range(64):
            bits = format(mask, "06b")
            value = {"dType": dtype, "type_": "chemfive"}
            value.update({k: v for (k, v), bit in zip(fields, bits) if bit == "1"})
            add(f"chemfive-{dtype}-{bits}", [value])
    for arrow in ["->", "→", "⟶", "<-", "<->", "<-->", "<=>", "⇌", "<=>>", "<<=>", "bad"]:
        for rd in (False, True):
            for rq in (False, True):
                add(f"arrow-{arrow}{rd}{rq}", [{"r": arrow, "rd": ["A"] if rd else [],
                                                "rq": [{"p1": "B", "type_": "text"}] if rq else [],
                                                "type_": "arrow"}])
    for kind in ["-", "1", "=", "2", "#", "3", "~", "~-", "~=", "~--", "-~-", "...", "....", "->", "<-",
                 "<", ">", "bad"]:
        add(f"bond-{kind}", [{"kind_": kind, "type_": "bond"}])
    for kind in ["+", "-", "=", "<", ">", "<<", ">>", "\\pm", "\\approx", "$\\approx$", "v", "(v)", "^",
                 "(^)", "bad"]:
        add(f"operator-{kind}", [{"kind_": kind, "type_": "operator"}])
    edges = [[], ["literal"], ["a", {"p1": "b", "type_": "1st-level escape"}, "c"], [None], [False], [7],
             [{}], [{"type_": "unknown"}], [{"color2": [{}], "type_": "color"}],
             [{"a": ["+2"], "dType": "other", "o": ["-O"], "type_": "chemfive"}],
             [{"p1": "a b-c", "type_": "text"}], [{"p1": True, "p2": "wrong", "type_": "frac-ce"}]]
    for i, values in enumerate(edges):
        add(f"edge-{i}", values)
    return rows


RETAINED = [("ce", "SO4^2- + Ba^2+ -> BaSO4 v"), ("ce", "2H2 + O2 -> 2H2O"), ("ce", "H2O"), ("ce", "CO2"), ("ce", "Na+"),
            ("ce", "Cl-"), ("ce", "^{14}_{6}C"), ("ce", "Fe^{III}"), ("ce", "A <=> B"), ("ce", "A ->[heat] B"),
            ("ce", "A ->[a][b] B"), ("ce", "H-O-H"), ("ce", "C=C"), ("ce", "C#C"), ("ce", "CuSO4.5H2O"), ("ce", "H2(g)"),
            ("ce", "(NH4)2SO4"), ("ce", "\\bond{bad}"), ("pu", "123 kg"), ("pu", "1,234.5 kg"), ("pu", "1.5e3 mol"),
            ("pu", "10 m/s"), ("pu", "20 °C"), ("pu", "2.0 mol L-1")]
EXTRA_INPUTS = ["1.23e4 kg", "1.23E4 kg", "1.23 x 10^4 kg", "1.23 * 10^{4} kg", "12345678.12345678 kg", "1 kg//mol",
                "1 kg/m s", "°C", "^oC", "^{o}F", "A ->[T[text]][M[x^2]] B", "A <=>[H2O][CO2] B", "A <-[x] B",
                "A \\bond{invalid} B", "\\color{red}{H2O}", "\\color\\blue{H2O}", "\\frac{H2}{O2}", "\\underbrace{H2}_{O2}",
                "A−B", "A–B", "A—B", "A‐B", "A…B", "A\nB", "X_{$\\frac{1}{2}$}", "\\ce{H2O} + \\pu{1 kg}"]
INVALID_MODES = ["unknown", "", "ce2", "图", "\x00", "\x07", "\x7f", "\x85", "​", "﻿", "a\nb", 'a"b', "\U0010ffff"]


def expand_rows(seeds: dict) -> list[dict]:
    quoted = r'"(?:[^"\\]|\\.)*"'
    source = (mathjax_dir() / "mhchem_test.go").read_text()
    rows = [{"origin": "original117", "mode": mode.lower(), "input": json.loads(argument), "upstream_want": json.loads(want)}
            for mode, argument, want in re.findall(r"\{Mode(TeX|CE|PU),\s*(" + quoted + r"),\s*(" + quoted + r")\}", source)]
    if not rows:
        sys.exit("no {Mode, input, want} cases found in internal/mhchem/mhchem_test.go")
    rows += [{"origin": "retained24", "mode": mode, "input": text} for mode, text in RETAINED]
    inputs = list(dict.fromkeys(seeds["common"] + seeds["observe_groups"] + EXTRA_INPUTS))
    rows += [{"origin": "exploratory", "mode": mode, "input": text} for mode in ["ce", "pu", "tex"] for text in inputs]
    rows += [{"origin": "mode_error", "mode": mode, "input": "H2O"} for mode in INVALID_MODES]
    return rows


def main() -> None:
    mode = sys.argv[1] if len(sys.argv) == 2 else ""
    seeds = json.loads(SEEDS.read_text())
    if mode == "pattern":
        table, helpers, function_keys = regexes(mathjax_dir())
        names = [n for n, _ in table] + function_keys + ["unknown-pattern"] + [n for n, _ in helpers]
        inputs = seeds["common"] + seeds["observe_groups"]
        rows = [{"name": n, "input": i} for n in names for i in inputs]
    elif mode == "regexp":
        table, helpers, _ = regexes(mathjax_dir())
        rows = [{"name": n, "source": s, "input": i} for n, s in table + helpers for i in seeds["common"]]
    elif mode == "texify":
        rows = texify_rows()
    elif mode == "expand":
        rows = expand_rows(seeds)
    else:
        sys.exit("usage: fixture-gen-mhchem.py pattern|regexp|texify|expand")
    mathtext_gen.write_output(rows)


if __name__ == "__main__":
    main()
