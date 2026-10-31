#!/usr/bin/env python3
"""Compare the in-process Rust math boundary with pinned D2 v0.9.0 MathJax.

The frozen hashes come from d2renderers/d2latex/latex_test.go.  Dimensions
alone are insufficient here: MathJax's SVG carries glyph paths, transforms,
error markup, and package-specific constructs that affect rendered output.
"""

import hashlib
import json
import subprocess
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, digest, verify_source


CASES = [
    ("basic", r"a + b = c", "39400be50e4d495cfb3c1704b68c38ab95597ea69bfbdbecef9c502343636f63", 72, 15),
    ("fraction", r"\frac{1}{2}", "f47560ab694c6a16451e73ce35b8480492153f2d3bb0f8323f7f295e3a42c68d", 18, 37),
    ("multiline", "a + b\n= c\n", "39400be50e4d495cfb3c1704b68c38ab95597ea69bfbdbecef9c502343636f63", 72, 15),
    ("scripts", r"x_i^2+\sum_{n=0}^{\infty}n^{-2}", "936458024f85e6628d73f2712b72b8f822aecd0d90d521740d0c2e27ae087280", 99, 51),
    ("delimiters", r"\left\langle \frac{x}{y}\middle|z\right\rangle", "0f3bd3fd536b90f6967d072311523481aa1e2d248afe7130cb446994d385296a", 60, 44),
    ("ams_matrix", r"\begin{pmatrix}a&b\\c&d\end{pmatrix}", "048e79bc11a8a6c287df5738697b99e7c52e05b01c15edc51289c5a04591efae", 64, 44),
    ("ams_align", r"\begin{aligned}a&=b+c\\d&=e\end{aligned}", "beea3e49eedda60558d71b4588c4c512c549123baa428013cefe1effbbd3e514", 72, 42),
    ("mathtools", r"\min_{\mathclap{\substack{x\in\mathbb{R}^n\\x\geq0\\Ax\leq b}}}c^Tx", "68dc56f03c3e1d51fcd77c7cb565cf17ae91e27f86e4bfa112bc3a2dd11105ea", 62, 55),
    ("amscd", r"\begin{CD}A@>f>>B\\@VgVV@VVhV\\C@>>k>D\end{CD}", "11db6e93c6718a81f80c269a01600fb0ddee08f30bd5765d323396400cc1de8b", 106, 112),
    ("braket", r"\bra{\psi}A\ket{\phi}+\braket{x|y}", "03df435de3e87e27eef1a9a10db840269ceebe1c8190dcac2b657a556d464f7b", 154, 19),
    ("cancel", r"\cancel{x}+\bcancel{y}+\xcancel{z}+\cancelto{0}{w}", "0c5aeba8d46e5a5f278a5a537d6687132f651f75b4d6717e5c6dee18b38d2a81", 155, 26),
    ("cases", r"f(x)=\begin{cases}x^2&x\geq0\\-x&x<0\end{cases}", "dd018141ff8cb02c1a2e5994998725a9966bdbf3c4bab05b48ae3df8e0f31d85", 159, 44),
    ("color", r"\color{red}{x}+\colorbox{yellow}{y}", "3cf885ed975f83b61c3977859092ebfdf1f18f85ecac23d7dfd866f19e5b1090", 53, 22),
    ("gensymb", r"90\degree+20\celsius+5\micro\ohm", "cc48530596511f31c9e3ac34819cad8cb1b229089663e94941b830f951b7fcfe", 134, 18),
    ("mhchem", r"\ce{2H2 + O2 -> 2H2O}", "32a36be4fc7ebaa02b6369c3acd571c11f16a8a9836b5a01d2c4437c70f67016", 165, 16),
    ("physics", r"\dv{x}f(x)+\qty(\frac{a}{b})", "b342affabb06996648d52fffcabfbf2f82b19b1abdd82fadd9ec8138430d5df8", 125, 38),
    ("invalid_merror", r"\frac{1}{", "c2d6ee6d434a5313546972d803487c1f505ea5ad70c94f94997cec11faef148b", 207, 18),
    ("d2_huge", r"\Huge{\frac{\alpha g^2}{\omega^5} e^{[ -0.74\bigl\{\frac{\omega U_\omega 19.5}{g}\bigr\}^{\!-4}\,]}}", "a200593e75830539243054b9da91acbe21bedd792c7147644cbb6bd14a7255b1", 382, 101),
    ("d2_emc2", r"e = mc^2", "e724fceadd904173810ae62ff609e48a78f1a777105a10cc23a21fce45f77052", 65, 18),
    ("d2_gibberish_sum", r"gibberish\; math:\sum_{i=0}^\infty i^2", "e0e37a92d36c8a44a9ab45c5fa97d5c7bcafade0e9c79169bcce9d6832ea3dcf", 179, 51),
    ("d2_linear_program", r"\min_{ \mathclap{\substack{ x \in \mathbb{R}^n \ x \geq 0 \ Ax \leq b }}} c^T x", "ff6a130d0d20e442931ba22d8f576feae1c47fcc7118e11957d34645232cedb1", 62, 32),
    ("d2_equation_split", "\\begin{equation} \\label{eq1}\n\\begin{split}\nA & = \\frac{\\\\pi r^2}{2} \\\\\n & = \\frac{1}{2} \\pi r^2\n\\end{split}\n\\end{equation}", "e00876f4f1c4aae7d7da24065e54f71cd940ed71d3f4edadb34fae406fc5a443", 82, 82),
    ("d2_limit", r"\lim_{h \rightarrow 0 } \frac{f(x+h)-f(x)}{h}", "4c33befe8c902c4ea5a2e30c62c978d9bacfc16d3c067cccc743c457446bc0b3", 162, 41),
    ("d2_quadratic", r"f(x) = x^2 + 2x + 1", "57c71028f1aa785897e79afe61723d9beb11ce56372247cba6480f2380083ae2", 150, 21),
    ("d2_physics_plugin", "\\var{F[g(x)]}\n\\dd(\\cos\\theta)", "4e6c3221a2cb2c21d32c8d66a51294f0839f03121a32e43e38e7d0ed89d828a7", 128, 19),
    ("d2_displaylines", "\\displaylines{x = a + b \\\\ y = b + c}\n\\sum_{k=1}^{n} h_{k} \\int_{0}^{1} \\bigl(\\partial_{k} f(x_{k-1}+t h_{k} e_{k}) -\\partial_{k} f(a)\\bigr) \\,dt", "8fa7020a6fa463de3a6bd656498cf9fd62518bda834f0895a6f42b7dbb8abbcf", 404, 52),
    ("d2_add", r"1 + 1", "b6eacdbf2d6531e407bfadc5d81bd3d5dc472bffa23cbc1908437553b0cfa072", 41, 14),
    ("backslash_parity", r"\bra{a}\ket{b}", "c58251f49b0317576946c88910847352c05e809e534f299cc4f3c12ce7e66bef", 37, 19),
]


