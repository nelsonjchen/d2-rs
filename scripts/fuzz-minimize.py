#!/usr/bin/env python3
"""Greedy line/token minimizer for fuzz witnesses (d2-rs vs Go divergence).

usage: fuzz-minimize.py <d2-rs> <go-d2> <findings.jsonl> <witness-dir> <layout> [--limit 60]

For each finding, repeatedly drops line chunks (halving) and then single
characters inside short files while the same divergence kind persists.
Prints each minimized input, de-duplicated, with its kind.
"""
import hashlib, json, os, subprocess, sys, tempfile

def outcome(binary, text, layout):
    d = tempfile.mkdtemp(prefix="mn-"); open(os.path.join(d, "input.d2"), "w").write(text)
    try:
        p = subprocess.run([binary, f"--layout={layout}", "input.d2", "o.svg"], cwd=d, capture_output=True, timeout=20,
                           env={**os.environ, "SOURCE_DATE_EPOCH": "946782245"})
        o = os.path.join(d, "o.svg"); h = hashlib.sha256(open(o, "rb").read()).hexdigest() if p.returncode == 0 and os.path.exists(o) else None
        r = (p.returncode, h)
    except subprocess.TimeoutExpired:
        r = ("timeout", None)
    subprocess.run(["rm", "-rf", d]); return r

def kind(rust, go, text, layout):
    g, r = outcome(go, text, layout), outcome(rust, text, layout)
    if "timeout" in (g[0], r[0]): return None
    if (g[0] == 0) != (r[0] == 0): return "exit-diff"
    if g[0] == 0 and g[1] != r[1]: return "output-diff"
    return None

def minimize(rust, go, text, layout, want):
    lines = text.split("\n"); chunk = max(1, len(lines) // 2)
    while chunk >= 1:
        i, changed = 0, False
        while i < len(lines):
            trial = lines[:i] + lines[i + chunk:]
            if trial and kind(rust, go, "\n".join(trial), layout) == want:
                lines, changed = trial, True
            else:
                i += chunk
        if not changed: chunk //= 2
    text = "\n".join(lines)
    if len(text) < 400:
        i = 0
        while i < len(text):
            trial = text[:i] + text[i + 1:]
            if trial.strip() and kind(rust, go, trial, layout) == want: text = trial
            else: i += 1
    return text

def main():
    rust, go, index, wdir, layout = sys.argv[1:6]; limit = int(sys.argv[7]) if len(sys.argv) > 7 else 60
    seen = {}
    rows = [json.loads(l) for l in open(index) if json.loads(l)["kind"] in ("output-diff", "exit-diff")][:limit]
    for r in rows:
        text = open(os.path.join(wdir, r["file"])).read()
        if kind(rust, go, text, layout) != r["kind"]: continue
        small = minimize(rust, go, text, layout, r["kind"])
        key = (r["kind"], small.strip())
        seen.setdefault(key, []).append(r["file"])
        print(f"--- {r['kind']} from {r['file']}\n{small}\n", flush=True)
    print("UNIQUE", len(seen))
    for (k, s), files in sorted(seen.items(), key=lambda x: -len(x[1])):
        print(f"=== {k} x{len(files)}\n{s}\n")

if __name__ == "__main__":
    main()
