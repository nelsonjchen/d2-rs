#!/usr/bin/env python3
"""Bounded, deterministic CLI differential against the pinned Go D2.

Inputs are upstream .d2 files and raw `script:` literals in Go tests. Each
selected input is also subject to one seeded mutation. The language matrix
provides its own isolated import tree; other inputs do not copy external assets.
"""
import struct
import zlib
import datetime
import io
import itertools
import platform
import zipfile
import argparse
import collections
import concurrent.futures
import difflib
import hashlib
import sys
import xml.etree.ElementTree as ET
import importlib.util
import json
import os
from pathlib import Path
import random
import re
import subprocess
import tempfile
import threading
import time
import shutil

import references

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
DURATION = re.compile(r" in [0-9.]+(?:ns|µs|us|ms|s|m[0-9.]+s|h[0-9hms.]+)$", re.M)
SCRIPT = re.compile(r"\bscript\s*:\s*`([^`]*)`", re.S)
KEY = re.compile(r"(?m)^\s*([a-zA-Z_][\w-]*)\s*(?::|$)")
BUDGET = 1024 * 1024 * 1024
FILE_LIMIT = 128 * 1024 * 1024
budget_lock = threading.Lock()
budget_used = 0
budget_exhausted = threading.Event()


def inputs():
    # Minimized differential witnesses remain in the corpus on every run.
    yield "fixed:reserved-edge", "fill -> a\n"
    for path in sorted(UPSTREAM.rglob("*.d2")):
        yield str(path.relative_to(UPSTREAM)), path.read_text(errors="replace")
    for path in sorted(UPSTREAM.rglob("*_test.go")):
        for index, match in enumerate(SCRIPT.finditer(path.read_text(errors="replace"))):
            yield f"{path.relative_to(UPSTREAM)}:script:{index}", match.group(1)


def _mutate_once(source, rng, legacy=False):
    lines = source.splitlines(keepends=True)
    kind = rng.randrange(6 if legacy else 14)
    if lines and kind == 0:
        del lines[rng.randrange(len(lines))]
    elif lines and kind == 1:
        i = rng.randrange(len(lines)); lines.insert(i, lines[i])
    elif kind == 2:
        source = re.sub(r"\b(rectangle|circle|oval|diamond)\b", lambda m: {"rectangle": "circle", "circle": "diamond", "oval": "rectangle", "diamond": "oval"}[m[0]], source)
        return source, "shapes"
    elif kind == 3:
        lines.append('\n differential_label: "雪 λ 🌿"\n')
    elif kind == 4:
        return "differential_container: {\n" + source + "\n}\n", "nest"
    elif kind <= 5:
        keys = [key for key in KEY.findall(source) if key.lower() not in RESERVED]
        if len(keys) >= 2:
            a, b = rng.sample(keys, 2); lines.append(f"\n{a} -> {b}\n")
        else:
            lines.append("\ndifferential_a -> differential_b\n")
        return "".join(lines), "edge"
    else:
        additions = (
            ('classes: {diff_style: {style.fill: "red"}}\ndifferential_class: {class: diff_style}\n', "class"),
            ('*: {style.stroke-width: 2}\n', "glob"),
            ('differential_import: @missing-local-file\n', "import"),
            ('vars: {differential_color: red}\ndifferential_var: {style.fill: ${differential_color}}\n', "vars"),
            ('layers: {differential_layer: {a -> b}}\n', "layer"),
            ('differential_near: {near: top-left}\n', "near"),
            ('differential_grid: {grid-columns: 2; a; b; c}\n', "grid"),
            ('differential_markdown: |md\n# Snow 雪\n|\ndifferential_icon: {icon: ./missing.svg}\n', "markdown-icon"),
        )
        addition, name = additions[kind - 6]
        return source + "\n" + addition, name
    return "".join(lines), ("drop", "duplicate", "shapes", "unicode", "nest", "edge")[kind]


RESERVED = set("shape label icon near style classes class vars layers scenarios steps direction grid-rows grid-columns grid-gap vertical-gap horizontal-gap width height top left tooltip link constraint source-arrowhead target-arrowhead fill stroke opacity fill-pattern stroke-dash border-radius font font-size font-color stroke-width bold italic underline text-transform shadow multiple double-border 3d animated filled".split())


def mutate(source, rng, legacy=False):
    for _ in range(64):
        result, kind = _mutate_once(source, rng, legacy)
        if result != source:
            return result, kind
    # A bounded fallback also gives empty/shape-free sources a real mutation.
    return source + '\ndifferential_label: "雪 λ 🌿"\n', "unicode"


def normalize(data):
    # The same policy is implemented in the WASM harness. Preserve XML tags
    # and terminal newlines: those are public options covered by this matrix.
    return re.sub(rb'(data-d2-version|d2Version)="[^"]*"', rb'\1=""', data)


def environment(epoch=None):
    result = {"PATH": os.defpath, "HOME": str(ROOT),
              "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TZ": "UTC", "NO_COLOR": "1", "GOMAXPROCS": "2"}
    if epoch is not None:
        result["SOURCE_DATE_EPOCH"] = str(epoch)
    return result


def format_of(options):
    return next((o.split("=", 1)[1] for o in options if o.startswith("--format=")),
                "png" if "--png" in options else "svg")


normalize_svg = normalize


