"""Generator for recipe mathtext-txt-font-subset (tools/fixtures/mathtext-txt.toml).

Builds tools/font-subset-oracle against the patched upstream tree (the
tools/oracle module with github.com/d2lang/d2 replaced, as regen-fixtures.py
does for go-program recipes) and writes, one JSON object per line inside an
array:

- crates/d2-rs-text/tests/fixtures/font_subset/oracle_cases.json:
  `font-subset-oracle cases` over every bundled family x style and the
  corpora of tools/fixtures/inputs/mathtext-txt/font-subset-corpora.json
  (cut SFNT, WOFF and GetEncodedSubset data-URI digests);
- crates/d2-rs-text/tests/fixtures/font_subset/golden_fonts.json:
  `font-subset-oracle golden` over every e2etests/testdata/**/*.exp.svg of
  the upstream tree (each unique embedded subset font with its recovered
  character set, re-run through GetEncodedSubset).
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CASES = "crates/d2-rs-text/tests/fixtures/font_subset/oracle_cases.json"
GOLDEN = "crates/d2-rs-text/tests/fixtures/font_subset/golden_fonts.json"
FAMILIES = ["SourceSansPro", "SourceCodePro", "HandDrawn"]
STYLES = ["regular", "bold", "semibold", "italic"]


def build(upstream: Path, work: Path) -> Path:
    module = work / "font-subset-oracle"
    module.mkdir()
    shutil.copy2(ROOT / "tools/font-subset-oracle/main.go", module / "main.go")
    go_mod, count = re.subn(r"(?m)^replace github\.com/d2lang/d2 => .*$", f"replace github.com/d2lang/d2 => {upstream}",
                            (ROOT / "tools/oracle/go.mod").read_text())
    if count != 1:
        sys.exit("tools/oracle/go.mod has no d2 replace line")
    (module / "go.mod").write_text(go_mod)
    shutil.copy2(ROOT / "tools/oracle/go.sum", module / "go.sum")
    binary = work / "bin" / "font-subset-oracle"
    # -mod=mod lets Go raise the scratch go.mod to the pin's own requirements
    # (tools/oracle/go.mod may lag them); the repository copy is never touched.
    subprocess.run(["go", "build", "-mod=mod", "-o", str(binary), "."], cwd=module, env=fixture_recipe.go_env(),
                   check=True, stdout=sys.stderr)
    return binary


def lines(rows: list[dict]) -> str:
    return "[\n" + ",\n".join(json.dumps(r) for r in rows) + "\n]\n"


def main() -> None:
    upstream = Path(fixture_recipe._env("D2RS_UPSTREAM"))
    work = fixture_recipe.workdir()
    binary = build(upstream, work)
    corpora = json.loads((ROOT / "tools/fixtures/inputs/mathtext-txt/font-subset-corpora.json").read_text())
    cases = work / "cases.json"
    cases.write_text(json.dumps([{"family": f, "style": s, "corpus": c} for f in FAMILIES for s in STYLES for c in corpora]))
    out = subprocess.run([str(binary), "cases", str(cases)], env=fixture_recipe.go_env(), check=True,
                         capture_output=True, text=True).stdout
    keys = ("family", "style", "corpus", "cut_sha256", "woff_sha256", "uri_sha256")
    fixture_recipe.output(CASES).write_text(lines([{k: r[k] for k in keys} for r in json.loads(out)]))
    svgs = sorted(str(p) for p in (upstream / "e2etests/testdata").rglob("*.exp.svg"))
    run = subprocess.run([str(binary), "golden", *svgs], env=fixture_recipe.go_env(), capture_output=True, text=True)
    sys.stderr.write(run.stderr)
    if run.returncode != 0:
        sys.exit(f"font-subset-oracle golden failed ({run.returncode})")
    rows = [json.loads(line) for line in run.stdout.splitlines() if line.strip()]
    for r in rows:
        if not r.get("golden_zlib_differs"):
            r.pop("golden_zlib_differs", None)
    fixture_recipe.output(GOLDEN).write_text(lines(rows))


if __name__ == "__main__":
    main()
