#!/usr/bin/env python3
"""Score the native d2scene / d2scenebuild / d2raster ports against pinned Go.

Inputs are the upstream e2e goldens' laid-out diagrams (e2etests/testdata/
<case>/<engine>/board.exp.json) with the case theme, as for the isometric
scoreboard. tools/scene-probe (Go, built in the trace checkout) builds every
board with d2scenebuild (Pad 0, config overrides, Appendix, no asset or font
resolvers) and, in raster mode, renders it with d2raster at Scale 1 on white
under the CLI's production ceilings. Font bytes are written once to a
content-addressed store.

Tiers (each compares every Go output; missing Rust output fails):
* model: crates/d2-rs-scene decodes and re-encodes each Go Document with no
  value change (font file references are compared as references).
* scenebuild: the Rust builder's Document JSON equals Go's, value by value, or
  the error text is identical.
* raster: the Rust renderer's pixels for Go's Document equal Go's PNG
  (decoded RGBA, compared with the raster gate's collector), or the error
  text is identical.

The Rust side of each tier is a command reading the same request lines on
stdin (see --rust). Captures are budgeted (default 1 GiB).

usage: scene-scoreboard.py --tier model|scenebuild|raster --capture <fresh dir> --output <report.json>
"""
from __future__ import annotations

import argparse
import collections
import importlib.util
import json
from pathlib import Path
import subprocess

import references

ROOT = Path(__file__).resolve().parents[1]
TESTDATA = ROOT.parent / "d2" / "e2etests/testdata"
GO_PROBE = ROOT / ".reference/bin/d2-scene-probe-6a7b4837"
RUST = {
    "model": ROOT / "target/diagnostic/examples/scene-roundtrip",
    "scenebuild": ROOT / "target/diagnostic/examples/scenebuild-probe",
    "raster": ROOT / "target/diagnostic/examples/raster-probe",
}


def raster_gate():
    spec = importlib.util.spec_from_file_location("scene_raster_gate", ROOT / "scripts/check-render-parity.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def requests(out, mode, assets, only):
    cases = {}
    for path in references.E2E_CASES.glob("*.json"):
        case = json.loads(path.read_text())
        cases[case["name"]] = case
    rows = []
    for board in sorted(TESTDATA.rglob("board.exp.json")):
        relative = board.parent.relative_to(TESTDATA).as_posix()
        name = relative.rsplit("/", 1)[0]
        case = cases.get(name)
        if case is None or (only and only not in relative):
            continue
        rows.append({"board": str(board), "themeID": case.get("themeID"), "out": str(out / relative),
                     "mode": mode, "assets": str(assets)})
    return rows


def run(command, rows, log, stdin_rows=True):
    with log.open("w") as stream:
        result = subprocess.run(command, input="\n".join(json.dumps(r) for r in rows) + "\n",
                                text=True, stdout=stream, stderr=subprocess.STDOUT, timeout=14400)
    return result.returncode


def size(path):
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tier", choices=sorted(RUST), required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--go", type=Path, default=GO_PROBE)
    parser.add_argument("--filter", default="")
    parser.add_argument("--budget", type=int, default=1 << 30)
    args = parser.parse_args()
    capture = args.capture.resolve()
    assert not capture.exists(), "use a fresh capture path"
    capture.mkdir(parents=True)
    assets = capture / "assets"
    go_mode = "raster" if args.tier == "raster" else "scenebuild"
    runs = {"go": run([str(args.go.resolve())], requests(capture / "go", go_mode, assets, args.filter), capture / "go.log")}
    if size(capture) > args.budget:
        raise SystemExit(f"capture budget exceeded: {size(capture)} > {args.budget}")
    rust = (args.rust or RUST[args.tier]).resolve()
    go_files = sorted(p for p in (capture / "go").rglob("*") if p.is_file())
    if args.tier == "model":
        documents = [str(p) for p in go_files if p.name.endswith(".scene.json")]
        runs["rust"] = subprocess.run([str(rust), *documents], capture_output=True, text=True).returncode
    else:
        # The Rust probe reads the same requests; raster requests point at the
        # Go documents, which it must render.
        rows = requests(capture / "rust", go_mode, assets, args.filter)
        for row in rows:
            row["go"] = row["out"].replace(str(capture / "rust"), str(capture / "go"), 1)
        runs["rust"] = run([str(rust)], rows, capture / "rust.log")
    if size(capture) > args.budget:
        raise SystemExit(f"capture budget exceeded: {size(capture)} > {args.budget}")
    gate = raster_gate() if args.tier == "raster" else None
    rows = []
    for go_path in go_files:
        rel = go_path.relative_to(capture / "go")
        if args.tier == "model":
            if not go_path.name.endswith(".scene.json"):
                continue
            rust_path = go_path.with_name(go_path.name + ".rs.json")
            row = {"board": rel.as_posix()}
            if not rust_path.is_file():
                row["outcome"] = "rust-missing"
            else:
                go_value = json.loads(go_path.read_bytes())
                rust_value = json.loads(rust_path.read_bytes())
                for asset in rust_value.get("Assets", {}).values():
                    if "FontAsset" in asset and asset["FontAsset"].get("Data") == "":
                        del asset["FontAsset"]["Data"]
                row["outcome"] = "match" if go_value == rust_value else "mismatch"
            rows.append(row)
            continue
        if args.tier == "scenebuild" and go_path.suffix == ".png":
            continue
        if args.tier == "raster" and go_path.name.endswith(".scene.json"):
            continue
        rust_path = capture / "rust" / rel
        row = {"board": rel.as_posix()}
        if go_path.suffix == ".err":
            alternative = [p for p in rust_path.parent.glob(rel.name.split(".")[0] + ".*") if p.suffix != ".err"]
            if rust_path.is_file():
                row["outcome"] = "match" if rust_path.read_bytes() == go_path.read_bytes() else "error-mismatch"
            else:
                row["outcome"] = "rust-unexpected-output" if alternative else "rust-missing"
        elif not rust_path.is_file():
            err = rust_path.with_name(rel.name.split(".")[0] + ".err")
            row["outcome"] = "rust-error" if err.is_file() else "rust-missing"
            if err.is_file():
                row["rust_error"] = err.read_text()[:300]
        elif go_path.suffix == ".png":
            result = gate.compare_png_outputs(go_path, rust_path)
            row["outcome"] = "match" if result["matched"] else "mismatch"
            if not result["matched"]:
                row["detail"] = {k: result.get(k) for k in ("error", "different_pixels", "go_size", "rust_size")}
        else:
            row["outcome"] = "match" if json.loads(go_path.read_bytes()) == json.loads(rust_path.read_bytes()) else "mismatch"
        rows.append(row)
    summary = dict(collections.Counter(row["outcome"] for row in rows))
    report = {"tier": args.tier, "source_commit": references.SOURCE_COMMIT, "runs": runs,
              "summary": summary, "rows": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(args.tier, summary, "boards", len(rows))


if __name__ == "__main__":
    main()