def validate_output(path, data, suffix):
    if not data: raise ValueError("empty artifact")
    if suffix == ".svg":
        if ET.fromstring(data).tag.rsplit("}", 1)[-1] != "svg": raise ValueError("not SVG")
    elif suffix == ".png":
        # Native exports include both RGB and RGBA. The raster scoreboard's
        # RGB-only decoder cannot validate transparent isometric exports.
        if not data.startswith(b"\x89PNG\r\n\x1a\n"): raise ValueError("not PNG")
        offset, compressed, header, ended = 8, bytearray(), None, False
        while offset + 12 <= len(data):
            size = struct.unpack_from(">I", data, offset)[0]
            kind = data[offset + 4:offset + 8]
            if offset + size + 12 > len(data): raise ValueError("truncated PNG")
            payload = data[offset + 8:offset + 8 + size]
            if zlib.crc32(kind + payload) != struct.unpack_from(">I", data, offset + size + 8)[0]: raise ValueError("PNG CRC")
            if header is None and kind != b"IHDR": raise ValueError("missing PNG header")
            if kind == b"IHDR":
                if header is not None or size != 13: raise ValueError("invalid PNG header")
                header = struct.unpack(">IIBBBBB", payload)
            if kind == b"IDAT": compressed.extend(payload)
            offset += size + 12
            if kind == b"IEND": ended = size == 0 and offset == len(data); break
        if not ended or header is None or not compressed: raise ValueError("incomplete PNG")
        width, height, depth, color, compression, filtering, interlace = header
        channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}.get(color)
        depths = {0: (1, 2, 4, 8, 16), 2: (8, 16), 3: (1, 2, 4, 8), 4: (8, 16), 6: (8, 16)}
        if not width or not height or depth not in depths.get(color, ()) or compression or filtering or interlace: raise ValueError("invalid native PNG header")
        expected = height * (1 + (width * channels * depth + 7) // 8)
        decoder, count, pending = zlib.decompressobj(), 0, bytes(compressed)
        while pending:
            count += len(decoder.decompress(pending, 1024 * 1024))
            if count > expected: raise ValueError("PNG pixel length")
            pending = decoder.unconsumed_tail
        if not decoder.eof or decoder.unused_data or count != expected: raise ValueError("incomplete PNG pixels")
    elif suffix == ".pdf":
        if not data.startswith(b"%PDF-") or not data.rstrip().endswith(b"%%EOF"): raise ValueError("incomplete PDF")
    elif suffix == ".pptx":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            if archive.testzip() is not None or "[Content_Types].xml" not in archive.namelist(): raise ValueError("invalid PPTX")
            for name in archive.namelist():
                if name.endswith((".xml", ".rels")): ET.fromstring(archive.read(name))
    elif suffix == ".gif":
        if not data.startswith((b"GIF87a", b"GIF89a")) or not data.endswith(b";") or len(data) < 14: raise ValueError("incomplete GIF")
    elif suffix == ".json": json.loads(data)
    elif suffix == ".txt": data.decode("utf-8")


def run(binary, source, options, timeout, directory, epoch=None, fixtures=None):
    global budget_used
    for name, content in (fixtures or {}).items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    (directory / "input.d2").write_text(source)
    fmt = format_of(options)
    command = next((o for o in options if o in ("fmt", "fmt-check", "validate")), None)
    if command:
        args = ["fmt", "--check", "input.d2"] if command == "fmt-check" else [command, "input.d2"]
    else:
        flags = [o for o in options if o not in ("--png", "--stdout") and not o.startswith("--format=")]
        args = ["--bundle=false", "--tala-seeds=1", *flags, "input.d2"]
        args += [f"--stdout-format={fmt}", "-"] if "--stdout" in options else [f"output.{fmt}"]
    logs = [directory / name for name in ("stdout.log", "stderr.log")]
    accounted = 0
    try:
        with logs[0].open("wb") as stdout, logs[1].open("wb") as stderr:
            process = subprocess.Popen([str(binary), *args], cwd=directory, stdout=stdout,
                                       stderr=stderr, env=environment(epoch))
    except OSError as error:
        return {"exit": "process-failure", "error": str(error)}
    deadline = time.monotonic() + timeout
    failure = None
    while True:
        finished = process.poll() is not None
        paths = [p for p in directory.rglob("*") if p.is_file()]
        sizes = []
        for path in paths:
            try: sizes.append(path.stat().st_size)
            except FileNotFoundError: pass  # Atomic publishers rename staging files.
        current = sum(sizes)
        with budget_lock:
            budget_used += max(0, current - accounted)
            accounted = current
            if budget_used > BUDGET or max(sizes, default=0) > FILE_LIMIT or shutil.disk_usage(directory).free < 30 * 1024**3:
                budget_exhausted.set()
        if budget_exhausted.is_set(): failure = "budget-exceeded"
        elif not finished and time.monotonic() > deadline: failure = "timeout"
        if failure:
            process.kill(); process.wait()
            return {"exit": failure}
        if finished: break
        time.sleep(0.02)
    def log_bytes(path):
        data = path.read_bytes()
        if path == logs[0] and "--stdout" in options:
            return normalize(data) if fmt == "svg" else data
        data = data.replace(str(directory).encode(), b"DIR").replace(str(binary).encode(), b"PROG")
        if path == logs[1]:
            data = DURATION.sub(" in T", data.decode(errors="backslashreplace")).encode()
        return data
    files, text_files, invalid = {}, {}, []
    for path in sorted(directory.rglob("*")):
        if not path.is_file() or path in logs or path.relative_to(directory).as_posix() in (fixtures or {}) or path.name == "input.d2" and not command:
            continue
        data = path.read_bytes()
        if process.returncode == 0 and path.name != "input.d2":
            try: validate_output(path, data, path.suffix)
            except (ValueError, OSError, ET.ParseError, zipfile.BadZipFile, zlib.error) as error: invalid.append(f"{path.relative_to(directory)}: {error}")
        if path.suffix == ".svg":
            data = normalize(data)
            text_files[path.relative_to(directory).as_posix()] = data.decode(errors="replace")
        files[path.relative_to(directory).as_posix()] = hashlib.sha256(data).hexdigest()
    output = log_bytes(logs[0])
    if process.returncode == 0 and "--stdout" in options and output:
        try: validate_output(logs[0], output, "." + fmt)
        except (ValueError, OSError, ET.ParseError, zipfile.BadZipFile, zlib.error) as error: invalid.append(f"stdout: {error}")
    if process.returncode == 0 and not command and fmt in ("png", "pdf", "pptx", "gif"):
        if not output and "--stdout" in options or not files and "--stdout" not in options: invalid.append("missing export output")
    return {"exit": process.returncode, "stdout": output, "stderr": log_bytes(logs[1]),
            "files": files, "text_files": text_files, "invalid_output": invalid}


def compare(job, args):
    name, source, mutation, options, *extra = job
    fixtures = extra[0] if extra else {}
    row = {"input": name, "mutation": mutation, "options": options,
           "source_sha256": hashlib.sha256(source.encode()).hexdigest()}
    if budget_exhausted.is_set(): return dict(row, outcome="budget-exceeded")
    epoch = getattr(args, "epoch", None)
    render_timeout = getattr(args, "render_timeout", max(getattr(args, "png_timeout", args.timeout), getattr(args, "tala_timeout", args.timeout)))
    timeout = max(args.timeout, render_timeout) if (format_of(options) in ("png", "pdf", "pptx", "gif") or "--layout=tala" in options or "--isometric" in options) else args.timeout
    if getattr(args, "baseline_matrix", False):
        timeout = args.png_timeout if "--png" in options else args.tala_timeout if "--layout=tala" in options else args.timeout
    with tempfile.TemporaryDirectory(prefix="d2-diff-") as temp:
        # Identical paths, clean filesystem for each process, including fmt.
        directory = Path(temp) / "run"
        directory.mkdir()
        go = run(args.go, source, options, timeout, directory, epoch, fixtures)
        shutil.rmtree(directory); directory.mkdir()
        rust = run(args.rust, source, options, timeout, directory, epoch, fixtures)
    row["go"], row["rust"] = ({key: value.decode(errors="backslashreplace") if isinstance(value, bytes) else value
                                  for key, value in result.items() if key != "text_files"} for result in (go, rust))
    if "budget-exceeded" in (go["exit"], rust["exit"]):
        row["outcome"] = "budget-exceeded"
    elif go["exit"] == "timeout" or isinstance(go["exit"], int) and (go["exit"] < 0 or b"panic:" in go.get("stderr", b"")):
        row.update(outcome="go-crash-or-timeout", go_exit=go["exit"], rust_exit=rust["exit"])
    elif "process-failure" in (go["exit"], rust["exit"]):
        row["outcome"] = "process-failure"
    elif go.get("invalid_output") or rust.get("invalid_output"):
        row.update(outcome="invalid-output")
    elif rust["exit"] == "timeout":
        row.update(outcome="timeout", cluster="rust timeout")
    elif go == rust:
        row["outcome"] = "match"
    else:
        row["outcome"] = "mismatch"
        if go["exit"] != rust["exit"]:
            row["cluster"] = f"exit {go['exit']} vs {rust['exit']}"
        elif go.get("stderr") != rust.get("stderr"):
            diff = next((line for line in difflib.ndiff(go.get("stderr", b"").decode(errors="replace").splitlines(), rust.get("stderr", b"").decode(errors="replace").splitlines()) if line.startswith(("- ", "+ "))), "")
            row["cluster"] = "stderr: " + re.sub(r"\d+", "#", diff)[:200]
        elif go.get("stdout") != rust.get("stdout"):
            row["cluster"] = "stdout bytes: " + format_of(options) if "--stdout" in options else "stdout"
        else:
            g, r = go.get("files", {}), rust.get("files", {})
            row["cluster"] = "output inventory" if g.keys() != r.keys() else "file bytes: " + ("fmt" if "fmt" in options else format_of(options))
        if row["cluster"] == "file bytes: svg":
            for path, left in go.get("text_files", {}).items():
                right = rust.get("text_files", {}).get(path, "")
                if left == right: continue
                a, b = left.splitlines(), right.splitlines()
                i = next((i for i, pair in enumerate(zip(a, b)) if pair[0] != pair[1]), min(len(a), len(b)))
                row["first_difference"] = {"file": path, "line": i + 1, "go": a[i][:500] if i < len(a) else "", "rust": b[i][:500] if i < len(b) else ""}
                tag = re.search(r"<([a-zA-Z][\w:-]*)", a[i] if i < len(a) else "")
                row["cluster"] = "svg element: " + (tag[1] if tag else "text")
                break
        row.update(source=source, fixtures=fixtures, go_files=go.get("files"), rust_files=rust.get("files"))
        for label, result in (("go", go), ("rust", rust)):
            for stream in ("stdout", "stderr"):
                row[label + "_" + stream] = result.get(stream, b"").decode(errors="backslashreplace")[-2000:]
    return row


def widened_options(options, index, source, rng):
    options = list(options)
    # Small seeded fractions keep expensive exports within the run budget.
    draw = rng.randrange(100)
    fmt = "svg"
    if draw < 12:
        fmt = ("png", "pdf", "pptx", "gif", "txt")[draw % 5]
        options = [o for o in options if o != "--png"] + ["--format=" + fmt]
    if fmt == "txt": options.append("--ascii-mode=" + rng.choice(("standard", "extended")))
    if fmt == "gif" and re.search(r"\b(layers|steps|scenarios)\s*:", source): options.append("--animate-interval=100")
    if rng.randrange(20) == 0: options.append("--stdout")
    for probability, flag in ((12, "--dark-theme=200"), (10, "--center"), (12, "--scale=0.5"),
                              (16, "--force-appendix"), (12, "--no-xml-tag"),
                              (12, "--omit-version"), (12, "--salt=differential")):
        if rng.randrange(probability) == 0: options.append(flag)
    if rng.randrange(4) == 0:
        options = [o for o in options if not o.startswith("--theme=")]
        options.append(f"--theme={rng.choice((1, 3, 4, 5, 6, 100, 101, 102, 103, 104, 105, 300, 301))}")
    # This board is created by the layer mutation; don't guess target paths
    # from raw syntax or send a target on unrelated inputs.
    if "layers: {differential_layer: {a -> b}}" in source and rng.randrange(2) == 0:
        options.append("--target=layers.differential_layer")
    if rng.randrange(100) == 0:
        options = [o for o in options if o != "--png" and not o.startswith("--scale=")]
        options += ["--isometric", "--scale=0.2"]
    return options


def fixed_jobs(wide=True):
    jobs = []
    jobs.append(("fixed:constant-near-excludes-relative-near",
                 'title: |md\n  # Service-Cluster Provisioning ("Outside view")\n'
                 '| {near: top-center}\ndiff_anchor\ndiff_near: {near: diff_anchor}\n',
                 "original", ["--layout=tala", "--theme=1", "--dark-theme=201",
                              "--pad=1", "--scale=0.75", "--sketch"]))
    jobs.append(("fixed:dagre-compound-fractional-rank-frame",
                 "direction: left\nn: {a -> b}\nx: hello\n", "original",
                 ["--layout=dagre", "--theme=7", "--dark-theme=200", "--pad=20",
                  "--scale=1", "--center"]))
    jobs.append(("fixed:dagre-nested-sequence-fractional-width",
                 "a: {\nb: {\nc: {\nd: {\nshape: sequence_diagram\n"
                 "a -> a: a self edge here\n}\n}\n}\n}\n", "original",
                 ["--layout=dagre", "--theme=102", "--dark-theme=200", "--pad=20",
                  "--scale=1", "--center"]))
    if wide:
        jobs.append(("fixed:centered-latex-viewport-pdf",
                     "diff_math: |latex\n\\frac{1}{2} + \\sqrt{x^2} = \\alpha\n|\n",
                     "original", ["--layout=dagre", "--theme=8", "--dark-theme=201",
                                  "--pad=100", "--scale=2", "--sketch", "--center",
                                  "--format=pdf"]))
    jobs.append(("fixed:tala-nested-hierarchy-placement",
                 "diff_depth_0: {\ni.m -> f\nr.gg -> r.s.t\nr.aa -> r.s.t\n"
                 "r.s.w -> i.m\n}\n", "original",
                 ["--layout=tala", "--theme=0", "--dark-theme=200", "--pad=0",
                  "--scale=0.25", "--center"]))
    # Native scene masks retain the integer-frame viewBox mapping in f64.
    mask_workflows = r"""
build_workflow: lambda-build.yaml {
push: Push to main branch {style.font-size: 25}
GHA: GitHub Actions {style.font-size: 25}
S3.style.font-size: 25
Terraform.style.font-size: 25
AWS.style.font-size: 25
push -> GHA: Triggers {style.font-size: 20}
GHA -> S3: Builds zip and pushes it {style.font-size: 20}
S3 <-> Terraform: Pulls zip to deploy {style.font-size: 20}
Terraform -> AWS: Changes live lambdas {style.font-size: 20}
}
deploy_workflow: lambda-deploy.yaml {
manual: Manual Trigger {style.font-size: 25}
GHA: GitHub Actions {style.font-size: 25}
AWS.style.font-size: 25
Manual -> GHA: Launches {style.font-size: 20}
GHA -> AWS: Builds zip\npushes them to S3.\n\nDeploys lambdas\nusing Terraform {style.font-size: 20}
}
"""
    jobs.append(("fixed:sketch-label-mask-canvas-gif", mask_workflows, "original",
                 ["--layout=elk", "--theme=101", "--dark-theme=201", "--pad=1",
                  "--scale=0.75", "--sketch", "--center", "--animate-interval=125", "--format=gif"]))
    jobs.append(("fixed:sketch-class-raster", "a: {shape: class}\n", "original",
                 ["--layout=tala", "--theme=200", "--sketch", "--pad=20", "--png"]))
    jobs.append(("fixed:dagre-compound-modifier", "a: {x: {style.multiple: true}; y}\n", "original",
                 ["--layout=dagre", "--theme=0", "--pad=20"]))
    jobs.append(("fixed:sequence-constant-near", "shape: sequence_diagram\na -> b\nn: {near: top-left}\n", "original",
                 ["--layout=dagre", "--theme=0", "--pad=0"]))
    sequence_hierarchy = (
        "web\nfeature: {shape: step}\nscoring: {shape: step}\nfeature -> scoring\n"
        "prediction service\nhome scorer -> memcache: \"\"\nhome ranker -> home scorer\n"
        "home ranker -> container2.crmixer: Candidate Fetch\ncontainer2: {crmixer}\n"
        "feature -> memcache\n"
    )
    for suffix, text in (("", sequence_hierarchy), ("-nested", "box: {\n" + sequence_hierarchy + "}\n")):
        jobs.append(("fixed:tala-sequence-hierarchy" + suffix, text, "original",
                     ["--layout=tala", "--theme=0", "--pad=20"]))
    jobs.append(("fixed:tala-alignment-scoring-scope",
                 "feature: Feature {shape: step}\nscoring: Scoring {shape: step}\n"
                 "feature -> scoring\nprediction service: Prediction Service\n"
                 "feature -> manhattan\nfeature -> memcache\n", "original",
                 ["--layout=tala", "--theme=0", "--pad=0"]))
    jobs.append(("fixed:tala-aggregate-hub-retry",
                 "feature: Feature {shape: step}\nscoring: Scoring {shape: step}\n"
                 "fetch -> feature\nfeature -> scoring\nprediction service: Prediction Service\n"
                 "feature -> manhattan\nfeature -> memcache\n", "original",
                 ["--layout=tala", "--theme=0", "--pad=0"]))
    jobs.append(("fixed:tala-sequence-cluster-alignment",
                 "feature: Feature {shape: step}\nscoring: Scoring {shape: step}\n"
                 "fetch -> feature\nfeature -> scoring\n"
                 "prediction service: Prediction Service {\n"
                 "icon: https://cdn-icons-png.flaticon.com/512/6461/6461819.png\n}\n"
                 "scoring -> prediction service\nhome scorer -> memcache: \"\"\n"
                 "home scorer -> etc: Feature Hydration\nfeature -> manhattan\n"
                 "feature -> memcache\nfeature -> etc: Candidate sources\n", "original",
                 ["--layout=tala", "--theme=0", "--sketch", "--pad=0"]))
    jobs.append(("fixed:sequence-grid-extraction",
                 "shape: sequence_diagram\na\ng: {grid-columns: 2; a; b; c}\n", "original",
                 ["--layout=dagre", "--theme=0", "--pad=0"]))
    jobs.append(("fixed:tala-tree-midpoint-center",
                 "main: {x -> y; y <- z}\nroot: {x -> y; y <- z}\n", "original",
                 ["--layout=tala", "--theme=0", "--sketch", "--pad=20"]))
    jobs.append(("fixed:elk-compound-normalized-hull",
                 "p: {x.start -> a; x.start -> b; x.start -> c; "
                 "a -> y.end; b -> y.end; c -> y.end}\n", "original",
                 ["--layout=elk", "--theme=200", "--pad=0"]))
    jobs.append(("fixed:elk-compound-centered-hierarchy",
                 "p: differential_container {x.start -> a; x.start -> b; x.start -> c; "
                 "a -> y.end; b -> y.end; c -> y.end}\n", "original",
                 ["--layout=elk", "--theme=200", "--sketch", "--pad=0"]))
    jobs.append(("fixed:tala-outside-label-fit",
                 "containers: {circle container: {diamond; hexagon}; "
                 "hexagon container: {shape: hexagon; oval}}\ncloud\nusers\n", "original",
                 ["--layout=tala", "--theme=0", "--sketch", "--pad=0"]))
    jobs.append(("fixed:dagre-hexagon-modifier-integer-offset",
                 "p: {h: {shape: hexagon; style.3d: true}}\n", "original",
                 ["--layout=dagre", "--theme=200", "--pad=20"]))
    jobs.append(("fixed:multiboard-layout-before-render",
                 "shape: sequence_diagram\ngroup.nested: {a -> b}\n"
                 "layers: {valid: {a -> b}}\n", "original",
                 ["--layout=tala", "--theme=0", "--pad=0"]))
    jobs.append(("fixed:multiboard-child-layout-before-render",
                 "layers: {valid: {a -> b}; invalid: {shape: sequence_diagram; "
                 "group.nested: {a -> b}}}\n", "original",
                 ["--layout=dagre", "--theme=0", "--pad=0"]))
    jobs.append(("fixed:tala-container-tree-mirror",
                 "a -> b\na -> c\na -> d\nd.e: |md\n- Text\n|\n", "original",
                 ["--layout=tala", "--theme=200", "--pad=20"]))
    jobs.append(("fixed:tala-direct-mirror-transaction",
                 "non container: {\n"
                 "  Default.class: icon\n"
                 "  OutsideLeftBottom.class: [icon; OutsideLeftBottom]\n}\n"
                 "container: {\n"
                 "  OutsideRightTop.class: [icon; OutsideRightTop]\n"
                 "  InsideTopLeft.InsideTopLeft\n"
                 "  InsideBottomLeft.InsideBottomLeft\n"
                 "  InsideBottomCenter.InsideBottomCenter\n"
                 "  InsideBottomRight.InsideBottomRight\n}\n"
                 "image: {\n"
                 "  Default.class: [icon; image]\n"
                 "  OutsideTopLeft.class: [icon; image; OutsideTopLeft]\n"
                 "  OutsideTopRight.class: [icon; image; OutsideTopRight]\n"
                 "  InsideBottomRight.class: [icon; image; InsideBottomRight]\n}\n"
                 "non container -> container -> image\n"
                 "classes: {\n"
                 "  icon.icon: https://icons.terrastruct.com/essentials/time.svg\n"
                 "  OutsideRightTop.icon.near: outside-right-top\n}\n", "original",
                 ["--layout=tala", "--theme=200", "--sketch", "--pad=0"]))
    jobs.append(("fixed:tala-grid-placeholder-label-obstacles",
                 "vars: {grid-container: {label: aaaaaaaaaaaaaaa aaaaaaaaaaaaaaa; "
                 "grid-rows: 2; grid-columns: 2; a; b; c; d}}\n"
                 "circle: {shape: circle; ...${grid-container}}\n"
                 "oval: {shape: oval; ...${grid-container}}\n"
                 "diamond: {shape: diamond; ...${grid-container}}\n"
                 "square: {shape: square; ...${grid-container}}\n"
                 "rect: {...${grid-container}}\n"
                 "cloud: {shape: cloud; ...${grid-container}}\n"
                 "differential_label: Text Text\n", "original",
                 ["--layout=tala", "--theme=200", "--pad=20"]))
    jobs.append(("fixed:dagre-near-language-metadata",
                 "description: |md\n# Title\n|\n"
                 "image: {shape: image; icon: https://example.com/image.svg; "
                 "width: 100; height: 100; near: center-left}\n", "original",
                 ["--layout=dagre", "--theme=0", "--pad=0"]))
    # Copy budgets cover the whole compilation even when only a small layer
    # is selected. Include controls below the boundary as well as refusals.
    for count in (24, 28):
        jobs.append((f"fixed:class-copy-budget-{count}",
                     'classes.c: {tooltip: "' + "x" * 1024 + '"}\n'
                     + "".join(f"n{i}.class: c\n" for i in range(count))
                     + "layers: {detail: {a}}\n", "original",
                     ["--layout=elk", "--target=layers.detail", "--pad=0"]))
    for length in (16000, 16384):
        jobs.append((f"fixed:board-copy-budget-{length}",
                     'a: "' + "x" * length + '"\nlayers: {detail: {b}}\n', "original",
                     ["--layout=dagre", "--target=layers.detail", "--pad=0"]))
    if not wide: return jobs
    jobs.append(("fixed:stretched-circle-inner-label-pdf",
                 "g: {grid-rows: 2; a: {shape: circle}; b: {width: 500}; c}\n",
                 "original", ["--layout=dagre", "--theme=201", "--dark-theme=201",
                              "--pad=100", "--scale=2", "--sketch", "--center", "--format=pdf"]))
    jobs.append(("fixed:single-board-svg-animation", "a -> b\n", "original",
                 ["--layout=dagre", "--theme=101", "--dark-theme=201", "--sketch",
                  "--pad=1", "--scale=0.75", "--animate-interval=125"]))
    jobs.append(("fixed:c4-sequence-lifelines", "shape: sequence_diagram\na -> b\n", "original",
                 ["--layout=dagre", "--theme=303", "--sketch", "--pad=100"]))
    jobs.append(("fixed:unselected-dagre-ancestor-edge",
                 "container -> container.second: c->2\n"
                 "layers: {differential_layer: {a -> b}; differential_other: {c -> d}}\n",
                 "original", ["--layout=dagre", "--target=layers.differential_layer"]))
    for fmt in ("png", "gif"):
        for shape, members in (("class", ""), ("sql_table", "x: int; ")):
            jobs.append(("fixed:sketch-rounded-separator-" + shape + "-" + fmt,
                         "a: {shape: " + shape + "; " + members + "style.border-radius: 5}\n",
                         "original", ["--layout=dagre", "--theme=101", "--sketch", "--center",
                                      "--pad=1", "--scale=0.75", "--format=" + fmt]))
        jobs.append(("fixed:fractional-sketch-shape-transform-" + fmt,
                     "a -> b\n", "original",
                     ["--layout=elk", "--theme=101", "--sketch", "--center",
                      "--pad=1", "--scale=0.75", "--format=" + fmt]))
        jobs.append(("fixed:fractional-sketch-link-" + fmt,
                     "source -> target: {link: https://example.com/route}\n", "original",
                     ["--layout=dagre", "--theme=101", "--dark-theme=201", "--sketch",
                      "--pad=1", "--scale=0.75", "--format=" + fmt]))
    jobs.append(('fixed:tala-class-with-image-peers',
                 'classes: {\n'
                 '  rare: {\n'
                 '    shape: image\n'
                 '    icon: https://raw.githubusercontent.com/frederic-loui/RARE-web/automated-update/docs/img/rare.svg\n'
                 '    height: 32\n'
                 '  }\n'
                 '}\n'
                 'r1.class: rare\n'
                 'r2.class: rare\n'
                 '\n'
                 'r1 <-> r2: {\n'
                 '  source-arrowhead.label: eth1\n'
                 '  target-arrowhead.label: eth1\n'
                 '}\n'
                 '\n'
                 'diff_class: {shape: class; -count: int; +run(): bool}\n', "original",
                 ['--layout=tala', '--theme=201', '--dark-theme=201', '--pad=100', '--scale=2', '--sketch', '--center']))
    jobs.append(('fixed:dark-sketch-teleport-pdf',
                 'direction: right\n'
                 '\n'
                 'users -- via -- teleport\n'
                 '\n'
                 'teleport -> jita: "all connections audited and logged"\n'
                 'teleport -> infra\n'
                 '\n'
                 'teleport -> identity provider\n'
                 'teleport <- identity provider\n'
                 '\n'
                 'users: "" {\n'
                 '  grid-columns: 1\n'
                 '\n'
                 '  Engineers: {\n'
                 '    shape: circle\n'
                 '    icon: https://icons.terrastruct.com/essentials%2F365-user.svg\n'
                 '  }\n'
                 '  grid-rows: 2\n'
                 '\n'
                 '  inp: |md\n'
                 '    # Identity Native Proxy\n'
                 '  | {\n'
                 '    width: 300\n'
                 '  }\n'
                 '\n'
                 '  Audit Log.icon: https://icons.terrastruct.com/tech%2Flaptop.svg\n'
                 '  Cert Authority.icon: https://icons.terrastruct.com/azure%2FWeb%20Service%20Color%2FApp%20Service%20Certificates.svg\n'
                 '}\n'
                 '\n'
                 'jita: "Just-in-time Access via" {\n'
                 '  grid-rows: 1\n'
                 '\n'
                 '  Slack.icon: https://icons.terrastruct.com/dev%2Fslack.svg\n'
                 '  Mattermost\n'
                 '  Jira\n'
                 '  Pagerduty\n'
                 '  Email.icon: https://icons.terrastruct.com/aws%2F_General%2FAWS-Email_light-bg.svg\n'
                 '}\n'
                 '\n'
                 'infra: Infrastructure {\n'
                 '  grid-rows: 2\n'
                 '\n'
                 '  ssh.icon: https://icons.terrastruct.com/essentials%2F112-server.svg\n'
                 '  Kubernetes.icon: https://icons.terrastruct.com/azure%2F_Companies%2FKubernetes.svg\n'
                 '  My SQL.icon: https://icons.terrastruct.com/dev%2Fmysql.svg\n'
                 '  MongoDB.icon: https://icons.terrastruct.com/dev%2Fmongodb.svg\n'
                 '  PSQL.icon: https://icons.terrastruct.com/dev%2Fpostgresql.svg\n'
                 '  Windows.icon: https://icons.terrastruct.com/dev%2Fwindows.svg\n'
                 '}\n'
                 '\n'
                 'identity provider: Indentity Provider {\n'
                 '  icon: https://icons.terrastruct.com/azure%2FIdentity%20Service%20Color%2FIdentity%20governance.svg\n'
                 '}\n', "original",
                 ['--layout=dagre', '--theme=201', '--dark-theme=201', '--pad=100', '--scale=2', '--sketch', '--center', '--format=pdf']))
    jobs.append(("fixed:gif-frame-pixel-budget", "a: {width: 1500; height: 1500}\n", "original",
                 ["--layout=dagre", "--format=gif", "--animate-interval=125"]))
    for fmt in ("svg", "png", "pdf", "pptx", "gif", "txt"):
        options = ["--layout=dagre", "--format=" + fmt]
        if fmt == "txt": options += ["--ascii-mode=standard"]
        jobs.append(("fixed:format-" + fmt, "a -> b\n", "original", options))
        jobs.append(("fixed:stdout-" + fmt, "a -> b\n", "original", options + ["--stdout"]))
    jobs.append(("fixed:png-sketch-3d", "a: {style.3d: true}\n", "original", ["--layout=elk", "--theme=0", "--sketch", "--pad=20", "--format=png"]))
    jobs.append(("fixed:pdf-link-appendix", "x.link: https://example.com\n", "original", ["--layout=elk", "--theme=200", "--pad=0", "--format=pdf"]))
    jobs.append(("fixed:pdf-scaled-edge-link", "x -> y: {link: https://google.com}\n", "original", ["--layout=dagre", "--theme=4", "--pad=100", "--scale=2", "--sketch", "--center", "--format=pdf"]))
    for fmt in ("pdf", "pptx"):
        jobs.append(("fixed:fractional-page-link-" + fmt, "x -> y: {link: https://google.com}\n", "original", ["--layout=dagre", "--theme=4", "--pad=20", "--scale=0.75", "--format=" + fmt]))
    jobs.append(("fixed:pdf-scaled-markdown-appendix", 'a: |md\n[abc](https://example.com "A & B")\n|\n', "original", ["--layout=dagre", "--scale=2", "--format=pdf"]))
    jobs.append(("fixed:pptx-sketch-3d", "y: {style.3d: true}\n", "original", ["--layout=elk", "--theme=200", "--sketch", "--pad=20", "--format=pptx", "--center"]))
    jobs.append(("fixed:pptx-unavailable-image-paper", 'outer: {a: {shape: image; icon: "http://127.0.0.1:1/missing.png"; width: 140; height: 80}}\n', "original",
                 ["--layout=dagre", "--theme=302", "--pad=20", "--scale=1", "--center", "--format=pptx"]))
    jobs.append(("fixed:animated-gif", "a\nlayers: {x: {b}}\n", "original",
                 ["--layout=dagre", "--format=gif", "--animate-interval=100"]))
    for name, source in (("nil-step", "steps: { a: 1; b: { x } }\n"),
                         ("parser-slice", "a: x*${x}a*\n")):
        jobs.append(("fixed:review-" + name, source, "original", ["--layout=dagre"]))
    for layout in ("dagre", "elk"):
        for fmt in ("png", "gif"):
            jobs.append(("fixed:sketch-border-label-local-mask",
                         "env: Your environment {\n  label.near: border-bottom-center\n  app: Your applicaton\n}\n", "original",
                         ["--layout=" + layout, "--theme=101", "--dark-theme=201", "--pad=1",
                          "--scale=0.75", "--sketch", "--center", "--format=" + fmt]))
    for layout in ("dagre", "elk", "tala"):
        jobs.append(("fixed:childless-grid-ancestor-fit", "a: {b: {c: {grid-columns: 1}}}\n", "original",
                     ["--layout=" + layout, "--theme=100", "--pad=0", "--scale=0.25", "--format=png"]))
    # Minimized post-export witnesses; D2 source only, no captured output.
    jobs.append(("fixed:markdown-fractional-viewport-clip", 'grid-rows: 2\ngrid-columns: 1\ngrid-gap: 50\nupper: " " {\n  grid-gap: 40\n  sidebar: " " {\n    style.fill: "#FFFFFF"\n    style.stroke: "#FFFFFF"\n    grid-rows: 4\n    grid-columns: 1\n    grid-gap: 26\n    compile_mode: "Compilation mode" {\n      grid-columns: 1\n      grid-gap: 12\n      mode: "CompilationMode.STOCK_TORCH_COMPILE" {\n        style.fill: "#1565c0"\n        style.stroke: "#0d47a1"\n        style.font-color: "#ffffff"\n        style.font-size: 14\n        style.bold: true\n        style.border-radius: 8\n        label.near: top-center\n        grid-rows: 1\n        grid-columns: 1\n        grid-gap: 0\n        width: 580\n        mode_d: "torch.compile(model, backend=\'inductor\',\\nfullgraph=True, dynamic=False)" {\n          style.fill: "#1565c0"\n          width: 450\n          height: 56\n        }\n      }\nstatic_shapes: |md\n        dynamic=False → sizes are baked in as constants.  \n        A graph built for 128 tokens works only for 128.  \n        A new count means a new compile.\n      | {\n  style.fill: "#EEF1F8"\n  style.stroke: "#CFD2DD"\n  style.font-color: "#37474f"\n  style.font-size: 14\n  style.border-radius: 6\n  width: 580\n}\n    }\n    warmup: "Startup: warmup walks both sets of buckets" {\n      style.fill: "#EEF1F8"\n      style.stroke: "#CFD2DD"\n      label.near: top-left\n      grid-rows: 3\n      grid-columns: 1\n      grid-gap: 12\n      bodies: "for T in token buckets: _dummy_run(T)" {\n        style.fill: "#1565c0"\n        style.stroke: "#0d47a1"\n        style.font-color: "#ffffff"\n        style.font-size: 14\n        style.bold: true\n        style.border-radius: 8\n        label.near: top-center\n        grid-rows: 1\n        grid-columns: 1\n        grid-gap: 0\n        width: 580\n        bodies_d: "one compiled body graph per bucket" {\n          style.fill: "#1565c0"\n          style.stroke: "#1565c0"\n          style.font-color: "#D6E4F7"\n          style.font-size: 12\n          style.bold: false\n          width: 450\n          height: 38\n        }\n      }\n      attn: "for (S, L): _dummy_run(force_attention=True)" {\n        style.fill: "#1565c0"\n        style.stroke: "#0d47a1"\n        style.font-color: "#ffffff"\n        style.font-size: 14\n        style.bold: true\n        style.border-radius: 8\n        label.near: top-center\n        grid-rows: 1\n        grid-columns: 1\n        grid-gap: 0\n        width: 580\n        attn_d: "without the flag no attention metadata is built,\\nso the attention path is never traced or compiled" {\n          style.fill: "#1565c0"\n          width: 450\n          height: 56\n        }\n      }\n      pool: "_dummy_pooler_run(hidden_states)" {\n        style.fill: "#1565c0"\n        style.stroke: "#0d47a1"\n        style.font-color: "#ffffff"\n        style.font-size: 14\n        style.bold: true\n        style.border-radius: 8\n        label.near: top-center\n        grid-rows: 1\n        grid-columns: 1\n        grid-gap: 0\n        width: 580\n        pool_d: "warms pooler shapes · upstream\'s version is only a memory probe" {\n          style.fill: "#1565c0"\n          style.stroke: "#1565c0"\n          style.font-color: "#D6E4F7"\n          style.font-size: 12\n          style.bold: false\n          width: 450\n          height: 38\n        }\n      }\n    }\n    invariants: "Design invariants" {\n      style.bold: true\n      style.border-radius: 10\n      label.near: top-left\nrules: |md\n        · The metadata always describes the real batch, so no  \n        padded row is ever pooled.\n        · Padding the body is safe: each layer treats every  \n        token row on its own.\n      | {\n  style.fill: "#EEF1F8"\n  style.stroke: "#EEF1F8"\n  style.font-color: "#37474f"\n  style.font-size: 14\n  width: 580\n}\n    }\n  }\n}\nlower: "Attention shape strategy: dense grid today, and planned flash attention" {\n  style.fill: "#EEF1F8"\n  grid-gap: 26\n  strategies: " " {\n    style.fill: "#EEF1F8"\n    style.stroke: "#EEF1F8"\n    grid-rows: 1\n    grid-columns: 2\n    grid-gap: 26\n    dense: "A · Dense grid, the strategy shown above" {\n      grid-rows: 1\n      grid-columns: 1\n      grid-gap: 0\n      width: 700\n      dense_body: |md\n        Pack the ragged flat list into a rectangle, so every  \n        shape is fixed at compile time.  \n        Needed because an offset computed from data does not  \n        barrier per gather.  \n        Works today. This is the current implementation.\n      | {\n        style.fill: "#E3E9FD"\n      }\n    }\n    flash: "B · Flash-style attention, planned" {\n      style.fill: "#E8F5E9"\n      style.stroke: "#2e7d32"\n      style.stroke-width: 2\n      style.font-color: "#0A0F25"\n      style.font-size: 15\n      grid-columns: 1\n      grid-gap: 0\n      width: 700\n      flash_body: |md\n        Never materialise the rectangle.  \n        Each block of queries reads its request\'s offset from  \n        the metadata at runtime, and works directly on the flat  \n        list, carrying a running softmax maximum and sum.  \n        That is the online-softmax structure the Spyre decoder  \n        already uses over KV pages.\n        · no [S, 1, L, L] mask; request boundaries are the offsets  \n        · no gather_pack / gather_unpack; the kernel reads the  \n        flat list directly  \n        · no per-layer index lists, so no per-gather barriers\n        **Shape axis: block count, bucketed like the decoder\'s  \n        page loop. L disappears.**\n        Both upstream backends already work this way, neither  \n        padding into a rectangle.  \n        Flash attention consumes the flat list with  \n        cu_seqlens_q = query_start_loc, and the CPU backend  \n        passes query[:num_actual_tokens] to a C++ kernel.\n      | {\n        style.fill: "#E8F5E9"\n        style.stroke: "#E8F5E9"\n        style.font-color: "#0A0F25"\n        style.font-size: 14\n        width: 620\n      }\n    }\n  }\n  footer: " " {\n    style.fill: "#EEF1F8"\n    style.stroke: "#EEF1F8"\n    grid-rows: 1\n    grid-columns: 2\n    grid-gap: 26\n    simplifies: "If flash attention lands, the design converges on upstream" {\n      style.fill: "#FFF8E1"\n      style.stroke: "#F9A825"\n      style.stroke-width: 2\n      style.font-color: "#5d4037"\n      style.font-size: 15\n      style.bold: true\n      style.border-radius: 8\n      label.near: top-left\n      grid-rows: 1\n      grid-columns: 1\n      grid-gap: 0\n      width: 1000\n      simplifies_body: |md\n        · the second shape axis collapses; bucket T and a block count,  \n        much closer to upstream\'s single axis  \n        · the batch-shape tables box disappears, and with it the  \n        per-layer rebuild problem and its barriers  \n        · gather_pack / SDPA / gather_unpack collapse into one  \n        blocked-attention kernel  \n        · the mask\'s L² growth at long prompts disappears, today\'s  \n        largest untested risk\n        **Open question:** whether a Spyre kernel can do the runtime  \n        offset arithmetic this depends on. If it cannot, the block  \n        count stays bucketed.  \n        Everything else holds either way: padding seam, unpadded  \n        metadata, warmup, compilation mode.\n      | {\n        style.fill: "#FFF8E1"\n        style.stroke: "#FFF8E1"\n        style.font-color: "#5d4037"\n        style.font-size: 14\n        width: 920\n      }\n    }\n  }\n}\ndiff_icon: {icon: https://icons.terrastruct.com/essentials/time.svg; icon.near: outside-top-right}\n', "original",
                 ["--layout=elk", "--theme=100", "--pad=0", "--scale=0.25", "--format=png"]))
    for layout in ("dagre", "elk", "tala"):
        jobs.append(("fixed:sequence-lifeline-round-bounds", 'How this is rendered: {\n  shape: sequence_diagram\n\tCLI; d2ast; d2compiler; d2layout; d2exporter; d2themes; d2renderer; d2sequencelayout; d2dagrelayout\n  CLI -> d2ast: "\'How this is rendered: {...}\'"\n  d2ast -> CLI: tokenized AST\n  CLI -> d2compiler: compile AST\n  d2compiler."measurements also take place"\n  d2compiler -> CLI: objects and edges\n  CLI -> d2layout.layout: run layout engines\n  d2layout.layout -> d2sequencelayout: run engine on shape: sequence_diagram, temporarily remove\n  only if root is not sequence: {\n    d2layout.layout -> d2dagrelayout: run core engine on rest\n  }\n  d2layout.layout <- d2sequencelayout: add back in sequence diagrams\n  d2layout -> CLI: diagram with correct positions and dimensions\n  CLI -> d2exporter: export diagram with chosen theme and renderer\n  d2exporter.export -> d2themes: get theme styles\n  d2exporter.export -> d2renderer: render to SVG\n  d2exporter.export -> CLI: resulting SVG\n}\ndiff_icon: {icon: https://icons.terrastruct.com/essentials/time.svg; icon.near: outside-top-right}\n', "original",
                     ["--layout=" + layout, "--theme=100", "--pad=0", "--scale=0.25", "--format=png"]))
        jobs.append(("fixed:animated-svg-missing-local-image", "asset: {shape: image; icon: ./icon.svg}\nlayers: {one: {a}}\n", "original",
                     ["--layout=" + layout, "--theme=303", "--dark-theme=201", "--pad=100", "--scale=2", "--sketch", "--animate-interval=125"]))
    jobs.append(("fixed:gif-fractional-sketch", 'OEM Factory -> company Warehouse\ncompany Warehouse.Master -> company Warehouse.Regional-1\ncompany Warehouse.Master -> company Warehouse.Regional-2\ncompany Warehouse.Master -> company Warehouse.Regional-N\ncompany Warehouse.Regional-1 -> company Warehouse.Regional-2\ncompany Warehouse.Regional-2 -> company Warehouse.Regional-N\ncompany Warehouse.Regional-N -> company Warehouse.Regional-1\ncompany Warehouse: |md\n  ### company Warehouse\n|\n', "original",
                 ['--layout=tala', '--theme=105', '--dark-theme=201', '--pad=1', '--scale=0.75', '--sketch', '--center', '--format=gif']))
    jobs.append(("fixed:tala-nested-sequence-placement", 'diff_depth_5: {\ndiff_depth_4: {\ndiff_depth_3: {\ndiff_depth_2: {\ndiff_depth_1: {\ndiff_depth_0: {\nr.gg-> r.s.t\nr.s.u.v-> r.z\nr.aa-> r.s.t\nr.s.t-> g\nr.s.t-> h\n}\n\n}\n\n}\n\n}\n\n}\n\n}\n', "original",
                 ['--layout=tala', '--theme=102', '--dark-theme=200', '--pad=20', '--scale=1', '--center']))
    jobs.append(("fixed:custom-mono-unicode-fallback", 'a: "Snow 雪"\n', "original",
                 ["--layout=dagre", "--font-mono=" + str(ROOT / "crates/d2-rs-text/assets/FuzzyBubbles-Regular.ttf")]))
    font = str(ROOT / "crates/d2-rs-text/assets/FuzzyBubbles-Regular.ttf")
    jobs.append(("fixed:custom-mono-bold-label", 'a: "Snow 雪" {style.bold: true; style.font: mono}\n', "original",
                 ["--layout=dagre", "--font-mono-bold=" + font]))
    jobs.append(("fixed:custom-mono-semibold-markdown", 'a: |md\n## `Snow 雪`\n|\n', "original",
                 ["--layout=dagre", "--font-mono-semibold=" + font]))
    jobs.append(("fixed:custom-semibold-markdown", 'a: |md\n## Snow 雪\n|\n', "original",
                 ["--layout=dagre", "--font-semibold=" + font]))
    return jobs


def expanded_jobs(corpus, seed, cap):
    """Round-robin sweeps; each source/structure reaches all three engines.

    Theme IDs mirror d2themes/d2themescatalog/catalog.go at the pin. Keep
    this separate from the older generators so their queues stay reproducible.
    """
    themes = (0, 1, 3, 4, 5, 6, 7, 8, 100, 101, 102, 103, 104, 105, 200, 201, 300, 301, 302, 303)
    rng = random.Random(seed ^ 0xD250)
    for cycle in range(24):
        for index, (name, source) in enumerate(corpus):
            variant = cycle * len(corpus) + index
            family = ("deep-nest", "relative-near", "row-grid", "column-grid", "sql-table", "class-shape", "markdown", "latex", "icon", "target-animation")[variant % 10]
            if family == "deep-nest":
                text = source
                for depth in range(rng.randrange(3, 7)):
                    text = f"diff_depth_{depth}: {{\n{text}\n}}\n"
            else:
                additions = {
                    "relative-near": "diff_anchor\ndiff_near: {near: diff_anchor}\n",
                    "row-grid": "diff_grid: {grid-rows: 2; grid-gap: 7; a; b; c; d; e}\n",
                    "column-grid": "diff_grid: {grid-columns: 3; horizontal-gap: 13; a; b; c; d; e}\n",
                    "sql-table": "diff_table: {shape: sql_table; id: int {constraint: primary_key}; name: text}\n",
                    "class-shape": "diff_class: {shape: class; -count: int; +run(): bool}\n",
                    "markdown": "diff_md: |md\n# 雪 λ\n\n- **bold** and `code`\n- [link](https://example.com)\n|\n",
                    "latex": "diff_math: |latex\n\\frac{1}{2} + \\sqrt{x^2} = \\alpha\n|\n",
                    "icon": "diff_icon: {icon: https://icons.terrastruct.com/essentials/time.svg; icon.near: outside-top-right}\n",
                    "target-animation": "layers: {differential_layer: {a -> b}; differential_other: {c -> d}}\n",
                }
                text = source + "\n" + additions[family]
            for mutation, content in (("sweep-original", source), (family, text)):
                theme = themes[variant % len(themes)]
                common = [f"--theme={theme}", f"--dark-theme={(200, 201)[variant % 2]}",
                          f"--pad={(0, 1, 20, 100)[variant % 4]}", f"--scale={(0.25, 0.75, 1, 2)[variant % 4]}"]
                if variant % 2: common.append("--sketch")
                if variant % 3: common.append("--center")
                if mutation == "sweep-original" and variant % 17 == 0:
                    common.append("--animate-interval=125")
                if mutation == "target-animation":
                    common.append("--target=layers.differential_layer" if (variant // 10) % 3 == 1 else "--animate-interval=125")
                for layout in ("dagre", "elk", "tala"):
                    options = [f"--layout={layout}", *common]
                    # Sample expensive formats, including complete board trees.
                    if mutation == "target-animation" and "--animate-interval=125" in options and (variant // 10) % 3 == 2:
                        options.append("--format=gif")
                    elif "--animate-interval=125" not in options and variant % 19 == 0:
                        options.append("--format=" + ("png", "pdf", "pptx", "gif")[(variant // 19) % 4])
                    if cap <= 0: return
                    yield name, content, mutation, options
                    cap -= 1


def language_jobs(seed, cap):
    """Independent seeded language cases, including a private import tree."""
    rng = random.Random(seed ^ 0xD211)
    fixtures = {"model.d2": "anchor: {child: hello}\nother -> anchor\n",
                "nested/inner.d2": "...@../model\ninner\n",
                "cycle-a.d2": "...@cycle-b\n", "cycle-b.d2": "...@cycle-a\n"}
    cases = (
        ("import-object", "box: @model\nbox.child -> outside\n"),
        ("import-spread", "...@model\nanchor -> outside\n"),
        ("import-relative", "...@nested/inner\n"),
        ("import-cycle", "...@cycle-a\n"),
        ("import-missing", "x: @missing\n"),
        ("near-import", "...@model\ncallout: {near: anchor}\n"),
        ("near-import-child", "box: @model\ncallout: {near: box.anchor.child}\n"),
        ("vars", "vars: {color: red; title: Hello}\na: ${title} {style.fill: ${color}}\n"),
        ("vars-nested", "vars: {theme: {color: blue; text: example}}\ncontainer: {a: ${theme.text} {style.fill: ${theme.color}}}\n"),
        ("vars-import", "vars: {item: @model}\nbox: ${item}\n"),
        ("vars-missing", "a: ${missing.value}\n"),
        ("vars-cycle", "vars: {a: ${b}; b: ${a}}\nx: ${a}\n"),
        ("glob", "a; b; box: {c; d}\n*: {style.fill: red}\n**: {style.stroke: blue}\n"),
        ("glob-filter", "a: {shape: circle}; b: {shape: square}\n*: {&shape: circle; style.fill: red}\n"),
        ("glob-exclude", "a: {shape: circle}; b: {shape: square}\n*: {!&shape: circle; style.fill: red}\n"),
        ("classes", "classes: {a: {style.fill: red}; b: {style.stroke: blue}}\nx: {class: [a; b]}\n"),
        ("classes-missing", "x: {class: [unknown; missing]}\n"),
        ("classes-inherited", "classes: {first: {style.fill: red}; second: {style.bold: true}}\na: {class: [first; second]; b.class: first}\nscenarios: {one: {a.class: second}}\n"),
        ("suspend", "a -> b\nb -> c\n**: suspend\n(** -> **)[*]: suspend\na: unsuspend\nb: unsuspend\n(* -> *)[*]: unsuspend\n"),
        ("boards", "a -> b\nlayers: {one: {b -> c}}\nscenarios: {two: {a -> c}}\nsteps: {three: {c -> d}; four: {d -> e}}\n"),
        ("boards-error", "a\nsteps: {one: {b}; one: {c}}\n"),
        ("boards-nested", "a -> b\nlayers: {one: {a: changed; scenarios: {two: {b -> c; steps: {three: {c -> d}}}}}}\n"),
    )
    for i in range(cap):
        family, source = cases[i % len(cases)]
        # Vary interactions independently of the original mutation queues.
        if i >= len(cases):
            source += rng.choice((
                "\nextra: {child; child -> sibling}\n",
                "\n*: {style.opacity: 0.5}\n",
                "\nlayers: {diff: {extra -> target}}\n",
                "\nscenarios: {diff: {extra: changed}}\n",
                "\nsteps: {first: {extra}; second: {extra -> target}}\n",
                "\nextra: {near: missing}\n",
                "\nextra: ${missing}\n",
                "\nclasses: {diff: {style.stroke: red}}\nextra.class: diff\n",
            ))
        layout = rng.choice(("dagre", "elk", "tala"))
        options = ["--layout=" + layout]
        if family == "boards" and rng.randrange(2):
            options.append("--target=" + rng.choice(("layers.one", "scenarios.two", "steps.three", "steps.four")))
        yield (f"language:{family}:{i // len(cases)}", source, "language-" + family, options, fixtures)


def rendering_jobs(seed, cap):
    """Independent rendering matrix; do not perturb the existing queues."""
    rng = random.Random(seed ^ 0x4e32)
    themes = (0, 1, 3, 4, 5, 6, 7, 8, 100, 101, 102, 103, 104, 105, 200, 201, 300, 301, 302, 303)
    boards = ("a -> b: edge\n", "outer: {a -> b; c: {shape: cylinder}}\n")
    matrix = list(itertools.product(themes, themes, (False, True), ("svg", "png", "pdf", "pptx", "gif")))
    rng.shuffle(matrix)
    cases = []
    for theme, dark, sketch, fmt in matrix:
        options = ["--layout=dagre", f"--theme={theme}", f"--dark-theme={dark}", f"--format={fmt}"]
        if sketch: options.append("--sketch")
        cases.append(("render:theme-matrix", rng.choice(boards), "render-theme-matrix", options))
    additions = {
        "markdown-table": 'x: |md\n| Name | Value |\n| --- | ---: |\n| 雪 | **λ** |\n|\n',
        "markdown-list": 'x: |md\n- outer\n  - nested **bold**\n  - `code`\n- second\n|\n',
        "markdown-long": 'x: |md\n' + ('Unicode 雪 λ long line ' * 12) + '\n|\n',
        "latex": 'x: |latex\n\\sum_{i=0}^{n} \\frac{\\alpha_i}{\\sqrt{x}}\n|\n',
        "code": 'x: |go\nfunc main() { println("雪 λ") }\n|\n',
        "styles": 'a: {style: {3d: true; shadow: true; opacity: 0.6; stroke-dash: 3}}\nb: {style: {multiple: true; double-border: true; border-radius: 12; fill-pattern: dots; text-transform: uppercase}}\na -> b\n',
        "icon": 'a: {icon: ./missing.svg; icon.near: outside-top-right; width: 180; height: 120}\n',
        "local-icon": f'a: {{icon: "{ROOT / "tests/d2/svg-arc-icon.svg"}"; icon.near: top-left; width: 180; height: 120}}\n',
        "local-image": f'a: {{shape: image; icon: "{ROOT / "crates/d2-rs-render/assets/grain.png"}"; width: 160; height: 100}}\n',
    }
    extras = []
    for name, source in additions.items():
        for layout in ("dagre", "elk", "tala"):
            for sketch in (False, True):
                options = [f"--layout={layout}", "--theme=0"]
                if sketch: options.append("--sketch")
                extras.append(("render:" + name, source, "render-" + name, options))
    for flag, value in (("dagre-nodesep", "17"), ("dagre-edgesep", "11"),
                        ("elk-nodeNodeBetweenLayers", "19"), ("elk-edgeNodeBetweenLayers", "23"),
                        ("elk-edgeEdgeBetweenLayers", "29"), ("elk-nodeSelfLoop", "31"),
                        ("elk-padding", "[top=12,left=13,bottom=14,right=15]"), ("tala-seeds", "4,5")):
        layout = flag.split("-", 1)[0]
        extras.append(("render:engine-" + flag, "a -> b\nb -> c\nc -> c\n", "render-engine", [f"--layout={layout}", f"--{flag}={value}"]))
    font = ROOT / "crates/d2-rs-text/assets/FuzzyBubbles-Regular.ttf"
    for flag in ("regular", "italic", "bold", "semibold", "mono", "mono-bold", "mono-italic", "mono-semibold"):
        if flag.endswith("semibold"):
            source = 'a: |md\n## Snow 雪\n|\n'
            if flag.startswith("mono"):
                source = 'a: |md\n## `Snow 雪`\n|\n'
        else:
            styles = ["style.bold: " + ("true" if flag.endswith("bold") else "false")]
            if flag.endswith("italic"): styles.append("style.italic: true")
            if flag.startswith("mono"): styles.append("style.font: mono")
            source = 'a: "Snow 雪" {' + '; '.join(styles) + '}\n'
        extras.append(("render:font-" + flag, source, "render-font", ["--layout=dagre", f"--font-{flag}={font}"]))
    rng.shuffle(extras)
    # Interleave extras so a capped run exercises each family.
    for i in range(min(cap, len(cases) + len(extras))):
        if extras and (i % 3 == 0 or not cases):
            yield extras.pop()
        elif cases:
            yield cases.pop()


def rendering_detail_jobs(seed, cap):
    """Additional seeded property and text combinations, separate from prior queues."""
    rng = random.Random(seed ^ 0x4e3233)
    shapes = ("rectangle", "circle", "diamond", "hexagon", "cylinder", "cloud", "person", "document")
    styles = ("3d: true", "multiple: true", "double-border: true", "shadow: true",
              "opacity: 0.35", "stroke-dash: 5", "border-radius: 16",
              "fill-pattern: lines", "text-transform: uppercase")
    labels = ('"Snow 雪 λ"', '|md\n- one\n  - **nested**\n  - `code`\n|',
              '|!md\n| A | B |\n| --- | --- |\n| λ | 雪 |\n!|',
              '|latex\n\\frac{a+b}{\\sqrt{x}}\n|', '|python\nprint("雪")\n|')
    jobs = []
    for shape, style, label, sketch in itertools.product(shapes, styles, labels, (False, True)):
        source = f'a: {label} {{shape: {shape}; style: {{{style}}}}}\n'
        options = ["--layout=dagre", "--theme=0"]
        if sketch: options.append("--sketch")
        jobs.append(("render-detail:" + shape, source, "render-detail-properties", options))
    rng.shuffle(jobs)
    yield from jobs[:cap]


def provenance(binary, oracle=False, clock=False):
    version = subprocess.check_output([str(binary), "--version"], text=True, timeout=10).strip()
    record = {"path": str(binary), "version": version, "sha256": references.digest(binary)}
    if oracle:
        metadata = subprocess.check_output(["go", "version", "-m", str(binary)], text=True, timeout=10)
        arch = {"x86_64": "amd64", "aarch64": "arm64"}[platform.machine()]
        if version != "v0.9.0" or references.MANIFEST["d2"]["go_toolchain"] not in metadata.splitlines()[0] or f"vcs.revision={references.SOURCE_COMMIT}" not in metadata or f"GOARCH={arch}" not in metadata or "Version=v0.9.0" not in metadata:
            raise ValueError("Go oracle must be the pinned, v0.9.0-stamped host build")
        if not clock and "vcs.modified=false" not in metadata:
            raise ValueError("Go oracle must be a clean pinned build")
        record["build_metadata"] = metadata
    return record


def failed(row):
    rust = row.get("rust", {})
    return (row["outcome"] not in ("match", "go-crash-or-timeout") or bool(rust.get("invalid_output")) or
            isinstance(rust.get("exit"), int) and rust["exit"] < 0 or any(
                row.get(engine, {}).get("exit") in ("timeout", "process-failure", "budget-exceeded") for engine in ("go", "rust")))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--go", type=Path, default=references.GO_ORACLE)
    p.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    p.add_argument("--output", type=Path, default=references.REPORTS / "differential/latest.json")
    p.add_argument("--seed", type=int, default=20261101)
    p.add_argument("--cap", type=int, default=2000, help="render sample cap; commands cover each corpus input additionally")
    p.add_argument("--timeout", type=float, default=10)
    p.add_argument("--render-timeout", type=float, default=60)
    p.add_argument("--tala-timeout", type=float, default=30)
    p.add_argument("--png-timeout", type=float, default=60)
    p.add_argument("--jobs", type=int, default=8)
    p.add_argument("--epoch", type=int, help="shared UTC clock; requires build-cli-clock-oracle.py oracle")
    p.add_argument("--baseline-matrix", action="store_true", help="retain original option matrix and omit command jobs")
    p.add_argument("--legacy-mutations", action="store_true")
    p.add_argument("--expanded-matrix", action="store_true", help="seeded structural and full theme sweeps across all engines")
    p.add_argument("--language-matrix", action="store_true", help="run the independent language mutation queue")
    p.add_argument("--rendering-matrix", action="store_true", help="seeded rendering families and theme cross-product")
    p.add_argument("--rendering-detail-matrix", action="store_true", help="seeded shape, style and text combinations")
    p.add_argument("--minimize-seconds", type=float, default=15)
    p.add_argument("--minimize", type=int, default=8, help="maximum clusters to minimize with at most 40 attempts each")
    args = p.parse_args()
    if min(args.cap, args.jobs, args.timeout, args.render_timeout, args.tala_timeout, args.png_timeout) <= 0 or args.minimize < 0: p.error("budgets must be positive")
    if sum((args.expanded_matrix, args.baseline_matrix, args.language_matrix, args.rendering_matrix, args.rendering_detail_matrix)) > 1: p.error("matrix modes are mutually exclusive")
    if not args.baseline_matrix and args.epoch is None: p.error("wide matrix requires --epoch and a clock oracle (build-cli-clock-oracle.py)")
    if args.epoch is not None and not 0 <= args.epoch <= 253402300799: p.error("invalid epoch")
    args.go, args.rust = args.go.resolve(), args.rust.resolve()
    references.verify_source()
    if args.epoch is not None:
        with tempfile.TemporaryDirectory(prefix="d2-clock-probe-") as temp:
            probe = run(args.go, "a->b\n", ["fmt-check"], args.timeout, Path(temp), args.epoch)
        timestamp = datetime.datetime.fromtimestamp(args.epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ").encode()
        if timestamp not in probe.get("stdout", b""): p.error("clock oracle warning probe failed")
    identity = {"go": provenance(args.go, True, clock=args.epoch is not None), "rust": provenance(args.rust),
                "source_head": references.command("git", "rev-parse", "HEAD", cwd=ROOT),
                "upstream_commit": references.command("git", "rev-parse", "HEAD", cwd=UPSTREAM)}
    if args.epoch is not None and args.go == references.GO_ORACLE.resolve(): p.error("--epoch requires a clock-instrumented oracle")
    rng = random.Random(args.seed)
    option_rng = random.Random(args.seed ^ 0xD2)
    corpus = list(inputs()); rng.shuffle(corpus)
    jobs = []
    for cycle in range(0 if args.expanded_matrix or args.language_matrix or args.rendering_matrix or args.rendering_detail_matrix else 24):
        for name, source in corpus:
            mutated, kind = mutate(source, rng, args.legacy_mutations)
            for text, mutation in ((source, "original"), (mutated, kind)):
                i = len(jobs)
                variant = (cycle + i // 2) % 24
                options = [f"--layout={('dagre', 'elk', 'tala')[variant % 3]}", f"--theme={(0, 200)[(variant // 3) % 2]}"]
                if (variant // 6) % 2: options.append("--sketch")
                options.append(f"--pad={(0, 20)[(variant // 12) % 2]}")
                if i // 2 % 50 == 0: options.append("--png")
                if not args.baseline_matrix: options = widened_options(options, i, text, option_rng)
                jobs.append((name, text, mutation, options))
                if len(jobs) >= args.cap: break
            if len(jobs) >= args.cap: break
        if len(jobs) >= args.cap: break
    if args.expanded_matrix: jobs = list(expanded_jobs(corpus, args.seed, args.cap))
    if args.language_matrix: jobs = list(language_jobs(args.seed, args.cap))
    if args.rendering_matrix: jobs = list(rendering_jobs(args.seed, args.cap))
    if args.rendering_detail_matrix: jobs = list(rendering_detail_jobs(args.seed, args.cap))
    if not args.language_matrix: jobs += fixed_jobs(not args.baseline_matrix)
    if not (args.baseline_matrix or args.language_matrix or args.rendering_matrix or args.rendering_detail_matrix):
        # Every original corpus input, plus sampled mutations and witnesses.
        command_sources = dict((source, name) for name, source in corpus)
        command_sources.update((source, name) for name, source, _, _ in jobs)
        jobs += [(name, source, "original", [command]) for source, name in command_sources.items()
                 for command in ("fmt", "fmt-check", "validate")]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        rows = [None] * len(jobs)
        futures = {pool.submit(compare, job, args): i for i, job in enumerate(jobs)}
        for count, future in enumerate(concurrent.futures.as_completed(futures), 1):
            rows[futures[future]] = future.result()
            if count % 100 == 0: print(f"{count}/{len(jobs)} scored", flush=True)
    # Retry timeouts once in isolation with the same timeout and environment.
    for i, row in enumerate(rows):
        if any(row.get(engine, {}).get("exit") == "timeout" for engine in ("go", "rust")):
            print(f"retry timeout {i + 1}/{len(rows)}: {row['input']}", flush=True)
            retry = compare(jobs[i], args)
            retry["first_attempt"] = row
            rows[i] = retry
    if any(references.digest(getattr(args, side)) != identity[side]["sha256"] for side in ("go", "rust")):
        rows.append({"outcome": "provenance-changed"})
    summary = collections.Counter(row["outcome"] for row in rows)
    families = collections.defaultdict(collections.Counter)
    for row in rows:
        family = next((option for option in row.get("options", ()) if option in ("fmt", "fmt-check", "validate")), row.get("mutation", "unknown"))
        families[family][row["outcome"]] += 1
    clusters = collections.Counter(row["cluster"] for row in rows if row["outcome"] == "mismatch")
    witnesses = []
    for cluster, _ in clusters.most_common(args.minimize):
        row = next(row for row in rows if row.get("cluster") == cluster)
        source = row["source"]
        lines = source.splitlines(keepends=True)
        attempts = 0
        deadline = time.monotonic() + args.minimize_seconds
        print(f"minimize {cluster}", flush=True)
        # Bounded line delta-debugging. Each accepted reduction reproduces the
        # same cluster under precisely the original options.
        width = max(1, len(lines) // 2)
        while width and attempts < 40 and time.monotonic() < deadline:
            changed = False
            for i in range(0, len(lines), width):
                candidate = lines[:i] + lines[i + width:]
                result = compare((row["input"], "".join(candidate), "minimized", row["options"], row.get("fixtures", {})), args)
                attempts += 1
                if result.get("cluster") == cluster and result["outcome"] == "mismatch":
                    lines = candidate; changed = True; break
                if attempts >= 40 or time.monotonic() >= deadline: break
            if not changed: width //= 2
        witnesses.append({"cluster": cluster, "input": row["input"], "options": row["options"],
                          "source": "".join(lines), "fixtures": row.get("fixtures", {}), "attempts": attempts})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {"budget_bytes": BUDGET, "file_limit_bytes": FILE_LIMIT, "captured_bytes": budget_used,
              "seed": args.seed, "corpus_size": len(corpus), "clock": args.epoch, "provenance": identity,
              "summary": summary, "families": families, "expanded_matrix": args.expanded_matrix,
              "rendering_detail_matrix": args.rendering_detail_matrix,
              "clusters": clusters.most_common(), "witnesses": witnesses, "rows": rows}
    encoded = json.dumps(report, ensure_ascii=False, indent=1) + "\n"
    if len(encoded.encode()) > FILE_LIMIT or budget_used + len(encoded.encode()) > BUDGET:
        raise RuntimeError("report exceeds capture budget")
    args.output.write_text(encoded)
    print(dict(summary))
    for cluster, count in clusters.most_common(): print(count, cluster)
    return int(any(failed(row) for row in rows))


if __name__ == "__main__":
    sys.exit(main())
