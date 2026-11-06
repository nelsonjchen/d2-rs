#!/usr/bin/env python3
"""PNG/GIF raster stress sweep: scale one raster feature at a time, d2-rs vs Go.

usage: raster-stress.py <d2-rs> [--go GO] [--sizes 10,40,160] [--repeat 3] [--only fam,...]

Each family generates a deterministic D2 board whose size knob n grows the
feature under test. Every row records median CPU (user+sys), wall and peak
RSS per implementation from interleaved runs, plus whether the outputs are
byte-identical. A row is flagged when d2-rs is over Go in CPU or RSS, or
when its CPU ratio grows with n.
"""
import argparse, hashlib, os, shutil, statistics, subprocess, sys, tempfile, time

import references

def chain(n, attrs="", direction="down"):
    lines = [f"direction: {direction}"]
    for i in range(n):
        lines.append(f"n{i}: node {i}")
        if attrs:
            lines.append(f"n{i}.style: {{{attrs}}}")
    lines += [f"n{i} -> n{i+1}" for i in range(n - 1)]
    return "\n".join(lines)

def grid(n, attrs="", cols=None):
    cols = cols or max(2, int(n ** 0.5))
    lines = [f"grid-columns: {cols}"]
    for i in range(n):
        lines.append(f"c{i}: cell {i}")
        if attrs:
            lines.append(f"c{i}.style: {{{attrs}}}")
    return "\n".join(lines)

def fanout(n, edge_attrs=""):
    lines = ["hub"]
    for i in range(n):
        lines.append(f"hub -> leaf{i}: edge {i}")
        if edge_attrs:
            lines.append(f"(hub -> leaf{i})[0].style: {{{edge_attrs}}}")
    return "\n".join(lines)

