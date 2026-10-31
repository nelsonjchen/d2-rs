#!/usr/bin/env python3
"""Retain unequal-line edge PNG/SVG and diagnostic glyph witnesses."""
from __future__ import annotations
import argparse, hashlib, importlib.util, json, os, subprocess, time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
CASES={"long-short": ["A much longer first line", "short"], "short-long": ["short", "A much longer second line"], "equal": ["identical", "identical"], "single": ["A much longer first line"], "three-unequal": ["A much longer first line", "short", "A medium final line"], "blank-middle": ["A much longer first line", "", "short"], "three-equal": ["identical", "identical", "identical"]}
def inventory(path):
 data=path.read_bytes()
 return {"path":str(path.relative_to(ROOT)),"bytes":len(data),"sha256":hashlib.sha256(data).hexdigest()}
def main():
 parser=argparse.ArgumentParser()
 parser.add_argument("--rust",default=".reference/bin/d2-rs-cluster-alignment-diagnostic-20261002")
 parser.add_argument("--output",default=".reference/multiline-text-anchor-quoted-before-20261002")
 parser.add_argument("--extended",action="store_true",help="Also inspect three-line and blank-line source precision")
 args=parser.parse_args()
 output=ROOT/args.output
 output.mkdir(parents=True,exist_ok=False)
 binaries={"go":ROOT/".reference/bin/d2-v0.9.0-official","rust":ROOT/args.rust}
 env=dict(os.environ)
 removed=sorted(k for k in env if k.startswith(("D2_TRACE_","D2_RS_TRACE_","D2RS_","TALA_TRACE_")))
 for k in removed: del env[k]
 env["GOMAXPROCS"]="1"
 report={"schema_version":1,"reference_commit":"d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda","scope":"Independent newline anchoring; not full renderer or TALA qualification","probe_source":inventory(Path(__file__)),"binaries":{k:inventory(v) for k,v in binaries.items()},"environment":{"removed":removed,"overrides":{"GOMAXPROCS":"1"}},"cases":[],"runs":[]}
 (output/"probe-source-at-run.py").write_bytes(Path(__file__).read_bytes())
 spec=importlib.util.spec_from_file_location("pixel",ROOT/"scripts/check-render-parity.py")
 pixel=importlib.util.module_from_spec(spec);spec.loader.exec_module(pixel)
 report["pixel_decoder_source"]=inventory(ROOT/"scripts/check-render-parity.py")
 for case,lines in CASES.items():
  if not args.extended and case in ("three-unequal","blank-middle","three-equal"): continue
  directory=output/case;directory.mkdir()
  source=directory/"source.d2"
  source.write_text("a: A\nb: B\na -> b: "+json.dumps("\n".join(lines))+"\n")
  for side,binary in binaries.items():
   for extension in ("svg","png"):
    target=directory/f"{side}.{extension}"
    argv=[str(binary),f"--layout={'tala' if side=='go' else 'tala-rs'}","--tala-seeds=1"]
    if side=="rust": argv.extend([f"--stdout-format={extension}",str(source),"-"])
    else: argv.extend([str(source),str(target)])
    started=time.monotonic();run=subprocess.run(argv,cwd=ROOT,env=env,capture_output=True)
    for kind,data in (("stdout",run.stdout),("stderr",run.stderr)): (directory/f"{side}.{extension}.{kind}").write_bytes(data)
    if side=="rust": target.write_bytes(run.stdout)
    report["runs"].append({"case":case,"side":side,"format":extension,"argv":argv,"exit_code":run.returncode,"elapsed_seconds":time.monotonic()-started,"stdout":inventory(directory/f"{side}.{extension}.stdout"),"stderr":inventory(directory/f"{side}.{extension}.stderr"),"output":inventory(target) if target.exists() else None})
    (output/"report.json").write_text(json.dumps(report,indent=2)+"\n")
    if run.returncode: raise RuntimeError(f"{case} {side} {extension}: {run.stderr.decode(errors='replace')}")
  gw,gh,gp=pixel.decode_png(directory/"go.png");rw,rh,rp=pixel.decode_png(directory/"rust.png")
  differences=sum(gp[i:i+3]!=rp[i:i+3] for i in range(0,len(gp),3)) if (gw,gh)==(rw,rh) else None
  item={"case":case,"lines":lines,"source":inventory(source),"go_size":[gw,gh],"rust_size":[rw,rh],"different_pixels":differences,"matched":(gw,gh,gp)==(rw,rh,rp)}
  report["cases"].append(item)
  print(json.dumps(item),flush=True)
 # Existing feature-gated trace reconstructs exact per-glyph transforms.
 source=output/"long-short/source.d2"
 argv=[str(binaries["rust"]),"--layout=tala-rs","--tala-seeds=1","--stdout-format=png",str(source),"-"]
 traceenv={**env,"D2_RS_TRACE_RASTER_PIXEL":"0,0"}
 run=subprocess.run(argv,cwd=ROOT,env=traceenv,capture_output=True)
 for kind,data in (("stdout",run.stdout),("stderr",run.stderr)): (output/f"rust-long-short-diagnostic.{kind}").write_bytes(data)
 report["diagnostic"]={"argv":argv,"environment_overrides":{"GOMAXPROCS":"1","D2_RS_TRACE_RASTER_PIXEL":"0,0"},"exit_code":run.returncode,"stdout":inventory(output/"rust-long-short-diagnostic.stdout"),"stderr":inventory(output/"rust-long-short-diagnostic.stderr"),"ordinary_png_equal":run.stdout==(output/"long-short/rust.png").read_bytes()}
 for name,src in (("scanline.rs",ROOT/"crates/d2-rs-export/src/scanline.rs"),("labels.go",ROOT.parent/"d2/d2renderers/d2scenebuild/labels.go")):
  dest=output/name;dest.write_bytes(src.read_bytes())
 report["source_snapshots"]={name:inventory(output/name) for name in ("scanline.rs","labels.go")}
 (output/"report.json").write_text(json.dumps(report,indent=2)+"\n")
 return 0
if __name__=="__main__": raise SystemExit(main())
