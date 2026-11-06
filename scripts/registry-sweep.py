#!/usr/bin/env python3
"""Registry enumeration: every shape, label/icon position, style keyword,
arrowhead and CLI flag D2 supports, one small board per case, d2-rs vs Go.

usage: registry-sweep.py <d2-rs> [--go GO] [--only shapes,styles,arrows,flags,special] [--out DIR] [--workers 8]

A case passes when both exit codes match and, if both succeed, output bytes
match (PDF is skipped for bytes). Mismatches are written to --out as
<group>-<n>.d2 plus a JSONL index with the arguments, exits and first error.
"""
import argparse, concurrent.futures as cf, hashlib, itertools, json, os, re, subprocess, tempfile

import references

SHAPES = ["rectangle", "square", "page", "parallelogram", "document", "cylinder", "queue", "package", "step", "callout",
          "stored_data", "person", "diamond", "oval", "circle", "hexagon", "cloud", "text", "c4-person"]
NEARS = ["top-left", "top-center", "top-right", "center-left", "center-center", "center-right", "bottom-left",
         "bottom-center", "bottom-right", "outside-top-left", "outside-top-center", "outside-bottom-right",
         "outside-left-center", "outside-right-center", "border-top-center", "border-bottom-left"]
SIZES = [None, (200, 31), (40, 40), (300, 25), (31, 200), (10, 10)]
ARROWS = ["triangle", "arrow", "diamond", "circle", "box", "cf-one", "cf-many", "cf-one-required", "cf-many-required", "cross"]
STYLES = {
    "opacity": ["0", "0.4", "1"], "stroke": ["red", "\"#123\"", "transparent"], "fill": ["\"#fafafa\"", "transparent", "\"linear-gradient(#000, #fff)\""],
    "fill-pattern": ["dots", "lines", "grain", "paper", "none"], "stroke-width": ["1", "5", "15"], "stroke-dash": ["0", "3", "10"],
    "border-radius": ["0", "8", "999"], "shadow": ["true"], "3d": ["true"], "multiple": ["true"], "double-border": ["true"],
    "font": ["mono"], "font-size": ["8", "20", "55"], "font-color": ["red", "\"#00ff00\""], "bold": ["true", "false"],
    "italic": ["true"], "underline": ["true"], "text-transform": ["uppercase", "lowercase", "title", "none"],
}
FLAGS = [["--sketch"], ["--center"], ["--pad=0"], ["--pad=500"], ["--scale=0.5"], ["--scale=2"], ["--theme=1"], ["--theme=200"],
         ["--theme=300"], ["--dark-theme=200"], ["--theme=0", "--dark-theme=201"], ["--salt=abc"], ["--omit-version"],
         ["--no-xml-tag"], ["--sketch", "--theme=300"], ["--center", "--pad=0"], ["--layout=elk"], ["--layout=tala"],
         ["--target=*"], ["--animate-interval=400"]]
BOARD = "a: Alpha\nb: Beta\na -> b: label\nc: {d -> e}\nlayers: {l1: {x -> y}}\n"

def cases(groups):
    n = 0
    if "shapes" in groups:
        for shape, size in itertools.product(SHAPES, SIZES):
            dims = f"; width: {size[0]}; height: {size[1]}" if size else ""
            for fmt in ("svg", "png"):
                yield "shapes", n, f"s: Label text {{shape: {shape}{dims}}}\nt: other\ns -> t", [], fmt; n += 1
        for shape, near in itertools.product(["rectangle", "circle", "cylinder", "person", "hexagon"], NEARS):
            yield "shapes", n, f"s: Label {{shape: {shape}; label.near: {near}; icon: https-free-icon}}\nt -> s".replace("icon: https-free-icon", ""), [], "svg"; n += 1
            yield "shapes", n, f"s: Label {{shape: {shape}; label.near: {near}; width: 160; height: 90}}\nt -> s", [], "svg"; n += 1
    if "styles" in groups:
        for key, values in STYLES.items():
            for value in values:
                for target in ("node", "edge"):
                    body = f"a: A {{style.{key}: {value}}}\na -> b" if target == "node" else f"a -> b: e {{style.{key}: {value}}}"
                    for fmt in ("svg", "png"):
                        yield "styles", n, body, [], fmt; n += 1
    if "arrows" in groups:
        for arrow, filled, end in itertools.product(ARROWS, (None, "true", "false"), ("source", "target")):
            fill = f"; {end}-arrowhead.style.filled: {filled}" if filled else ""
            for label in ("", f"; {end}-arrowhead.label: 1"):
                body = f"a -> b: e {{{end}-arrowhead.shape: {arrow}{fill}{label}}}"
                yield "arrows", n, body, [], "svg"; n += 1
    if "special" in groups:
        for c in special_cases(n):
            yield c
    if "flags" in groups:
        for flags in FLAGS:
            for fmt in ("svg", "png"):
                yield "flags", n, BOARD, flags, fmt; n += 1