def nested(n):
    out, depth = [], max(1, n // 4)
    for d in range(depth):
        out.append("  " * d + f"l{d}: {{")
        out.append("  " * (d + 1) + f"style: {{fill: \"#{(40 + 5 * d) % 255:02x}88cc\"; opacity: 0.8}}")
        out.append("  " * (d + 1) + f"x{d}a -> x{d}b")
    out += ["  " * d + "}" for d in reversed(range(depth))]
    return "\n".join(out)

def markdown(n):
    body = "\n".join(f"- item **{i}** with `code` and _emphasis_" for i in range(n))
    return f"doc: |md\n  # Heading\n{chr(10).join('  ' + l for l in body.splitlines())}\n|\nother -> doc"

def code(n):
    body = "\n".join(f"  func f{i}(x int) int {{ return x * {i} }}" for i in range(n))
    return f"src: |go\n{body}\n|\nsrc -> out"

def sql(n):
    cols = "\n".join(f"  c{i}: varchar(64)" for i in range(n))
    return f"t1: {{\n  shape: sql_table\n  id: int {{constraint: primary_key}}\n{cols}\n}}\nt2: {{\n  shape: sql_table\n  id: int {{constraint: primary_key}}\n  t1_id: int {{constraint: foreign_key}}\n}}\nt2.t1_id -> t1.id"

def emoji(n):
    faces = "😀😃😄😁😆🙂🙃😉😊😇🥰😍🤩😘😗☺😚😙🥲😋"
    return grid(n).replace("cell", "") + "\n" + "\n".join(f"c{i}: {faces[i % len(faces)]} cell {i} ✨" for i in range(n))

def steps(n):
    out = ["a -> b", "steps: {"]
    for i in range(n):
        out.append(f"  s{i}: {{\n    a{i} -> b{i}: step {i}\n    a{i}.style.fill: \"#{(i * 37) % 255:02x}aa66\"\n  }}")
    out.append("}")
    return "\n".join(out)

FAMILIES = {
    # name: (generator, extra cli args, output extension)
    "plain-chain": (lambda n: chain(n), [], "png"),
    "tall-chain": (lambda n: chain(n * 4), [], "png"),
    "wide-chain": (lambda n: chain(n * 4, direction="right"), [], "png"),
    "shadow-grid": (lambda n: grid(n, "shadow: true"), [], "png"),
    "shadow-chain": (lambda n: chain(n, "shadow: true"), [], "png"),
    "pattern-dots": (lambda n: grid(n, "fill-pattern: dots"), [], "png"),
    "pattern-lines": (lambda n: grid(n, "fill-pattern: lines"), [], "png"),
    "pattern-grain": (lambda n: grid(n, "fill-pattern: grain"), [], "png"),
    "pattern-paper": (lambda n: grid(n, "fill-pattern: paper"), [], "png"),
    "gradient": (lambda n: grid(n, 'fill: "linear-gradient(#f69d3c, #3f87a6)"'), [], "png"),
    "opacity": (lambda n: grid(n, "opacity: 0.4"), [], "png"),
    "3d-multiple": (lambda n: grid(n, "3d: true; multiple: true"), [], "png"),
    "double-border": (lambda n: grid(n, "double-border: true; border-radius: 8"), [], "png"),
    "sketch-chain": (lambda n: chain(n), ["--sketch"], "png"),
    "sketch-shadow": (lambda n: grid(n, "shadow: true"), ["--sketch"], "png"),
    "dashed-fanout": (lambda n: fanout(n, "stroke-dash: 3; stroke-width: 3"), [], "png"),
    "labeled-fanout": (lambda n: fanout(n), [], "png"),
    "nested-fill": (lambda n: nested(n), [], "png"),
    "markdown": (lambda n: markdown(n), [], "png"),
    "code-block": (lambda n: code(n), [], "png"),
    "sql-columns": (lambda n: sql(n), [], "png"),
    "emoji": (lambda n: emoji(n), [], "png"),
    "dark-theme": (lambda n: grid(n, "shadow: true"), ["--theme=200"], "png"),
    "scale-up": (lambda n: grid(40), [f"--scale={0}"], "png"),  # n -> scale below
    "gif-steps": (lambda n: steps(max(2, n // 4)), ["--animate-interval=125"], "gif"),
    # Audit suspects: per-frame pipeline, icon decode, font fallback, tall
    # shadow geometry, PDF link pass.
    "animated-edges": (lambda n: fanout(max(2, n // 4), "animated: true"), [], "gif"),
    "png-icons": (lambda n: "\n".join(["grid-columns: %d" % max(2, int(n ** 0.5))] + [f"i{k}: {{shape: image; icon: ./icon.png}}" for k in range(n)]), [], "png"),
    "cjk-labels": (lambda n: "\n".join([f"c{i}: 漢字かなカナ한글 {i} — Ελληνικά ☃" for i in range(n)] + [f"c{i} -> c{i+1}" for i in range(n - 1)]), [], "png"),
    "tall-shadow-box": (lambda n: "box: {\n  style.shadow: true\n" + "\n".join(f"  k{i} -> k{i+1}" for i in range(n)) + "\n}", [], "png"),
    "pdf-links": (lambda n: grid(n) + "\n" + "\n".join(f"c{i}.link: https://example.com/{i}\nc{i}.tooltip: tip {i}" for i in range(n)), [], "pdf"),
}

def icon_png():
    import struct, zlib
    w = h = 32
    raw = b"".join(b"\0" + b"".join(bytes((x * 8 % 256, y * 8 % 256, 128, 255)) for x in range(w)) for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")

def run(binary, src_dir, args, ext, timeout):
    out = os.path.join(src_dir, f"output.{ext}")
    if os.path.exists(out):
        os.remove(out)
    start = time.monotonic()
    proc = subprocess.Popen([binary, "--layout=dagre", *args, "input.d2", f"output.{ext}"], cwd=src_dir,
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env={**os.environ, "SOURCE_DATE_EPOCH": "946782245"})
    deadline = start + timeout
    while True:
        pid, status, ru = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            break
        if time.monotonic() > deadline:
            proc.kill(); os.wait4(proc.pid, 0)
            return None
        time.sleep(0.005)
    wall = time.monotonic() - start
    ok = os.waitstatus_to_exitcode(status) == 0 and os.path.exists(out)
    digest = hashlib.sha256(open(out, "rb").read()).hexdigest() if ok else None
    return ok, ru.ru_utime + ru.ru_stime, wall, ru.ru_maxrss / 1024.0, digest

def main():
    p = argparse.ArgumentParser()
    p.add_argument("rust"); p.add_argument("--go", default=str(references.GO_ORACLE))
    p.add_argument("--sizes", default="10,40,160"); p.add_argument("--repeat", type=int, default=3)
    p.add_argument("--only", default=""); p.add_argument("--timeout", type=float, default=300)
    a = p.parse_args()
    sizes = [int(s) for s in a.sizes.split(",")]
    fams = [f for f in FAMILIES if not a.only or f in a.only.split(",")]
    flagged = []
    for fam in fams:
        gen, extra, ext = FAMILIES[fam]
        ratios = []
        for n in sizes:
            args = [x.format(max(1, n // 10)) if "{0}" in x else x for x in extra]
            d = tempfile.mkdtemp(prefix="pngstress-")
            open(os.path.join(d, "input.d2"), "w").write(gen(n))
            open(os.path.join(d, "icon.png"), "wb").write(icon_png())
            samples = {"go": [], "rust": []}
            for _ in range(a.repeat):
                for impl, binary in (("go", a.go), ("rust", a.rust)):
                    samples[impl].append(run(binary, d, args, ext, a.timeout))
            shutil.rmtree(d, ignore_errors=True)
            g, r = samples["go"], samples["rust"]
            if any(x is None for x in r) and not any(x is None for x in g):
                print(f"{fam:15} n={n:4} RUST-TIMEOUT"); flagged.append((fam, n, "rust timeout")); continue
            if any(x is None for x in g + r):
                print(f"{fam:15} n={n:4} timeout (both)"); continue
            if not all(x[0] for x in g):
                print(f"{fam:15} n={n:4} go-failed rust-ok={all(x[0] for x in r)}"); continue
            if not all(x[0] for x in r):
                print(f"{fam:15} n={n:4} RUST-FAILED (Go succeeded)"); flagged.append((fam, n, "rust failed")); continue
            med = lambda xs, i: statistics.median(x[i] for x in xs)
            cpu = med(r, 1) / max(med(g, 1), 1e-6); rss = med(r, 3) / max(med(g, 3), 1e-6)
            same = len({x[4] for x in g + r}) == 1
            ratios.append((n, cpu))
            mark = ""
            if cpu > 1.0 or rss > 1.0: mark += " OVER-GO"
            if not same and ext != "pdf": mark += " MISMATCH"
            print(f"{fam:15} n={n:4} cpu {cpu:5.2f}x  rss {rss:5.2f}x  go {med(g,1):7.3f}s {med(g,3):7.1f}MiB  rust {med(r,1):7.3f}s {med(r,3):7.1f}MiB  same={same}{mark}", flush=True)
            if mark: flagged.append((fam, n, mark.strip()))
        if len(ratios) >= 2 and ratios[-1][1] > ratios[0][1] * 1.5 and ratios[-1][1] > 0.6:
            print(f"{fam:15} GROWING cpu ratio {ratios[0][1]:.2f} -> {ratios[-1][1]:.2f}"); flagged.append((fam, ratios[-1][0], "growing"))
    print("FLAGGED:", *(f"{f}@{n}:{why}" for f, n, why in flagged) or ["none"])

if __name__ == "__main__":
    main()
