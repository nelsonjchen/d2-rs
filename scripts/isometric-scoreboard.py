#!/usr/bin/env python3
"""Score d2-rs-isometric against the pinned Go isometric renderer.

Inputs are the upstream e2e goldens' laid-out diagrams (e2etests/testdata/
<case>/<engine>/board.exp.json) with the case's theme from the e2e case dump,
exactly as e2etests/isometric_test.go renders them. Both engines run the same
stdin protocol (tools/isometric-probe/main.go, built inside the trace checkout;
crates/d2-rs-isometric/examples/isometric-probe.rs) for every board in source
traversal order.

Tiers:
* api: original public RenderPage, PNG and SVG calls, complete PNG/SVG bytes,
  annotations and errors (select --suite page|svg|png|paint|panels|composition|
  fonts|links|still|rich-boards). PNG and SVG also
  include source-backed golden-board deadline witnesses; paint captures authored
  gradient/icon rendering and a container-gradient admission board witness.
  Panels exercises embedded/external legend icons and caller import limits on
  the three upstream txtar/legend-mono boards through all three still APIs.
  Composition observes the original legend, sequence and decorated-root fixtures
  at their public still-image boundaries. Fonts checks configured primary/mono
  face admission on unicode/with-style and txtar/legend-mono boards.
  Rich-boards observes the exact minimal targets constructed by six original
  Markdown/code/class/SQL/LaTeX tests through complete public still images.
* png: all 20 upstream PNG goldens, on ordinary Go raster meshes and optionally
  native whole-board meshes; exact decoded pixels after Go golden reproduction.
* scene: d2isometric.BuildScene JSON, compared value by value (numbers as
  float64, no tolerance) or the exact error text.
* svg: d2isometricimg.Render SVG bytes (1200x1200, FitContent), or the exact
  error text. Each Go SVG must equal the committed isometric*.exp.svg golden,
  otherwise the board is go-unreproduced and cannot pass.
* surfaces: the complete native SVG writer and retained vector-surface serializer
  on Go meshes/documents, compared byte for byte with independently reproduced
  Go SVG goldens. This does not qualify native mesh construction.
* text: actual plain-label painter calls made by the Go mesh builder; compare
  physical layout, complete retained SVG fragments and premultiplied RGBA hashes.
  This does not qualify native mesh construction or rich-label painting.
* rich: actual rich-label painter calls, compared by source viewport, complete
  retained SVG and premultiplied RGBA hashes. Class/SQL cells bypass this painter.
* primitives: actual native tessellation calls (box, sphere, cylinder, flat,
  extruded profile), compared as complete ordered triangles/material identities.
  This does not qualify complete mesh assembly, framing or SVG end to end.
* writer: the native SVG writer (visibility, ordering, batching, lighting,
  covers, ground shadow, links) on Go's own mesh, dumped by the test-only hook
  tools/isometric-probe/mesh_hook_test.go. Retained vector-surface fragments
  are replaced by one marker on both sides (balanced <g> scan of the golden),
  so this tier qualifies everything except surface serialization.

Missing or extra outputs are failures. Captures are budgeted (default 1 GiB).

usage: isometric-scoreboard.py --tier scene|svg|writer|surfaces|text|rich|primitives --capture <fresh dir> --output <report.json>
"""
from __future__ import annotations

import argparse
import collections
import contextlib
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = (ROOT / references.MANIFEST["d2"]["checkout"]).resolve()
TESTDATA = UPSTREAM / "e2etests/testdata"
GO_PROBE = ROOT / f".reference/bin/d2-isometric-probe-{references.SOURCE_COMMIT[:8]}"
RUST_PROBE = ROOT / "target/diagnostic/examples/isometric-probe"
RUST_WRITER = ROOT / "target/diagnostic/examples/isometric-writer"
TRACE = references.TRACE_CHECKOUT
HOOK = ROOT / "tools/isometric-probe/mesh_hook_test.go"


def stub_surfaces(svg):
    """Replace every retained surface fragment with the writer tier's marker."""
    out, i = [], 0
    pattern = re.compile(r'(<defs><g id="surface-\d+">|<g transform="scale\(\d+ \d+\)">)')
    while True:
        m = pattern.search(svg, i)
        if not m:
            out.append(svg[i:])
            break
        out.append(svg[i:m.end()])
        depth, j = 1, m.end()
        tag = re.compile(r'<g[ >]|</g>')
        while depth:
            t = tag.search(svg, j)
            depth += 1 if t.group() != "</g>" else -1
            j = t.end()
        out.append("§</g>")
        i = j
    return "".join(out)


