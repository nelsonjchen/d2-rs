#!/usr/bin/env python3
"""Score rendering under non-default CLI flags against the pinned Go CLI.

The e2e tiers render each board with default flags. This tier renders a fixed
set of upstream e2e boards under themes, sketch, padding, centering, scale,
dark theme and each layout engine, to SVG and PNG, with the pinned Go CLI and
d2-rs. SVG compares bytes exactly; PNG uses check-render-parity.py's
compare_png_outputs gate (decoded pixels and sizes, no tolerance). Exit codes
must agree when either side fails.

usage: flag-matrix-scoreboard.py <report.json> [rust-binary]
"""
import subprocess, sys, os, json, shutil, tempfile, itertools
import references
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"scripts"))
import importlib.util
spec=importlib.util.spec_from_file_location("rp",ROOT/"scripts/check-render-parity.py"); rp=importlib.util.module_from_spec(spec); spec.loader.exec_module(rp)
GO=references.GO_ORACLE; RS=(Path(sys.argv[2]) if len(sys.argv)>2 else ROOT/"target/diagnostic/d2-rs").resolve()
boards={}
for name in ["stable__hexagon_3d","stable__chaos2","txtar__sql-table-reserved","stable__md_code_inline","stable__direction","stable__arrowhead_labels","stable__container_edges","txtar__c4-person-label"]:
    p=references.E2E_CASES/f"{name}.json"
    if p.exists(): boards[name]=json.loads(p.read_text())["script"]
flags=[[],["--theme=1"],["--theme=4"],["--theme=8"],["--theme=101"],["--theme=104"],["--theme=200"],["--theme=300"],["--theme=303"],["--sketch"],["--pad=0"],["--pad=37"],["--center"],["--scale=0.5"],["--scale=2"],["--dark-theme=200"],["-l","elk"],["-l","tala"],["--sketch","--theme=300"]]
work=Path(tempfile.mkdtemp(prefix="flag-matrix-"))
res=[]; bad=0
for (bname,script),fl in itertools.product(boards.items(),flags):
    for ext in ["svg","png"]:
        d=work/"case"; shutil.rmtree(d,ignore_errors=True); d.mkdir()
        (d/"in.d2").write_text(script)
        out={}
        for n,b in (("go",GO),("rs",RS)):
            r=subprocess.run([str(b),*fl,"in.d2",f"{n}.{ext}"],cwd=d,capture_output=True,timeout=300)
            out[n]=r.returncode
        if out["go"]!=0 or out["rs"]!=0:
            ok = out["go"]==out["rs"]; detail=str(out)
        elif ext=="svg":
            ok=(d/"go.svg").read_bytes()==(d/"rs.svg").read_bytes(); detail=""
        else:
            c=rp.compare_png_outputs(d/"go.png",d/"rs.png"); ok=c["matched"]; detail=str({k:c.get(k) for k in ("different_pixels","go_size","rust_size")})
        if not ok:
            bad+=1; print("MISMATCH",bname," ".join(fl),ext,detail, flush=True)
        res.append({"board":bname,"flags":fl,"format":ext,"outcome":"match" if ok else "mismatch","detail":detail})
print(f"{len(res)-bad}/{len(res)} match")
shutil.rmtree(work)
out=Path(sys.argv[1]); out.parent.mkdir(parents=True,exist_ok=True)
out.write_text(json.dumps({"summary":{"match":len(res)-bad,"mismatch":bad},"rows":res},indent=1)+"\n")