SEQ = {
    "basic": "shape: sequence_diagram\na; b; c\na -> b: hi\nb -> c: there\nc -> a: back",
    "self": "shape: sequence_diagram\na; b\na -> a: self\na -> b",
    "span": "shape: sequence_diagram\na; b\na.t1 -> b.t1: call\nb.t1 -> a.t1: ret",
    "nested-span": "shape: sequence_diagram\na; b\na.t1.t2 -> b.t3: deep\nb.t3 -> a.t1",
    "group": "shape: sequence_diagram\na; b; c\ng: {\n  a -> b: in group\n}\nb -> c",
    "note": "shape: sequence_diagram\na; b\na.\"a note\"\na -> b\nb.\"another\nnote\"",
    "nested-group": "shape: sequence_diagram\na; b\nouter: {\n  inner: {\n    a -> b\n  }\n  b -> a\n}",
    "md-note": "shape: sequence_diagram\na; b\na.n: |md\n  **bold** note\n|\na -> b",
    "styled": "shape: sequence_diagram\na; b\na -> b: x {style.stroke-dash: 3}\nb.style.fill: red",
    "many-actors": "shape: sequence_diagram\n" + "\n".join(f"p{i} -> p{i+1}: m{i}" for i in range(12)),
}
GRID = {
    "basic": "grid-rows: 2\ngrid-columns: 3\na; b; c; d; e; f",
    "gap": "grid-columns: 3\ngrid-gap: 0\na; b; c; d",
    "vgap": "grid-columns: 2\nvertical-gap: 40\nhorizontal-gap: 5\na; b; c; d",
    "nested": "grid-columns: 2\na: {grid-rows: 2; x; y}\nb; c",
    "edges-in": "g: {grid-columns: 2; a; b; c}\ng.a -> g.c\nout -> g.b",
    "sized": "grid-columns: 2\na: {width: 300}\nb: {height: 90}\nc; d",
    "labels": "g: Big grid label {grid-columns: 2; a; b}",
    "dims-mixed": "grid-rows: 1\na: {width: 30; height: 30}\nb: {width: 300; height: 100}",
}
SQLCLASS = {
    "sql": "t: {shape: sql_table\n  id: int {constraint: primary_key}\n  name: varchar(40)\n  owner: int {constraint: foreign_key}\n  e: text {constraint: [unique; primary_key]}\n}\nu: {shape: sql_table; id: int}\nt.owner -> u.id",
    "sql-long": "t: {shape: sql_table\n  a_very_long_column_name_here: \"a very long type name(with, args)\"\n  b: int\n}",
    "class": "c: {shape: class\n  +field: int\n  -priv: string\n  #prot(a int): bool\n  \"method(x, y)\": \"(int, error)\"\n}",
    "class-empty": "c: {shape: class}",
    "class-edge": "a: {shape: class; x: int}\nb: {shape: class; y: int}\na -> b: uses",
}
MD = {
    "headers": "m: |md\n  # H1\n  ## H2\n  ### H3\n  #### H4\n|",
    "lists": "m: |md\n  - a\n  - b\n    - nested\n  1. one\n  2. two\n|",
    "table": "m: ||md\n  | a | b |\n  |---|:-:|\n  | 1 | 2 |\n||",
    "code": "m: |md\n  inline `code` and\n  ```\n  block code\n  ```\n|",
    "quote": "m: |md\n  > quoted\n  > text\n|",
    "links": "m: |md\n  [link](https://example.com) and <https://x.y>\n|",
    "emph": "m: |md\n  *em* **strong** ***both*** ~~strike~~\n|",
    "hr": "m: |md\n  above\n\n  ---\n\n  below\n|",
    "long-para": "m: |md\n  " + " ".join(f"word{i}" for i in range(80)) + "\n|",
    "mixed-script": "m: |md\n  # Ελληνικά 漢字 مرحبا\n  - emoji 😀 list\n|",
}
NEAR = {
    "root-near": "title: Title {near: top-center}\na -> b",
    "root-near-all": "\n".join(f"n{i}: {pos} {{near: {pos}}}" for i, pos in enumerate(["top-left", "top-center", "top-right", "center-left", "center-right", "bottom-left", "bottom-center", "bottom-right"])) + "\na -> b",
    "near-object": "a -> b\nnote: beside b {near: b}",
    "icon-near": "a: A {icon: ./icon.svg; icon.near: top-left}\nb: B {icon: ./icon.svg}\nc: {shape: image; icon: ./icon.svg}\na -> b -> c",
    "icon-png": "a: A {icon: ./icon.png}\nb: {shape: image; icon: ./icon.png}\na -> b",
    "edge-labels": "a -> b: mid {source-arrowhead.label: s; target-arrowhead.label: t}",
    "edge-label-near": "a -> b: mid {label.near: outside-top-center}",
    "container-edges": "x: {a; b}\ny: {c}\nx.a -> y.c\nx -> y\nx.b -> x.a",
    "tooltip-link": "a: {tooltip: hello; link: https://example.com}\nb: {link: layers.l}\na -> b\nlayers: {l: {z}}",
}
import base64
ICON_PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAKklEQVR4nGNgGAWjYBSMglEwCkbBKBgFo2AUjIJRMApGwSgYBaNgFIyCUQAAuAQAAXhm0nEAAAAASUVORK5CYII=")
ICON_SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="#36c"/></svg>'