def expected_boards(rows):
    expected = []
    for row in rows:
        relative = Path(row["board"]).parent.relative_to(TESTDATA).as_posix()
        def walk(diagram, name, relative=relative):
            if not diagram.get("isFolderOnly", False):
                expected.append(relative + "/" + name)
            for kind in ("layers", "scenarios", "steps"):
                for index, child in enumerate(diagram.get(kind) or []):
                    walk(child, f"{name}.{kind}.{index}")
        walk(json.loads(Path(row["board"]).read_text()), "isometric")
    return expected


@contextlib.contextmanager
def text_instrumentation(rich=False):
    """Leave the pinned source body unchanged; wrap only in the trace checkout.

    The test-only wrapper is installed for one serial producer and removed even
    on failure. Check exact source bytes before applying and before restoring.
    """
    package = TRACE / "d2renderers/d2isometric/d2isometricimg"
    source = package / ("rich_labels.go" if rich else "text.go")
    original = source.read_bytes()
    assert original == (UPSTREAM / "d2renderers/d2isometric/d2isometricimg" / source.name).read_bytes()
    old = b"func (p *richLabelPainter) texture(" if rich else b"func (p *textPainter) texture("
    new = old.replace(b" texture(", b" d2rsOriginalTexture(")
    assert original.count(old) == 1
    instrumented = original.replace(old, new)
    hook_name = "rich_hook_test.go" if rich else "text_hook_test.go"
    hook = package / ("d2rs_" + hook_name)
    assert not hook.exists(), "another text producer is active"
    hook.write_bytes(HOOK.with_name(hook_name).read_bytes())
    try:
        source.write_bytes(instrumented)
        yield
    finally:
        assert source.read_bytes() == instrumented, "trace text source changed during capture; refusing to overwrite"
        source.write_bytes(original)
        hook.unlink()


@contextlib.contextmanager
def primitive_instrumentation():
    """Observe actual calls, retaining every pinned source instruction."""
    package = TRACE / "d2renderers/d2isometric/d2isometricimg"
    edits = []
    hook = package / "d2rs_primitive_hook_test.go"
    assert not hook.exists(), "another primitive producer is active"
    for name, methods in (("geometry.go", ("box", "sphere", "cylinder", "flat")),
                          ("native_shapes.go", ("extrudedProfile",))):
        path = package / name
        original = path.read_bytes()
        assert original == (UPSTREAM / "d2renderers/d2isometric/d2isometricimg" / name).read_bytes()
        changed = original
        for method in methods:
            old = f"func (b *meshBuilder) {method}(".encode()
            new = f"func (b *meshBuilder) d2rsOriginal{method[0].upper() + method[1:]}(".encode()
            assert changed.count(old) == 1
            changed = changed.replace(old, new)
        edits.append((path, original, changed))
    hook.write_bytes(HOOK.with_name("primitive_hook_test.go").read_bytes())
    try:
        for path, _, changed in edits:
            path.write_bytes(changed)
        yield
    finally:
        for path, original, changed in edits:
            assert path.read_bytes() == changed, "trace source changed during primitive capture"
            path.write_bytes(original)
        hook.unlink()


def writer_tier(capture, rows, full=False, rust=RUST_WRITER, budget=1 << 30, baseline=None, text=False, rich=False, primitives=False):
    with primitive_instrumentation() if primitives else text_instrumentation(rich) if text else contextlib.nullcontext():
        return _writer_tier(capture, rows, full, rust, budget, baseline, text, rich, primitives)


def compare_text_calls(calls, actual):
    if not isinstance(actual, list) or len(actual) != len(calls):
        return [{"inventory": "native call count differs"}]
    differences = []
    for i, (call, got) in enumerate(zip(calls, actual)):
        want = call["want"]
        if not isinstance(got, dict):
            differences.append({"call": i, "inventory": "native call is not an object"})
        elif not equal_call_value(want, got):
            differences.append({"call": i, "fields": sorted(key for key in set(want) | set(got)
                                                           if key not in want or key not in got or not equal_call_value(want[key], got[key]))})
    return differences


