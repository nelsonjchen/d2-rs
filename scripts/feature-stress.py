#!/usr/bin/env python3
"""Second stress sweep: language features, many boards, PPTX/PDF and
pathological shapes at growing size, d2-rs vs Go.

usage: feature-stress.py <d2-rs> [--go GO] [--sizes 10,100,1000] [--repeat 3] [--only fam,...]

Each family maps n to a dict of input files (input.d2 plus any imports), the
CLI layout and extra arguments, and the output extension. Rows report median
CPU and peak RSS ratios and whether outputs are byte-identical (PDF embeds a
date and is timed only). Flags: over Go, mismatch, Rust failure where Go
succeeds, Rust timeout, and CPU ratio growing with n.
"""
import argparse, hashlib, os, shutil, statistics, subprocess, tempfile, time

import references

def objs(n, prefix="o"):
    return "\n".join(f"{prefix}{i}: obj {i}" for i in range(n))

def chain_edges(n, prefix="o"):
    return "\n".join(f"{prefix}{i} -> {prefix}{i+1}" for i in range(n - 1))

def glob_style(n):
    return "\n".join([objs(n), chain_edges(n), "*.style.fill: \"#f6c\"", "*.shape: rectangle", "(* -> *)[*].style.stroke: red"])

def glob_nested(n):
    groups = max(2, int(n ** 0.5))
    lines = []
    for g in range(groups):
        lines.append(f"g{g}: {{")
        lines += [f"  m{i}: m {i}" for i in range(n // groups)]
        lines.append("}")
    lines += ["**.style.stroke-width: 3", "g*.style.fill: \"#eef\""]
    return "\n".join(lines)

def classes(n):
    k = max(1, n // 10)
    defs = ["classes: {"] + [f"  c{i}: {{style.fill: \"#{(i * 41) % 255:02x}aacc\"; shape: hexagon}}" for i in range(k)] + ["}"]
    uses = [f"o{i}.class: [c{i % k}; c{(i + 1) % k}]" for i in range(n)]
    return "\n".join(defs + [objs(n)] + uses + [chain_edges(n)])

def variables(n):
    k = max(1, n // 5)
    defs = ["vars: {"] + [f"  v{i}: value {i}" for i in range(k)] + ["}"]
    return "\n".join(defs + [f"o{i}: ${{v{i % k}}} and ${{v{(i * 7) % k}}}" for i in range(n)] + [chain_edges(n)])

def many_lines(n):
    return "\n".join([objs(n * 5), chain_edges(n * 5)] + [f"o{i} -> o{(i * 13) % (n * 5)}" for i in range(n * 5)])

def deep(n):
    depth = max(2, n // 10)
    path = ".".join(f"d{i}" for i in range(depth))
    return f"{path}: leaf\n{path}2: other\n{path} -> {path}2"

def imports(n):
    files = {"lib.d2": "\n".join(f"shared{i}: shared {i}" for i in range(n))}
    files["input.d2"] = "\n".join(["...@lib"] + [f"o{i}: obj {i}" for i in range(n)] + [f"o{i} -> shared{i}" for i in range(n)])
    return files

def boards(kind, n):
    out = ["start -> finish", f"{kind}: {{"]
    for i in range(n):
        out.append(f"  b{i}: {{\n    x{i} -> y{i}: board {i}\n    x{i}.style.fill: \"#{(i * 29) % 255:02x}cc88\"\n  }}")
    out.append("}")
    return "\n".join(out)

def grid_shadow(n):
    cols = max(2, int(n ** 0.5))
    return "\n".join([f"grid-columns: {cols}"] + [f"c{i}: cell {i}\nc{i}.style.shadow: true" for i in range(n)])

def parallel(n):
    return "\n".join(f"a -> b: e{i}" for i in range(n))

def self_loops(n):
    return "\n".join(f"a -> a: loop {i}" for i in range(n))

def long_label(n):
    return "big: " + " ".join(f"word{i}" for i in range(n)) + "\nbig -> other"

def md_table(n):
    rows = "\n".join(f"  | r{i} | **{i}** | `c{i}` |" for i in range(n))
    return f"t: ||md\n  | a | b | c |\n  |---|---|---|\n{rows}\n||\nt -> u"

def latex(n):
    return "\n".join(f"f{i}: |latex\n  \\\\sum_{{k=0}}^{{{i}}} k^2 = \\\\frac{{{i}({i}+1)(2\\\\cdot{i}+1)}}{{6}}\n|" for i in range(n))

def wrap(gen):
    return lambda n: (lambda r: r if isinstance(r, dict) else {"input.d2": r})(gen(n))

FAMILIES = {}
for eng in ("dagre", "elk", "tala"):
    FAMILIES[f"glob-style-{eng}"] = (wrap(glob_style), eng, [], "svg")
    FAMILIES[f"classes-{eng}"] = (wrap(classes), eng, [], "svg")
    FAMILIES[f"parallel-{eng}"] = (wrap(parallel), eng, [], "svg")
FAMILIES.update({
    "glob-nested": (wrap(glob_nested), "dagre", [], "svg"),
    "vars": (wrap(variables), "dagre", [], "svg"),
    "many-lines": (wrap(many_lines), "dagre", [], "svg"),
    "deep-nesting": (wrap(deep), "dagre", [], "svg"),
    "deep-nesting-elk": (wrap(deep), "elk", [], "svg"),
    "imports": (wrap(imports), "dagre", [], "svg"),
    "self-loops": (wrap(self_loops), "dagre", [], "svg"),
    "long-label": (wrap(long_label), "dagre", [], "svg"),
    "md-table": (wrap(md_table), "dagre", [], "svg"),
    "md-table-png": (wrap(md_table), "dagre", [], "png"),
    "latex": (wrap(latex), "dagre", [], "svg"),
    "layers-pptx": (wrap(lambda n: boards("layers", n)), "dagre", [], "pptx"),
    "scenarios-pptx": (wrap(lambda n: boards("scenarios", n)), "dagre", [], "pptx"),
    "steps-pptx": (wrap(lambda n: boards("steps", n)), "dagre", [], "pptx"),
    "steps-gif": (wrap(lambda n: boards("steps", n)), "dagre", ["--animate-interval=400"], "gif"),
    "layers-pdf": (wrap(lambda n: boards("layers", n)), "dagre", [], "pdf"),
    "grid-pptx": (wrap(grid_shadow), "dagre", [], "pptx"),
    "grid-pdf": (wrap(grid_shadow), "dagre", [], "pdf"),
    "chain-pptx": (wrap(lambda n: objs(n) + "\n" + chain_edges(n)), "dagre", [], "pptx"),
})

TEXTS = ["مرحبا بالعالم", "שלום עולם", "abc مرحبا 123 שלום", "é́ ñ ǟ", "👩‍👩‍👧‍👦 🏳️‍🌈 👍🏽",
         "‮RTL override‬", "Ελληνικά Кириллица ქართული", "漢字かなカナ한글", "x y z", "tab\there"]

def hard_text(n):
    return "\n".join([f"t{i}: \"{TEXTS[i % len(TEXTS)]} {i}\"" for i in range(n)] + [f"t{i} -> t{i+1}: \"{TEXTS[(i + 3) % len(TEXTS)]}\"" for i in range(n - 1)])

def extreme_dims(n):
    return "\n".join([f"w{i}: {{width: {1 + (i * 997) % 4000}; height: {1 + (i * 61) % 900}}}" for i in range(n)] + [f"w{i} -> w{i+1}" for i in range(n - 1)])

def grid_row(n):
    return "\n".join([f"grid-rows: 1"] + [f"c{i}" for i in range(n)])

def grid_col(n):
    return "\n".join([f"grid-columns: 1"] + [f"c{i}" for i in range(n)])

def themed(n):
    return "\n".join([f"n{i}: node {i}\nn{i}.style: {{shadow: true; 3d: true; border-radius: 4}}" for i in range(n)] + [f"n{i} -> n{i+1}: e" for i in range(n - 1)])

FAMILIES.update({
    "hard-text-svg": (wrap(hard_text), "dagre", [], "svg"),
    "hard-text-png": (wrap(hard_text), "dagre", [], "png"),
    "hard-text-elk": (wrap(hard_text), "elk", [], "svg"),
    "extreme-dims": (wrap(extreme_dims), "dagre", [], "svg"),
    "extreme-dims-png": (wrap(extreme_dims), "dagre", [], "png"),
    "grid-one-row": (wrap(grid_row), "dagre", [], "svg"),
    "grid-one-col": (wrap(grid_col), "dagre", [], "svg"),
    "scale-small": (wrap(themed), "dagre", ["--scale=0.05"], "png"),
    "scale-big": (wrap(themed), "dagre", ["--scale=8"], "png"),
    "pad-big": (wrap(themed), "dagre", ["--pad=2000"], "png"),
})
for theme in (0, 1, 3, 4, 5, 6, 8, 100, 101, 102, 103, 104, 105, 200, 201, 300, 301, 302, 303):
    FAMILIES[f"theme-{theme}"] = (wrap(themed), "dagre", [f"--theme={theme}"], "png")

def run(binary, d, layout, args, ext, timeout):
    out = os.path.join(d, f"output.{ext}")
    if os.path.exists(out):
        os.remove(out)
    start = time.monotonic()
    proc = subprocess.Popen([binary, f"--layout={layout}", *args, "input.d2", f"output.{ext}"], cwd=d,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env={**os.environ, "SOURCE_DATE_EPOCH": "946782245"})
    while True:
        pid, status, ru = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            break
        if time.monotonic() - start > timeout:
            proc.kill(); os.wait4(proc.pid, 0)
            return None
        time.sleep(0.005)
    ok = os.waitstatus_to_exitcode(status) == 0 and os.path.exists(out)
    digest = hashlib.sha256(open(out, "rb").read()).hexdigest() if ok else None
    return ok, ru.ru_utime + ru.ru_stime, ru.ru_maxrss / 1024.0, digest, os.waitstatus_to_exitcode(status)

def main():
    p = argparse.ArgumentParser()
    p.add_argument("rust"); p.add_argument("--go", default=str(references.GO_ORACLE))
    p.add_argument("--sizes", default="10,100,1000"); p.add_argument("--repeat", type=int, default=3)
    p.add_argument("--only", default=""); p.add_argument("--timeout", type=float, default=300)
    a = p.parse_args()
    sizes = [int(s) for s in a.sizes.split(",")]
    wanted = a.only.split(",") if a.only else list(FAMILIES)
    unknown = [f for f in wanted if f not in FAMILIES]
    if unknown:
        raise SystemExit(f"unknown families: {unknown}")
    flagged = []
    for fam in wanted:
        gen, layout, args, ext = FAMILIES[fam]
        ratios = []
        for n in sizes:
            d = tempfile.mkdtemp(prefix="stress2-")
            for name, text in gen(n).items():
                open(os.path.join(d, name), "w").write(text)
            g, r = [], []
            for _ in range(a.repeat):
                g.append(run(a.go, d, layout, args, ext, a.timeout)); r.append(run(a.rust, d, layout, args, ext, a.timeout))
            shutil.rmtree(d, ignore_errors=True)
            tag = f"{fam:18} n={n:5}"
            if any(x is None for x in g + r):
                gt, rt = any(x is None for x in g), any(x is None for x in r)
                print(f"{tag} timeout go={gt} rust={rt}", flush=True)
                if rt and not gt: flagged.append(f"{fam}@{n}:rust-timeout")
                continue
            gok, rok = all(x[0] for x in g), all(x[0] for x in r)
            if not gok:
                print(f"{tag} go-failed(exit {g[0][4]}) rust-ok={rok} rust-exit={r[0][4]}", flush=True)
                if r[0][4] not in (0, 1): flagged.append(f"{fam}@{n}:rust-exit-{r[0][4]}")
                continue
            if not rok:
                print(f"{tag} RUST-FAILED exit {r[0][4]} (Go succeeded)", flush=True); flagged.append(f"{fam}@{n}:rust-failed"); continue
            med = lambda xs, i: statistics.median(x[i] for x in xs)
            cpu = med(r, 1) / max(med(g, 1), 1e-6); rss = med(r, 2) / max(med(g, 2), 1e-6)
            same = len({x[3] for x in g + r}) == 1
            ratios.append((n, cpu)); mark = ""
            if cpu > 1.0 or rss > 1.0: mark += " OVER-GO"
            if not same and ext != "pdf": mark += " MISMATCH"
            print(f"{tag} cpu {cpu:5.2f}x rss {rss:5.2f}x  go {med(g,1):8.3f}s {med(g,2):7.1f}MiB  rust {med(r,1):8.3f}s {med(r,2):7.1f}MiB same={same}{mark}", flush=True)
            if mark: flagged.append(f"{fam}@{n}:{mark.strip()}")
        if len(ratios) >= 2 and ratios[-1][1] > ratios[0][1] * 1.5 and ratios[-1][1] > 0.6:
            print(f"{fam:18} GROWING cpu {ratios[0][1]:.2f} -> {ratios[-1][1]:.2f}", flush=True); flagged.append(f"{fam}@{ratios[-1][0]}:growing")
    print("FLAGGED:", " ".join(flagged) or "none")

if __name__ == "__main__":
    main()