def run(command, payload):
    return subprocess.run(command, input=json.dumps(payload).encode(), capture_output=True, check=True)


def main():
    verify_source()
    oracle = REFERENCE / "bin" / "d2-oracle"
    rust = ROOT / "target" / "release" / "examples" / "math-oracle"
    if not rust.exists():
        subprocess.run(["cargo", "build", "--release", "--offline", "-p", "d2-rs-math", "--example", "math-oracle"], cwd=ROOT, check=True)
    expressions = [expression for _, expression, *_ in CASES]
    left = json.loads(run([str(oracle), "math"], expressions).stdout)
    right = json.loads(run([str(rust)], [{"expression": expression} for expression in expressions]).stdout)
    rows = []
    for (name, expression, expected_hash, expected_width, expected_height), go, rust_row in zip(CASES, left, right):
        go_hash = hashlib.sha256((go.get("svg") or "").encode()).hexdigest()
        rust_hash = hashlib.sha256((rust_row.get("svg") or "").encode()).hexdigest()
        row = {
            "name": name,
            "expression": expression,
            "expected_sha256": expected_hash,
            "oracle_sha256": go_hash,
            "rust_sha256": rust_hash,
            "expected_dimensions": [expected_width, expected_height],
            "oracle_dimensions": [go.get("width"), go.get("height")],
            "rust_dimensions": [rust_row.get("width"), rust_row.get("height")],
            "oracle_error": [go.get("render_error"), go.get("measure_error")],
            "rust_error": [rust_row.get("render_error"), rust_row.get("measure_error")],
        }
        row["oracle_matches_frozen"] = go_hash == expected_hash and row["oracle_dimensions"] == [expected_width, expected_height]
        row["dimensions_match"] = row["oracle_dimensions"] == row["rust_dimensions"]
        row["svg_matches"] = go_hash == rust_hash
        row["matched"] = row["oracle_matches_frozen"] and row["dimensions_match"] and row["svg_matches"] and row["oracle_error"] == row["rust_error"]
        rows.append(row)
    report = {
        "boundary": "D2 v0.9.0 d2latex/mathjax-go versus native Rust math",
        "reference_tag": MANIFEST["d2"]["tag"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "cases": len(rows),
        "matched": sum(row["matched"] for row in rows),
        "dimension_matches": sum(row["dimensions_match"] for row in rows),
        "svg_matches": sum(row["svg_matches"] for row in rows),
        "oracle_frozen_matches": sum(row["oracle_matches_frozen"] for row in rows),
        "oracle_sha256": digest(oracle),
        "rust_sha256": digest(rust),
        "source_sha256": digest(verify_source() / "d2renderers/d2latex/latex_test.go"),
        "rows": rows,
    }
    output = ROOT / "oracle-cache/reports" / "math-parity.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in {"rows"}}, indent=2))
    raise SystemExit(report["matched"] != report["cases"])


if __name__ == "__main__":
    main()