def special_cases(n):
    for group, table, fmts, engines in (("seq", SEQ, ("svg", "png"), ("dagre", "elk", "tala")), ("grid", GRID, ("svg", "png"), ("dagre", "elk", "tala")),
                                         ("sqlclass", SQLCLASS, ("svg", "png"), ("dagre", "elk", "tala")), ("md", MD, ("svg", "png"), ("dagre",)),
                                         ("near", NEAR, ("svg", "png"), ("dagre", "elk", "tala"))):
        for name, body in table.items():
            for fmt in fmts:
                for eng in engines:
                    for sketch in ([], ["--sketch"]):
                        if sketch and (eng != "dagre" or fmt != "svg"): continue
                        yield group, f"{name}-{eng}-{fmt}{'-sketch' if sketch else ''}", body, [f"--layout={eng}", *sketch], fmt

def run(binary, d, args, fmt, name):
    layout = [] if any(a.startswith("--layout") for a in args) else ["--layout=dagre"]
    try:
        p = subprocess.run([binary, *layout, *args, "input.d2", f"{name}.{fmt}"], cwd=d, capture_output=True, timeout=60,
                           env={**os.environ, "SOURCE_DATE_EPOCH": "946782245"})
    except subprocess.TimeoutExpired:
        return "timeout", "", None
    out = os.path.join(d, f"{name}.{fmt}")
    digest = hashlib.sha256(open(out, "rb").read()).hexdigest() if p.returncode == 0 and os.path.exists(out) else None
    err = next((l for l in p.stderr.decode("utf-8", "replace").splitlines() if "err" in l.lower()), "")
    return p.returncode, err.replace(d, "<dir>"), digest

def check(job):
    rust, go, group, n, body, args, fmt = job
    d = tempfile.mkdtemp(prefix="en-"); open(os.path.join(d, "input.d2"), "w").write(body)
    open(os.path.join(d, "icon.svg"), "w").write(ICON_SVG); open(os.path.join(d, "icon.png"), "wb").write(ICON_PNG)
    g = run(go, d, args, fmt, "go"); r = run(rust, d, args, fmt, "rust"); subprocess.run(["rm", "-rf", d])
    if g[0] == "timeout" or r[0] == "timeout":
        kind = "rust-timeout" if r[0] == "timeout" and g[0] != "timeout" else None
    elif (g[0] == 0) != (r[0] == 0): kind = "exit-diff"
    elif g[0] == 0 and g[2] != r[2] and fmt != "pdf": kind = "output-diff"
    elif g[0] != 0 and re.sub(r"\s+", " ", g[1]) != re.sub(r"\s+", " ", r[1]): kind = "error-diff"
    else: kind = None
    return group, n, body, args, fmt, kind, g[0], r[0], g[1], r[1]

def main():
    a = argparse.ArgumentParser(); a.add_argument("rust"); a.add_argument("--go", default=str(references.GO_ORACLE))
    a.add_argument("--only", default="shapes,styles,arrows,flags,special"); a.add_argument("--out", default=str(references.REPORTS / "registry-sweep"))
    a.add_argument("--workers", type=int, default=8)
    o = a.parse_args(); os.makedirs(o.out, exist_ok=True)
    jobs = [(o.rust, o.go, *c) for c in cases(o.only.split(","))]
    totals, index = {}, open(os.path.join(o.out, "findings.jsonl"), "w")
    with cf.ThreadPoolExecutor(o.workers) as ex:
        for group, n, body, args, fmt, kind, ge, re_, gerr, rerr in ex.map(check, jobs):
            t = totals.setdefault(group, {"cases": 0}); t["cases"] += 1
            if kind:
                t[kind] = t.get(kind, 0) + 1
                name = f"{group}-{n}.d2"; open(os.path.join(o.out, name), "w").write(body)
                index.write(json.dumps({"group": group, "n": n, "file": name, "args": args, "fmt": fmt, "kind": kind,
                                        "go_exit": ge, "rust_exit": re_, "go_err": gerr[:240], "rust_err": rerr[:240]}) + "\n")
    print("DONE", json.dumps(totals), flush=True)

if __name__ == "__main__":
    main()
