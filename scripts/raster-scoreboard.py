#!/usr/bin/env python3
"""Score PNG output of every applicable upstream e2e board against pinned Go.

Each board from the e2e case dump (no measured texts, no expected error) is
rendered by the pinned Go CLI and by d2-rs with the same --layout and theme,
then compared with check-render-parity.py's compare_png_outputs gate: decoded
RGB pixels and canvas sizes for every artifact, no tolerance. Go failures are
reported separately and never count as matches. Rendered files are disposable
and deleted per board; only the JSON report is kept.

usage: raster-scoreboard.py <dagre|elk|tala> <report.json> <scratch-dir> [rust-binary]
"""
import json, sys, subprocess, tempfile, importlib.util, concurrent.futures, shutil

import references
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("rp", ROOT / "scripts/check-render-parity.py")
rp = importlib.util.module_from_spec(spec); sys.path.insert(0, str(ROOT / "scripts")); spec.loader.exec_module(rp)
GO = references.GO_ORACLE
RS = (Path(sys.argv[4]) if len(sys.argv) > 4 else ROOT / "target/diagnostic/d2-rs").resolve()
engine = sys.argv[1]; out = Path(sys.argv[2]); scratch = Path(sys.argv[3])
cases = []
for p in sorted(references.E2E_CASES.glob("*.json")):
    c = json.loads(p.read_text())
    if c["hasMtexts"] or c["expErr"] or not c["script"].strip(): continue
    if engine != "dagre" and c["justDagre"]: continue
    cases.append(c)
def run(c):
    work = Path(tempfile.mkdtemp(dir=scratch))
    try:
        (work / "in.d2").write_text(c["script"])
        args = [f"--layout={engine}"] + ([f"--theme={c['themeID']}"] if c.get("themeID") else [])
        res = {}
        for name, binary in [("go", GO), ("rs", RS)]:
            r = subprocess.run([str(binary), *args, "in.d2", f"{name}.png"], cwd=work, capture_output=True, timeout=180)
            res[name] = r.returncode
        if res["go"] != 0:
            return {"name": c["name"], "outcome": "go-failed"}
        if res["rs"] != 0:
            return {"name": c["name"], "outcome": "rust-failed"}
        cmp = rp.compare_png_outputs(work / "go.png", work / "rs.png")
        row = {"name": c["name"], "outcome": "match" if cmp["matched"] else "mismatch"}
        if not cmp["matched"]:
            row["detail"] = {k: cmp.get(k) for k in ("error", "different_pixels", "mean_absolute_channel_error", "go_size", "rust_size")}
            arts = cmp.get("artifacts") or []
            if len(arts) > 1:
                row["detail"]["artifacts"] = [(a["name"], a.get("different_pixels"), a.get("go_size"), a.get("rust_size")) for a in arts if not a["matched"]][:5]
        return row
    except subprocess.TimeoutExpired:
        return {"name": c["name"], "outcome": "timeout"}
    finally:
        shutil.rmtree(work, ignore_errors=True)
with concurrent.futures.ThreadPoolExecutor(8) as ex:
    rows = list(ex.map(run, cases))
from collections import Counter
summary = Counter(r["outcome"] for r in rows)
out.write_text(json.dumps({"engine": engine, "summary": summary, "rows": rows}, indent=1))
print(engine, dict(summary))
