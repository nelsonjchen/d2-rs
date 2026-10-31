#!/usr/bin/env python3
"""Capture exact ordered RNG calls and draws from native D2 and pinned Go.

All raw files are immutable per suffix. No event, scope, value or tail is
normalized; startup/renderer stderr outside RNG is retained separately.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/"scripts"))
from exact_json import difference

def file_record(path):
 return {"path":str(path.relative_to(ROOT)),"bytes":path.stat().st_size,
         "sha256":hashlib.sha256(path.read_bytes()).hexdigest()}

def events(data):
 result=[]
 for line in data.decode().splitlines():
  if not line.startswith("{"): continue
  row=json.loads(line)
  if str(row.get("event","")).startswith("rng"):
   assert row["event"] in {"rng","rng-init","rng-scope","rng-operation"},row
   result.append(row)
 return result

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument("--suffix",required=True)
 parser.add_argument("--seeds",type=int,nargs="+",default=[1])
 parser.add_argument("--fixtures",nargs="+",default=["tests/d2/tala-source-arrow-repeated-labels.d2"])
 parser.add_argument("--repeats",type=int,default=1)
 args=parser.parse_args()
 destination=ROOT/f".reference/tala-rng-operations-{args.suffix}"
 destination.mkdir(exist_ok=False)
 binaries={"official":ROOT/".reference/bin/d2-v0.9.0-official",
           "go":ROOT/".reference/bin/d2-v0.9.0-rng-operation-trace",
           "rust":ROOT/"target/diagnostic/d2-rs"}
 hashes={side:file_record(path) for side,path in binaries.items()}
 base={k:v for k,v in os.environ.items() if not k.startswith(("D2_TRACE_","D2RS_","TALA_TRACE_","WEFTAN_")) and k not in {"DEV_MODE","TEST_MODE"}}
 runs=[]; comparisons=[]; seen={}; nondeterminism=[]
 for fixture in args.fixtures:
  source=ROOT/fixture
  for seed in args.seeds:
   for repeat in range(args.repeats):
    folder=destination/f"{source.stem}-seed-{seed}-repeat-{repeat}";folder.mkdir()
    captures={}
    for side,binary in binaries.items():
     modes=["disabled","enabled"] if side!="official" else ["disabled"]
     for mode in modes:
      overrides={}
      if mode=="enabled":
       prefix="TALA" if side=="go" else "WEFTAN"
       overrides[f"{prefix}_TRACE_RNG_JSONL"]="1"
       overrides["D2_TRACE_RNG_OPERATIONS" if side=="go" else "D2RS_TRACE_RNG_OPERATIONS"]="1"
      svg=folder/f"{side}-{mode}.svg"
      argv=[str(binary),f"--layout={'tala-rs' if side=='rust' else 'tala'}",f"--tala-seeds={seed}",str(source),str(svg)]
      run=subprocess.run(argv,cwd=ROOT,env=base|overrides,capture_output=True,timeout=120)
      (folder/f"{side}-{mode}.stdout").write_bytes(run.stdout)
      (folder/f"{side}-{mode}.stderr").write_bytes(run.stderr)
      assert run.returncode==0,run.stderr.decode()
      parsed=events(run.stderr)
      if mode=="disabled": assert not parsed,(side,parsed[:1])
      captures[side,mode]={"events":parsed,"svg":svg.read_bytes()}
      runs.append({"fixture":fixture,"fixture_sha256":file_record(source)["sha256"],"seed":seed,"repeat":repeat,
                   "side":side,"mode":mode,"argv":argv,"environment_overrides":overrides,"exit_code":run.returncode,
                   "event_counts":dict(Counter(row["event"] for row in parsed))})
    go=captures["go","enabled"]["events"];rust=captures["rust","enabled"]["events"]
    families={name:{"go":sum(r["event"]==name for r in go),"rust":sum(r["event"]==name for r in rust)} for name in ["rng-init","rng-scope","rng","rng-operation"]}
    assert all(go and counts["go"] for counts in families.values()),families
    primitives={side:[row for row in rows if row["event"]!="rng-operation"] for side,rows in [("go",go),("rust",rust)]}
    observer_same={side:captures[side,"disabled"]["svg"]==captures[side,"enabled"]["svg"] for side in ["go","rust"]}
    first=difference(go,rust)
    ordered_first=next(({'index':i,'go':a,'rust':b} for i,(a,b) in enumerate(zip(go,rust)) if difference(a,b)),None)
    comparisons.append({"fixture":fixture,"seed":seed,"repeat":repeat,"event_counts":families,"ordered_rng_equal":first is None,
                        "first_difference":first,"first_common_event_difference":ordered_first,
                        "primitive_draw_init_equal":difference(primitives["go"],primitives["rust"]) is None,
                        "unmatched_tail_counts":{"go":max(0,len(go)-len(rust)),"rust":max(0,len(rust)-len(go))},
                        "observer_svg_identical":observer_same,"instrumented_go_svg_matches_official":captures["go","disabled"]["svg"]==captures["official","disabled"]["svg"]})
    for side,rows in [("go",go),("rust",rust)]:
     key=(fixture,seed,side)
     if key in seen and difference(seen[key],rows):nondeterminism.append({"fixture":fixture,"seed":seed,"side":side,"repeat":repeat,"difference":difference(seen[key],rows)})
     seen.setdefault(key,rows)
    print(json.dumps(comparisons[-1]),flush=True)
 for side,path in binaries.items():assert file_record(path)==hashes[side]
 report={"source_commit":"d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda","scope":"Exact ordered RNG initialization, primitive draws and semantic Float64/Int63/Shuffle call/swap/result boundaries; other layout operation families are outside this gate.",
         "binaries":hashes,"runs":runs,"comparisons":comparisons,"nondeterminism":nondeterminism,"normalization":"None; exact identities, values, float bits, event and swap order and missing tails.",
         "passed":all(c["ordered_rng_equal"] and all(c["observer_svg_identical"].values()) and c["instrumented_go_svg_matches_official"] for c in comparisons) and not nondeterminism,
         "raw_artifacts":[file_record(path) for path in sorted(destination.rglob("*")) if path.is_file()],"script":file_record(Path(__file__)),"full_operation_trace_parity":False,"full_goal_complete":False}
 output=ROOT/f"docs/tala-rng-operations-{args.suffix}.json";assert not output.exists();output.write_text(json.dumps(report,indent=2)+"\n")
 return 0 if report["passed"] else 1

if __name__=="__main__":raise SystemExit(main())
