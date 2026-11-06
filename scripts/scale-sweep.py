#!/usr/bin/env python3
"""Measure generated D2 families against pinned Go as their size increases.

Run with uv run --no-project python scripts/scale-sweep.py. Defaults cover
25..800 nodes, all three SVG engines, and Dagre PNG/PDF, with three
interleaved samples. Filter --families/--sizes/--engines/--formats for a
bounded investigation. Generated inputs and retained outputs live beside
the JSON report under oracle-cache/reports/scale-sweep, never in the tree.
PDF output is measured but is not compared with Go (embedded timestamps).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import resource
import signal
import stat
import statistics
import struct
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
import zlib

import references

FAMILIES = ("chain", "fanout", "grid", "sparse", "dense", "nested", "sequence", "sql", "long-label")
RASTER_FAMILIES = ("markdown", "emoji", "opacity", "gradient", "code-block", "shadow", "labeled-edges")
ENGINES = ("dagre", "elk", "tala")
FORMATS = ("svg", "png", "pdf")
SIZES = (25, 50, 100, 200, 400, 800)
MIB = 1024 * 1024


def generate(family, size, seed=20261104):
    """Size counts nodes (actors or tables for sequence/SQL, including containers)."""
    # Raster stress knobs count list items/code lines or painted nodes/edges.
    # These opt-in families leave the original default queue unchanged.
    if family == "markdown":
        body = "\n".join(f"  - item **{i}** with `code` and _emphasis_" for i in range(size))
        return f"doc: |md\n  # Heading\n{body}\n|\nother -> doc"
    if family == "code-block":
        body = "\n".join(f"  func f{i}(x int) int {{ return x * {i} }}" for i in range(size))
        return f"src: |go\n{body}\n|\nsrc -> out"
    if family == "labeled-edges":
        return "hub\n" + "\n".join(f"hub -> leaf{i}: edge {i}" for i in range(size))
    if family in ("emoji", "opacity", "gradient", "shadow"):
        lines = [f"grid-columns: {max(2, math.isqrt(size))}"]
        faces = "😀😃😄😁😆🙂🙃😉😊😇🥰😍🤩😘😗☺😚😙🥲😋"
        style = {"opacity": "opacity: 0.4", "gradient": 'fill: "linear-gradient(#f69d3c, #3f87a6)"',
                 "shadow": "shadow: true"}.get(family)
        for i in range(size):
            label = f"{faces[i % len(faces)]} cell {i} ✨" if family == "emoji" else f"cell {i}"
            lines.append(f"c{i}: {label}")
            if style:
                lines.append(f"c{i}.style: {{{style}}}")
        return "\n".join(lines)
    rng = random.Random(f"{seed}:{family}:{size}")
    nodes = [f"n{i}: Node {i}" for i in range(size)]
    if family == "grid":
        return f"grid-columns: {math.ceil(math.sqrt(size))}\n" + "\n".join(nodes) + "\n"
    if family == "nested":
        # Go rejects syntax nested past its parser limit. Keep growing the
        # leaf inventory after reaching 96 containers, below that limit.
        depth = min(size // 2, 96)
        lines = ["  " * i + f"c{i}: {{" for i in range(depth)]
        lines += ["  " * depth + line for line in nodes[:size - depth]]
        lines += ["  " * i + "}" for i in reversed(range(depth))]
        return "\n".join(lines) + "\n"
    if family == "sequence":
        lines = ["shape: sequence_diagram", *nodes]
        lines += [f"n{i % size} -> n{(i + 1) % size}: Message {i}" for i in range(3 * size)]
        return "\n".join(lines) + "\n"
    if family == "sql":
        lines = []
        for i in range(size):
            lines += [f"n{i}: {{", "  shape: sql_table", "  id: int {constraint: primary_key}"]
            lines += [f"  c{j}: int" for j in range(15)]
            lines += ["}"]
        lines += [f"n{i}.c0 -> n{i + 1}.id" for i in range(size - 1)]
        return "\n".join(lines) + "\n"
    if family == "long-label":
        nodes = [f'n{i}: "Node {i}: ' + "a deliberately long label " * 8 + '"' for i in range(size)]
    if family in ("chain", "long-label"):
        edges = [(i, i + 1) for i in range(size - 1)]
    elif family == "fanout":
        edges = [((i - 1) // 8, i) for i in range(1, size)]
    elif family in ("sparse", "dense"):
        pairs = [(i, j) for i in range(size) for j in range(i + 1, size)]
        count = min(len(pairs), 2 * size if family == "sparse" else size * size // 8)
        edges = sorted(rng.sample(pairs, count))
    else:
        raise ValueError(f"unknown family: {family}")
    return "\n".join([*nodes, *(f"n{i} -> n{j}" for i, j in edges)]) + "\n"


def inventory(directory):
    files = {}
    for path in directory.rglob("*"):
        # The monitor runs while the CLI atomically renames staged outputs.
        # A name returned by rglob may disappear before stat; one stat also
        # avoids the is_file/stat race on the same entry.
        try:
            info = path.stat()
        except FileNotFoundError:
            continue
        if stat.S_ISREG(info.st_mode):
            files[str(path.relative_to(directory))] = info.st_size
    return files


def validate_output(path, fmt):
    """Reject truncated captures; validate PNG scanlines without retaining pixels."""
    if fmt == "svg":
        root = ET.parse(path).getroot()
        if root.tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError("missing SVG root")
        return
    if fmt == "pdf":
        with path.open("rb") as stream:
            if stream.read(5) != b"%PDF-":
                raise ValueError("missing PDF header")
            stream.seek(max(0, path.stat().st_size - 100))
            if b"%%EOF" not in stream.read():
                raise ValueError("missing PDF trailer")
        return
    decoder = zlib.decompressobj()
    decoded = 0
    stride = expected = None
    seen_idat = False
    with path.open("rb") as stream:
        if stream.read(8) != b"\x89PNG\r\n\x1a\n":
            raise ValueError("missing PNG signature")
        while True:
            header = stream.read(8)
            if len(header) != 8:
                raise ValueError("missing PNG trailer")
            length, kind = struct.unpack(">I4s", header)
            if length > path.stat().st_size - stream.tell() - 4:
                raise ValueError("truncated PNG chunk")
            payload = stream.read(length)
            if stream.read(4) != struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF):
                raise ValueError("bad PNG CRC")
            if stride is None and kind != b"IHDR":
                raise ValueError("missing PNG header")
            if kind == b"IHDR":
                if stride is not None or length != 13:
                    raise ValueError("invalid PNG header")
                width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
                if not width or not height or (depth, color, compression, filtering, interlace) != (8, 2, 0, 0, 0):
                    raise ValueError("expected nonempty opaque RGB PNG")
                stride, expected = width * 3 + 1, height * (width * 3 + 1)
            elif kind == b"IDAT":
                seen_idat = True
                pending = payload
                while pending:
                    block = decoder.decompress(pending, 65536)
                    if any(value > 4 for value in block[(-decoded) % stride::stride]):
                        raise ValueError("invalid PNG scanline filter")
                    decoded += len(block)
                    if decoded > expected or decoder.unused_data:
                        raise ValueError("trailing PNG scanline data")
                    pending = decoder.unconsumed_tail
            elif kind == b"IEND":
                if length or not seen_idat or not decoder.eof or decoded != expected or stream.read(1):
                    raise ValueError("incomplete PNG stream")
                return


def measure_child(request):
    """One fresh worker per CLI: ru_maxrss must not carry over from other runs."""
    directory = Path(request["directory"])
    limit = min(request["file_limit"], request["remaining"])

    # Set the inherited limit in the worker, keeping Popen's vfork/exec
    # fast path. preexec_fn would fork the Python heap into the measured
    # child and inflate short CLI runs' CPU and ru_maxrss.
    resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    stopped = None
    start = time.perf_counter()
    with (directory / "stderr.txt").open("wb") as stderr:
        process = subprocess.Popen(request["command"], stdout=subprocess.DEVNULL, stderr=stderr,
                                   start_new_session=True)

        def monitor(_signal, _frame):
            nonlocal stopped
            if stopped:
                return
            if time.perf_counter() - start >= request["timeout"]:
                stopped = "timeout"
            elif sum(inventory(directory).values()) >= request["remaining"]:
                stopped = "output-budget"
            if stopped:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

        signal.signal(signal.SIGALRM, monitor)
        signal.setitimer(signal.ITIMER_REAL, 0.05, 0.05)
        try:
            # Blocking wait4 measures short CLI runs without Popen.wait's
            # timeout polling/rounding, and returns this child's own usage.
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        except BaseException:
            # Cancellation must not leave a long-running CLI behind.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
        wall = time.perf_counter() - start
    files = inventory(directory)
    if sum(files.values()) >= request["remaining"] or any(size >= limit for size in files.values()) or process.returncode == -signal.SIGXFSZ:
        stopped = "output-budget"
    with (directory / "stderr.txt").open("rb") as stream:
        excerpt = stream.read(8000).decode(errors="replace")
        stream.seek(0)
        stderr_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    return {"exit": process.returncode, "outcome": stopped or ("ok" if process.returncode == 0 else "error"),
            "wall": wall, "cpu": usage.ru_utime + usage.ru_stime,
            "rss": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
            "bytes": sum(files.values()), "stderr": excerpt, "stderr_hash": stderr_hash}


def run_sample(binary, engine, fmt, source, directory, args, remaining):
    directory.mkdir()
    output = directory / f"out.{fmt}"
    # Let the outer limit fire first; override the CLI's 120 s default.
    command = [str(binary), f"--layout={engine}", f"--timeout={math.ceil(args.timeout) + 1}",
               str(source), str(output)]
    request = {"command": command, "directory": str(directory), "timeout": args.timeout,
               "remaining": remaining, "file_limit": args.file_mib * MIB}
    environment = {k: v for k, v in os.environ.items()
                   if not k.startswith(("D2_", "D2RS_", "TALA_TRACE_"))}
    # The worker is outside the timed interval and cannot contaminate the
    # CLI's CPU/RSS. CLI descendants belong to a group killed on timeout.
    worker = None
    try:
        worker = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--measure-child", json.dumps(request)],
                                env=environment, capture_output=True, text=True)
        if worker.returncode:
            raise ValueError(f"measurement worker exited {worker.returncode}")
        sample = json.loads(worker.stdout)
        if (not isinstance(sample, dict) or
                not {"exit", "outcome", "wall", "cpu", "rss", "bytes", "stderr", "stderr_hash"} <= sample.keys()):
            raise ValueError("incomplete measurement worker response")
    except (OSError, ValueError, TypeError) as error:
        # A worker failure is a failed sample, never an oracle rejection or
        # an excuse to discard this row and abort the rest of the sweep.
        sample = {"exit": None, "outcome": "worker-failed", "wall": None, "cpu": None, "rss": None,
                  "bytes": sum(inventory(directory).values()), "stderr": "", "stderr_hash": None,
                  "worker_exit": worker.returncode if worker is not None else None, "worker_error": str(error),
                  "worker_stderr": worker.stderr[:8000] if worker is not None else "",
                  "worker_stdout": worker.stdout[:8000] if worker is not None else ""}
    sample["hash"] = None
    sample["output_bytes"] = output.stat().st_size if output.is_file() else 0
    if sample["outcome"] == "ok":
        if not sample["output_bytes"]:
            sample["outcome"] = "missing-output"
        else:
            try:
                validate_output(output, fmt)
                if set(inventory(directory)) != {output.name, "stderr.txt"}:
                    raise ValueError("unexpected output inventory")
                with output.open("rb") as stream:
                    sample["hash"] = hashlib.file_digest(stream, "sha256").hexdigest()
            except (OSError, ValueError, ET.ParseError, zlib.error) as error:
                sample["outcome"] = "invalid-output"
                sample["validation_error"] = str(error)
    return sample


def finish_row(row):
    for impl in ("go", "rust"):
        samples = row[impl]["samples"]
        row[impl].update({key: statistics.median(s[key] for s in samples) if samples and all(s[key] is not None for s in samples) else None
                          for key in ("wall", "cpu", "rss")})
    go, rust = row["go"]["samples"], row["rust"]["samples"]
    if any(s["outcome"] == "output-budget" for s in go + rust):
        status = "output-budget"
    elif any(s["outcome"] == "worker-failed" for s in go + rust):
        status = "worker-failed"
    elif any(s["outcome"] == "timeout" for s in go):
        status = "skip-go-timeout"
    elif go and any(s["outcome"] == "error" for s in go):
        if len(go) == len(rust) and all(s["outcome"] == "error" for s in go + rust):
            status = ("skip-go-error" if all((a["exit"], a.get("stderr_hash", a["stderr"])) ==
                                            (b["exit"], b.get("stderr_hash", b["stderr"])) for a, b in zip(go, rust))
                      else "parity-error-mismatch")
        else:
            status = "parity-exit-mismatch" if rust else "incomplete"
    elif any(s["outcome"] != "ok" for s in go):
        status = "go-failed"
    elif any(s["outcome"] != "ok" for s in rust):
        status = "rust-failed"
    elif len(go) != row["repeat"] or len(rust) != row["repeat"]:
        status = "incomplete"
    else:
        status = "measured"
    # Compare every sample. A later match must not hide a mismatch or an
    # intermittently missing/corrupt file. PDFs are explicitly incomparable.
    row["identical"] = None
    if row["format"] != "pdf":
        row["identical"] = (status == "measured" and len({s["hash"] for s in go + rust}) == 1
                            and go[0]["hash"] is not None)
        if status == "measured" and not row["identical"]:
            status = "parity-mismatch"
    for key in ("wall", "cpu", "rss"):
        row[key + "_ratio"] = (row["rust"][key] / row["go"][key]
                                if go and rust and row["go"][key] and row["rust"][key] is not None else None)
    row["status"] = status
    row["over_go"] = (status in ("measured", "parity-mismatch") and
                      (row["wall_ratio"] > 1 or row["rss_ratio"] > 1))
    return row


def trends(rows):
    result = []
    groups = sorted({(r["family"], r["engine"], r["format"]) for r in rows})
    for family, engine, fmt in groups:
        group = sorted((r for r in rows if (r["family"], r["engine"], r["format"]) == (family, engine, fmt)
                        and r["status"] in ("measured", "parity-mismatch")), key=lambda r: r["size"])
        if len(group) < 2:
            continue
        first, last = group[0], group[-1]
        growth = {key: last[key + "_ratio"] / first[key + "_ratio"] for key in ("wall", "cpu", "rss")}
        intervals = [{"sizes": [a["size"], b["size"]],
                      "relative_growth": {key: b[key + "_ratio"] / a[key + "_ratio"] for key in ("wall", "cpu", "rss")}}
                     for a, b in zip(group, group[1:])]
        result.append({"family": family, "engine": engine, "format": fmt,
                       "sizes": [first["size"], last["size"]], "relative_growth": growth,
                       "intervals": intervals,
                       "rising": any(i["relative_growth"][key] > 1 for i in intervals for key in ("wall", "rss"))})
    return result


def table(rows):
    sizes = sorted({r["size"] for r in rows})
    lines = ["Cells: Rust/Go wall / CPU / RSS; ! output mismatch; PDF bytes not compared.", "",
             "| family | engine/format | " + " | ".join(map(str, sizes)) + " |",
             "|---|---|" + "---|" * len(sizes)]
    for family, engine, fmt in sorted({(r["family"], r["engine"], r["format"]) for r in rows}):
        selected = {r["size"]: r for r in rows if (r["family"], r["engine"], r["format"]) == (family, engine, fmt)}
        cells = []
        for size in sizes:
            r = selected.get(size)
            if r is None:
                cell = "—"
            elif r["status"] not in ("measured", "parity-mismatch"):
                cell = r["status"]
            else:
                cell = " / ".join(f"{r[key + '_ratio']:.2f}" for key in ("wall", "cpu", "rss"))
                if r["status"] == "parity-mismatch":
                    cell += " !"
            cells.append(cell)
        lines.append(f"| {family} | {engine}/{fmt} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def binary_record(binary):
    with binary.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    return {"path": str(binary), "sha256": digest, "version": version}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rust", type=Path, default=references.ROOT / "target/release/d2-rs")
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--output", type=Path, default=references.REPORTS / "scale-sweep/scale.json")
    parser.add_argument("--families", default=",".join(FAMILIES), help="original families or opt-in raster stress: " + ",".join(RASTER_FAMILIES))
    parser.add_argument("--sizes", default=",".join(map(str, SIZES)))
    parser.add_argument("--engines", default=",".join(ENGINES))
    parser.add_argument("--formats", default=",".join(FORMATS), help="PNG/PDF run only with Dagre")
    parser.add_argument("--seed", type=int, default=20261104)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--budget-mib", type=int, default=1024, help="cumulative input/output capture budget")
    parser.add_argument("--file-mib", type=int, default=128)
    parser.add_argument("--max-jobs", type=int, default=None)
    args = parser.parse_args()
    selections = {}
    for key, choices in (("families", FAMILIES + RASTER_FAMILIES), ("engines", ENGINES), ("formats", FORMATS)):
        selections[key] = getattr(args, key).split(",")
        if not selections[key] or set(selections[key]) - set(choices) or len(set(selections[key])) != len(selections[key]):
            parser.error(f"--{key} must select unique members of {','.join(choices)}")
    try:
        sizes = sorted(set(map(int, args.sizes.split(","))))
    except ValueError:
        parser.error("--sizes must contain positive integers")
    if not sizes or min(sizes) < 2 or min(args.repeat, args.timeout, args.budget_mib, args.file_mib) <= 0:
        parser.error("sizes must be at least 2; repeat, timeout and budgets must be positive")
    if args.max_jobs is not None and args.max_jobs < 1:
        parser.error("--max-jobs must be positive")
    binaries = {impl: path.resolve() for impl, path in (("go", args.go), ("rust", args.rust))}
    records = {impl: binary_record(path) for impl, path in binaries.items()}
    metadata = subprocess.check_output(["go", "version", "-m", str(binaries["go"])], text=True)
    arch = {"aarch64": "arm64", "arm64": "arm64", "x86_64": "amd64", "AMD64": "amd64"}.get(platform.machine())
    for expected in (references.MANIFEST["d2"]["go_toolchain"], f"vcs.revision={references.SOURCE_COMMIT}", "vcs.modified=false", f"GOARCH={arch}"):
        if expected not in metadata:
            parser.error(f"Go oracle metadata lacks {expected}")
    expected_version = references.MANIFEST["d2"]["base_tag"]
    if records["go"]["version"] != expected_version:
        parser.error(f"Go oracle must have the manifest's {expected_version} version stamp")
    args.output = args.output.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Artifacts may be in the ignored cache or outside the repository.
    if args.output.is_relative_to(references.ROOT) and not args.output.is_relative_to(references.ROOT / "oracle-cache"):
        parser.error("put reports under oracle-cache/ or outside the repository")
    work = Path(tempfile.mkdtemp(prefix=args.output.stem + "-", dir=args.output.parent))
    report = {"source_commit": references.SOURCE_COMMIT, "rust_commit": references.command("git", "rev-parse", "HEAD", cwd=references.ROOT),
              "rust_dirty": bool(references.command("git", "status", "--porcelain", cwd=references.ROOT)),
              "host": platform.platform(), "architecture": arch, "binaries": records,
              "python": sys.version,
              "settings": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
              "artifacts": str(work), "measurement": "isolated wait4; wall/CPU seconds; peak RSS bytes; interleaved alternating order",
              "bytes_captured": 0, "rows": [], "trends": [], "complete": False}
    jobs = [(family, size, engine, fmt) for family in selections["families"] for size in sizes
            for engine in selections["engines"] for fmt in selections["formats"] if fmt == "svg" or engine == "dagre"]
    if not jobs:
        parser.error("no jobs selected (PNG/PDF require Dagre)")
    if args.max_jobs:
        jobs = jobs[:args.max_jobs]

    def save():
        report["trends"] = trends(report["rows"])
        temporary = args.output.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(report, indent=1) + "\n")
        temporary.replace(args.output)
        args.output.with_suffix(".md").write_text(table(report["rows"]))

    try:
        for family, size, engine, fmt in jobs:
            directory = work / f"{family}-{size}-{engine}-{fmt}"
            directory.mkdir()
            source = directory / "input.d2"
            script = generate(family, size, args.seed)
            if report["bytes_captured"] + len(script.encode()) >= args.budget_mib * MIB:
                report["stop_reason"] = "output-budget"
                break
            source.write_text(script)
            report["bytes_captured"] += source.stat().st_size
            row = {"family": family, "size": size, "engine": engine, "format": fmt,
                   "source": str(source), "repeat": args.repeat, "go": {"samples": []}, "rust": {"samples": []}}
            stop = False
            for repeat in range(args.repeat):
                order = ("go", "rust") if repeat % 2 == 0 else ("rust", "go")
                for impl in order:
                    remaining = args.budget_mib * MIB - report["bytes_captured"]
                    if remaining <= 0:
                        stop = True
                        break
                    sample = run_sample(binaries[impl], engine, fmt, source, directory / f"{impl}-{repeat}", args, remaining)
                    report["bytes_captured"] += sample["bytes"]
                    row[impl]["samples"].append(sample)
                    if sample["outcome"] == "worker-failed":
                        stop = True
                        break
                    if sample["outcome"] == "output-budget" or (impl == "go" and sample["outcome"] == "timeout"):
                        stop = True
                        break
                # Still run Rust for Go rejections: accepting a board Go's
                # raster budget rejects is an exit-status parity bug.
                if any(s["outcome"] != "ok" for s in row["go"]["samples"]):
                    stop = True
                if stop:
                    break
            finish_row(row)
            report["rows"].append(row)
            save()
            print(f"{len(report['rows'])}/{len(jobs)} {family}/{size} {engine}/{fmt}: {row['status']}"
                  + (f" wall {row['wall_ratio']:.2f} CPU {row['cpu_ratio']:.2f} RSS {row['rss_ratio']:.2f}" if row['wall_ratio'] is not None else ""), flush=True)
            if row["status"] == "output-budget" or report["bytes_captured"] >= args.budget_mib * MIB:
                report["stop_reason"] = "output-budget"
                break
        else:
            report["complete"] = True
    finally:
        save()
    print(table(report["rows"]), end="")
    failed = any(r["over_go"] or r["status"] not in ("measured", "skip-go-timeout", "skip-go-error") for r in report["rows"])
    return int(not report["complete"] or failed or any(t["rising"] for t in report["trends"]))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--measure-child":
        print(json.dumps(measure_child(json.loads(sys.argv[2]))))
    else:
        sys.exit(main())