def equal_call_value(a, b):
    """Go JSON numbers are float64; preserve boolean and container types."""
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (float, int)) and isinstance(b, (float, int)):
        return float(a) == float(b)
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(equal_call_value(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(equal_call_value(x, y) for x, y in zip(a, b))
    return a == b


class CaptureLimit(RuntimeError):
    pass


def bounded_native(command, log, guard, timeout=600):
    """Monitor native output while it is being written, including on failure."""
    process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    started = time.monotonic()
    try:
        while True:
            failure = guard()
            if failure:
                raise CaptureLimit(failure)
            if time.monotonic() - started > timeout:
                raise subprocess.TimeoutExpired(command, timeout)
            code = process.poll()
            if code is not None:
                return code
            time.sleep(0.05)
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def _writer_tier(capture, rows, full=False, rust=RUST_WRITER, budget=1 << 30, baseline=None, text=False, rich=False, primitives=False):
    """Consume atomically published Go meshes as they arrive; retire passing dumps.

    Inventory is derived from the inputs, never from the outputs we happened to
    receive. The source must complete successfully; missing/error/extra rows
    remain explicit. Failed native meshes are retained as witnesses.
    """
    actual_commit = references.command("git", "rev-parse", "HEAD", cwd=TRACE)
    assert actual_commit == references.SOURCE_COMMIT, "trace checkout does not match the pinned oracle"
    package = TRACE / "d2renderers/d2isometric/d2isometricimg"
    for source, name in [(HOOK.with_name("asset_options_test.go"), "d2rs_asset_options_test.go"),
                         (HOOK, "d2rs_mesh_hook_test.go"),
                         (HOOK.with_name("surface_json_test.go"), "d2rs_surface_json_test.go")]:
        (package / name).write_text(source.read_text())
    requests_path = capture / "mesh-requests.jsonl"
    requests_path.write_text("\n".join(json.dumps({**r, "out": r["out"].replace("/rust/", "/mesh/", 1),
        "mode": "primitives" if primitives else "rich" if rich else "text" if text else "surfaces" if full else "mesh", "assets": str(capture / "assets"), "stream": True}) for r in rows) + "\n")
    env = dict(os.environ, D2RS_ISOMETRIC_MESH=str(requests_path), GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
               GOCACHE=str(ROOT / ".reference/go-build"), GOFLAGS="-mod=mod", GOPROXY="off", GOTOOLCHAIN=references.MANIFEST["d2"]["go_toolchain"])
    inventory = expected_boards(rows)
    assert inventory and len(inventory) == len(set(inventory)), "empty or duplicate board inventory"
    expected = set(inventory)
    results, seen = [], set()
    with (capture / "go.log").open("w") as log, (capture / "rust.log").open("w") as native_log:
        process = subprocess.Popen(["go", "test", "./d2renderers/d2isometric/d2isometricimg", "-run", "TestD2rsMeshDump",
                               "-count=1", "-timeout", "3h"], cwd=TRACE, env=env, stdout=log, stderr=subprocess.STDOUT,
                               start_new_session=True)
        failure = None
        statistics = {"peak_bytes": 0, "largest_file": 0, "retired": []}
        def guard():
            sizes = []
            for path in capture.rglob("*"):
                try:
                    if path.is_file():
                        sizes.append(path.stat().st_size)
                except FileNotFoundError:
                    pass
            statistics["peak_bytes"] = max(statistics["peak_bytes"], sum(sizes))
            statistics["largest_file"] = max(statistics["largest_file"], max(sizes, default=0))
            if sum(sizes) > budget or max(sizes, default=0) > 128 << 20:
                return "budget-exceeded"
            if shutil.disk_usage(capture).free < 20 << 30:
                return "free-space-exhausted"
            return None
        def retire(path):
            stat = path.stat()
            statistics["retired"].append({"path": str(path), "bytes": stat.st_size,
                                          "allocated": stat.st_blocks * 512})
            path.unlink()
        def interrupted(signum, frame):
            raise KeyboardInterrupt(f"capture interrupted by signal {signum}")
        previous_handler = signal.signal(signal.SIGTERM, interrupted)
        started = time.monotonic()
        try:
            while True:
                for mesh in sorted((capture / "mesh").rglob("*")):
                    if not mesh.is_file() or (mesh.suffix != ".err" and not mesh.name.endswith(".mesh.json")) or mesh in seen:
                        continue
                    seen.add(mesh)
                    rel = mesh.relative_to(capture / "mesh").as_posix()
                    board = rel.removesuffix(".err") if mesh.suffix == ".err" else rel.removesuffix(".mesh.json")
                    golden = TESTDATA / (board + ".exp.svg")
                    row = {"board": board}
                    if board not in expected:
                        row["outcome"] = "go-extra-board"
                    elif mesh.suffix == ".err":
                        row["outcome"] = "go-error"
                        row["go_error"] = mesh.read_text()[:300]
                    elif not golden.is_file() or not (capture / "go" / (board + ".svg")).is_file() or (capture / "go" / (board + ".svg")).read_bytes() != golden.read_bytes():
                        row["outcome"] = "go-unreproduced"
                    else:
                        out = capture / "rust" / (board + ".svg")
                        out.parent.mkdir(parents=True, exist_ok=True)
                        if baseline is not None:
                            before = capture / "before" / (board + ".svg")
                            before.parent.mkdir(parents=True, exist_ok=True)
                            previous = bounded_native([str(baseline.resolve()), str(mesh), str(before)], native_log, guard)
                            row["before"] = "match" if previous == 0 and before.is_file() and before.read_bytes() == golden.read_bytes() else "mismatch"
                            row["before_writer"] = "match" if previous == 0 and before.is_file() and before.read_text() == stub_surfaces(golden.read_text()) else "mismatch"
                        prior_outputs = {p for p in out.parent.rglob("*") if p.is_file()}
                        command = [str(rust.resolve()), str(mesh), str(out)] + (["--surfaces"] if full and not text else [])
                        try:
                            code = bounded_native(command, native_log, guard)
                        except subprocess.TimeoutExpired:
                            code = -1
                        except CaptureLimit as error:
                            failure, code = str(error), -1
                        produced = {p for p in out.parent.rglob("*") if p.is_file()} - prior_outputs
                        permitted = {out, out.with_name(out.name + ".err")}
                        if produced - permitted or len(produced & permitted) > 1:
                            row["outcome"] = "rust-extra-output"
                            row["unexpected_outputs"] = sorted(str(p) for p in produced)
                        elif code != 0:
                            row["outcome"] = "rust-failed"
                            row["exit"] = code
                        elif out.is_file():
                            if text or primitives:
                                calls = json.loads(mesh.read_bytes())
                                if primitives:
                                    witnesses = capture / "witnesses"
                                    witnesses.mkdir(exist_ok=True)
                                    for call in calls:
                                        path = witnesses / (call["op"] + ".json")
                                        if not path.exists():
                                            path.write_text(json.dumps(call) + "\n")
                                            manifest_path = witnesses / "manifest.json"
                                            manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
                                            manifest[call["op"]] = {"board": board, "sha256": references.digest(path)}
                                            manifest_path.write_text(json.dumps(manifest, indent=1) + "\n")
                                if primitives:
                                    row["operations"] = dict(collections.Counter(call["op"] for call in calls))
                                row["calls"] = len(calls)
                                row["input_sha256"] = references.digest(mesh)
                                try:
                                    actual = json.loads(out.read_bytes())
                                except (ValueError, UnicodeDecodeError) as error:
                                    row["outcome"] = "rust-invalid-output"
                                    row["rust_error"] = str(error)[:300]
                                else:
                                    differences = compare_text_calls(calls, actual)
                                    row["matched_calls"] = max(0, len(calls) - len(differences))
                                    row["outcome"] = "mismatch" if differences else "match"
                                    row["call_differences"] = differences
                            else:
                                source = golden.read_text()
                                row["writer"] = "match" if stub_surfaces(out.read_text()) == stub_surfaces(source) else "mismatch"
                                expected_svg = source if full else stub_surfaces(source)
                                row["outcome"] = "match" if out.read_text() == expected_svg else "mismatch"
                        else:
                            err = out.with_name(out.name + ".err")
                            row["outcome"] = "rust-error"
                            row["rust_error"] = err.read_text()[:300] if err.is_file() else "missing output"
                        if row["outcome"] == "match":
                            # Successful bytes are bound to the pristine golden;
                            # keep compact inventories rather than a second corpus.
                            row["output_sha256" if text or primitives else "svg_sha256"] = hashlib.sha256(out.read_bytes()).hexdigest()
                            row["output_bytes" if text or primitives else "svg_bytes"] = out.stat().st_size
                            retire(out)
                            retire(mesh)
                        source_svg = capture / "go" / (board + ".svg")
                        if source_svg.is_file() and source_svg.read_bytes() == golden.read_bytes():
                            row["go_svg_sha256"] = hashlib.sha256(source_svg.read_bytes()).hexdigest()
                            retire(source_svg)
                        if baseline is not None and row.get("before_writer") == "match":
                            # Exact reconstructible negative: stub_surfaces(golden).
                            row["before_sha256"] = hashlib.sha256(before.read_bytes()).hexdigest()
                            row["before_encoding"] = "stub_surfaces(golden)"
                            retire(before)
                    results.append(row)
                    with (capture / "results.jsonl").open("a") as completed:
                        completed.write(json.dumps(row) + "\n")
                    mesh.with_name(mesh.name + ".consumed").touch()
                failure = failure or guard()
                if time.monotonic() - started > 10800:
                    failure = "timeout"
                if failure:
                    break
                if process.poll() is not None:
                    # One final scan includes the last atomic publication.
                    pending = [p for p in (capture / "mesh").rglob("*") if p.is_file() and (p.suffix == ".err" or p.name.endswith(".mesh.json")) and p not in seen]
                    if not pending:
                        break
                time.sleep(0.1)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
            signal.signal(signal.SIGTERM, previous_handler)
            (capture / "capture-statistics.json").write_text(json.dumps(statistics, indent=1) + "\n")
    for board in sorted(expected - {r["board"] for r in results}):
        results.append({"board": board, "outcome": "go-missing"})
    if code != 0 or failure:
        for row in results:
            if row["outcome"] == "match":
                row["outcome"] = "incomplete-run"
    return code, sorted(results, key=lambda r: r["board"]), failure


def requests(out, mode, only):
    cases = {}
    for path in references.E2E_CASES.glob("*.json"):
        case = json.loads(path.read_text())
        cases[case["name"]] = case
    rows = []
    for board in sorted(TESTDATA.rglob("board.exp.json")):
        relative = board.parent.relative_to(TESTDATA).as_posix()
        name, _engine = relative.rsplit("/", 1)
        case = cases.get(name)
        if case is None or (only and only not in relative):
            continue
        rows.append({"board": str(board), "themeID": case.get("themeID"),
                     "out": str(out / relative), "mode": mode})
    return rows


def direct_guard(capture, budget):
    total = 0
    for path in capture.rglob("*"):
        try:
            if not path.is_file():
                continue
            length = path.stat().st_size
        except FileNotFoundError:
            continue
        total += length
        if length > 128 << 20:
            return "file-budget-exceeded"
    if total > budget:
        return "budget-exceeded"
    if shutil.disk_usage(capture).free < 20 << 30:
        return "free-space-budget-exceeded"
    return None


def run(probe, rows, log, capture, budget):
    request = log.with_suffix(".requests.jsonl")
    request.write_text("".join(json.dumps(r) + "\n" for r in rows))
    with request.open() as source, log.open("w") as stream:
        process = subprocess.Popen([str(probe)], stdin=source, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        failure = None
        started = time.monotonic()
        try:
            while process.poll() is None:
                failure = direct_guard(capture, budget)
                if failure or time.monotonic() - started > 7200:
                    failure = failure or "timeout"
                    break
                time.sleep(0.1)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
    return code, failure or direct_guard(capture, budget)


def direct_inventory(root):
    files, metadata = {}, []
    for p in root.rglob("*"):
        if not p.is_file():
            continue
        relative = p.relative_to(root)
        # Finder metadata is recorded separately; it cannot denote a board.
        if p.name == ".DS_Store":
            metadata.append({"path": str(relative), "sha256": references.digest(p)})
        else:
            files[relative] = p
    return files, metadata


def compare_direct(capture, requests_, tier, runs, failure=None, source_profile=False):
    expected = set(expected_boards(requests_))
    if not expected:
        raise ValueError("an empty capture cannot pass")
    go_files, go_metadata = direct_inventory(capture / "go")
    rust_files, rust_metadata = direct_inventory(capture / "rust")
    def group(files):
        result = collections.defaultdict(list)
        for rel, path in files.items():
            result[rel.with_suffix("").as_posix()].append(path)
        return result
    go, rust = group(go_files), group(rust_files)
    results = []
    for name in sorted(expected | set(go) | set(rust)):
        row = {"board": name}
        left, right = go.get(name, []), rust.get(name, [])
        if name not in expected:
            row["outcome"] = "unexpected-board"
        elif len(left) != 1:
            row["outcome"] = "go-missing" if not left else "go-duplicate"
        elif len(right) != 1:
            row["outcome"] = "rust-missing" if not right else "rust-duplicate"
        else:
            source, native = left[0], right[0]
            a, b = source.read_bytes(), native.read_bytes()
            row.update(go_sha256=hashlib.sha256(a).hexdigest(), rust_sha256=hashlib.sha256(b).hexdigest())
            suffix = source.suffix
            if suffix not in ({".svg", ".err"} if tier == "svg" else {".json", ".err"}) or not a:
                row["outcome"] = "go-invalid-output"
            elif source_profile and suffix == ".svg" and not valid_svg(a):
                row["outcome"] = "go-invalid-output"
            elif suffix == ".svg" and not source_profile and (not (TESTDATA / (name + ".exp.svg")).is_file()
                    or (TESTDATA / (name + ".exp.svg")).read_bytes() != a):
                row["outcome"] = "go-unreproduced"
            elif suffix != native.suffix:
                row["outcome"] = "rust-error" if native.suffix == ".err" else "rust-unexpected-output"
            elif not b:
                row["outcome"] = "rust-empty"
            elif suffix == ".json":
                try:
                    row["outcome"] = "match" if equal_call_value(json.loads(a), json.loads(b)) else "mismatch"
                except ValueError:
                    row["outcome"] = "invalid-json"
            else:
                row["outcome"] = ("error-match" if suffix == ".err" else "match") if a == b else "mismatch"
                if source_profile and row["outcome"] in ("match", "error-match"):
                    row["outcome"] = "source-" + row["outcome"]
            if native.suffix == ".err":
                row["rust_error"] = b.decode(errors="replace")[:300]
            if suffix == ".err":
                row["kind"] = "error"
                row["go_error"] = a.decode(errors="replace")[:300]
        if (failure or any(runs.values())) and row["outcome"] in ("match", "error-match", "source-match", "source-error-match"):
            row["outcome"] = "incomplete-run"
        results.append(row)
    return results, {"go": go_metadata, "rust": rust_metadata}


def valid_svg(data):
    try:
        return ET.fromstring(data).tag in ("svg", "{http://www.w3.org/2000/svg}svg")
    except ET.ParseError:
        return False


def asset_sources(path):
    """Admit explicit captured HTTP inputs, preserving original board locators.

    These source comparisons cannot count as upstream golden matches: the
    unconfigured upstream tests deliberately expect external-resource errors.
    """
    manifest = json.loads(path.read_text())
    if not isinstance(manifest, dict) or not manifest:
        raise ValueError("asset manifest must be a nonempty source map")
    total = 0
    for source, value in manifest.items():
        if not source.startswith(("https://", "http://")):
            raise ValueError("captured transport requires HTTP source locators")
        asset = Path(value["path"])
        if not asset.is_absolute() or not value["mime"]:
            raise ValueError("asset inputs require absolute paths and MIME types")
        length = asset.stat().st_size
        total += length
        if not length or length > 8 << 20 or total > 16 << 20:
            raise ValueError("asset input byte budget exceeded")
        if references.digest(asset) != value["sha256"]:
            raise ValueError("asset input hash differs")
    return manifest


def size(path):
    return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())


def main():
    import sys
    if "--tier" in sys.argv and sys.argv[sys.argv.index("--tier") + 1] == "api":
        sys.path.insert(0, str(ROOT / "tools/isometric-probe"))
        import api_scoreboard
        return api_scoreboard.main()
    if "--tier" in sys.argv and sys.argv[sys.argv.index("--tier") + 1] == "png":
        import isometric_png_scoreboard
        return isometric_png_scoreboard.main()
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tier", choices=["scene", "svg", "writer", "surfaces", "text", "rich", "primitives"], required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rust", type=Path, default=RUST_PROBE)
    parser.add_argument("--baseline-writer", type=Path)
    parser.add_argument("--go", type=Path, default=GO_PROBE)
    parser.add_argument("--filter", default="")
    parser.add_argument("--budget", type=int, default=1 << 30)
    parser.add_argument("--asset-manifest", type=Path,
                        help="explicit immutable HTTP inputs; SVG source comparison, never a golden match")
    args = parser.parse_args()
    if args.asset_manifest and args.tier != "svg":
        parser.error("--asset-manifest is only supported by whole-board SVG")
    if args.tier in ("text", "rich", "primitives") and args.baseline_writer:
        parser.error("--baseline-writer is only for SVG writer tiers")
    references.verify_source()
    capture = args.capture.resolve()
    assert not capture.exists(), "use a fresh capture path"
    capture.mkdir(parents=True)
    if args.tier in ("writer", "surfaces", "text", "rich", "primitives"):
        rows = requests(capture / "rust", "mesh", args.filter)
        if not rows:
            parser.error("no boards selected; an empty capture cannot pass")
        writer = (ROOT / f"target/diagnostic/examples/isometric-{args.tier}" if args.tier in ("text", "rich", "primitives") else RUST_WRITER) if args.rust == RUST_PROBE else args.rust
        code, rows, failure = writer_tier(capture, rows, args.tier == "surfaces",
            writer, args.budget, args.baseline_writer, args.tier in ("text", "rich"), args.tier == "rich", args.tier == "primitives")
        summary = dict(collections.Counter(row["outcome"] for row in rows))
        report = {"tier": args.tier, "source_commit": references.SOURCE_COMMIT, "runs": {"go": code}, "failure": failure,
                  "rust": str(writer.resolve()), "rust_sha256": references.digest(writer),
                  "baseline_writer": str(args.baseline_writer.resolve()) if args.baseline_writer else None,
                  "baseline_sha256": references.digest(args.baseline_writer) if args.baseline_writer else None,
                  "expected_boards": len(expected_boards(requests(capture / "rust", "mesh", args.filter))),
                  "budgets": {"total": args.budget, "file": 128 << 20, "free": 20 << 30},
                  "summary": summary, "before_summary": dict(collections.Counter(row.get("before", "go-error") for row in rows)) if args.baseline_writer else None,
                  "writer_summary": dict(collections.Counter(row.get("writer", "go-error") for row in rows)), "rows": rows}
        if args.tier in ("text", "rich", "primitives"):
            report["call_summary"] = {
                "calls": sum(row.get("calls", 0) for row in rows),
                "matched_calls": sum(row.get("matched_calls", 0) for row in rows),
                "empty_boards": sum(row.get("calls") == 0 for row in rows),
                "nonempty_matches": sum(row.get("calls", 0) > 0 and row["outcome"] == "match" for row in rows),
            }
        report["capture_statistics"] = str(capture / "capture-statistics.json")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=1) + "\n")
        print(args.tier, summary, "boards", len(rows))
        return int(code != 0 or failure is not None)
    runs, failure = {}, None
    selected = requests(capture / "go", args.tier, args.filter)
    assets = asset_sources(args.asset_manifest) if args.asset_manifest else None
    if not selected:
        parser.error("no boards selected; an empty capture cannot pass")
    for engine, probe in [("go", args.go), ("rust", args.rust)]:
        rows = requests(capture / engine, args.tier, args.filter)
        if assets is not None:
            for request in rows:
                request["assetSources"] = assets
        runs[engine], failure = run(probe.resolve(), rows, capture / f"{engine}.log", capture, args.budget)
        if failure:
            break
    if assets is not None:
        assert asset_sources(args.asset_manifest) == assets, "asset inputs changed during rendering"
    rows, metadata = compare_direct(capture, selected, args.tier, runs, failure, assets is not None)
    summary = dict(collections.Counter(row["outcome"] for row in rows))
    report = {"tier": args.tier, "source_commit": references.SOURCE_COMMIT, "runs": runs,
              "failure": failure, "expected_boards": len(expected_boards(selected)),
              "rust": str(args.rust.resolve()), "rust_sha256": references.digest(args.rust),
              "budgets": {"total": args.budget, "file": 128 << 20, "free": 20 << 30},
              "metadata": metadata, "summary": summary, "rows": rows}
    if assets is not None:
        report.update(profile="explicit-assets-source-comparison", asset_sources=assets,
                      asset_manifest_sha256=references.digest(args.asset_manifest))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(args.tier, summary, "boards", len(rows))
    return int(failure is not None or any(runs.values()))


if __name__ == "__main__":
    raise SystemExit(main())
