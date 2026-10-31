#!/usr/bin/env python3
"""Run the pinned v0.9.0 TALA fuzz corpus through the native graph boundary.

The upstream records contain the seven scalar arguments to ``FuzzAutolayout``
rather than graph JSON.  The development oracle uses the source-shaped Go
generator to materialize each graph; both the Go engine and the Rust adapter
then consume that same graph.  This keeps the fixture qualification about the
engine boundary, rather than accidentally comparing two different generators.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, digest, verify_source


CORPUS = (
    verify_source()
    / "d2layouts/d2talalayout/internal/engine/testdata/fuzz/FuzzAutolayout"
)


def fuzz_seed(data: bytes) -> int:
    match = re.search(rb"^int64\((-?\d+)\)$", data, re.MULTILINE)
    if match is None:
        raise ValueError("Go fuzz record has no int64 seed")
    return int(match.group(1))


def run(
    command: list[str], data: bytes, timeout: int, environment: dict[str, str] | None = None
) -> tuple[int, bytes, bytes]:
    try:
        result = subprocess.run(
            command, input=data, capture_output=True, timeout=timeout, env=environment
        )
    except subprocess.TimeoutExpired as error:
        return 124, b"", str(error).encode()
    return result.returncode, result.stdout, result.stderr


def stage_events(stderr: bytes) -> list[dict]:
    events = []
    for line in stderr.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and value.get("event") == "stage":
            if value.get("stage") == "SelectedResult":
                continue
            value.pop("seed", None)
            events.append(value)
    return events


def point(value: dict | None) -> tuple[float, float] | None:
    if value is None:
        return None
    return (value["x"], value["y"])


def normalized(document: dict) -> dict:
    nodes = {
        node["id"]: (
            node["width"],
            node["height"],
            point(node.get("topLeft", node.get("top_left"))),
        )
        for node in document.get("nodes", [])
    }
    edges = {
        edge["id"]: (
            edge["from"],
            edge["to"],
            [point(item) for item in edge.get("points", [])],
        )
        for edge in document.get("edges", [])
    }
    return {"nodes": nodes, "edges": edges}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument(
        "--additional-corpus",
        action="append",
        type=Path,
        default=[],
        help="extra Go fuzz-record directory, kept separate from the pinned upstream corpus",
    )
    parser.add_argument(
        "--report", type=Path, default=ROOT / "oracle-cache/reports/go-fuzz-corpus-parity.json"
    )
    args = parser.parse_args()

    source = verify_source()
    go = REFERENCE / "bin/tala-engine-oracle-trace"
    rust_candidates = (
        ROOT / "target/diagnostic/examples/graphjson-layout",
        ROOT / "target/oracle/examples/graphjson-layout",
        ROOT / "target/debug/examples/graphjson-layout",
    )
    rust = next((candidate for candidate in rust_candidates if candidate.exists()), None)
    if not go.exists() or rust is None:
        raise SystemExit("missing trace oracle or graphjson-layout example")
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("TALA_", "WEFTAN_"))
    }
    go_environment = {**environment, "TALA_TRACE_JSONL": "1"}
    rust_environment = {**environment, "WEFTAN_TRACE_JSONL": "1"}

    corpora = [(CORPUS, "pinned upstream fuzz corpus")]
    for additional in args.additional_corpus:
        corpus_path = additional if additional.is_absolute() else ROOT / additional
        corpus_path = corpus_path.resolve()
        if not corpus_path.is_dir():
            parser.error(f"additional corpus directory does not exist: {corpus_path}")
        corpora.append((corpus_path, "local deterministic expansion"))

    rows = []
    for corpus_path, corpus_origin in corpora:
        for path in sorted(entry for entry in corpus_path.iterdir() if entry.is_file()):
            data = path.read_bytes()
            seed = fuzz_seed(data)
            go_graph_exit, go_graph, go_graph_error = run(
                [str(go), "-operation", "fuzz"], data, args.timeout
            )
            try:
                fixture = str(path.relative_to(source))
            except ValueError:
                fixture = str(path.relative_to(ROOT))
            row = {
                "fixture": fixture,
                "corpus_origin": corpus_origin,
                "input_sha256": hashlib.sha256(data).hexdigest(),
                "seed": seed,
                "go_graph_exit": go_graph_exit,
            }
            if go_graph_exit != 0:
                row.update({"passed": False, "go_graph_error": go_graph_error.decode(errors="replace")})
                rows.append(row)
                continue
            row["graph_sha256"] = hashlib.sha256(go_graph).hexdigest()
            go_exit, go_output, go_error = run(
                [str(go), "-operation", "layout", "-seed", str(seed)],
                go_graph,
                args.timeout,
                go_environment,
            )
            rust_exit, rust_output, rust_error = run(
                [str(rust), str(seed)], go_graph, args.timeout, rust_environment
            )
            go_trace = stage_events(go_error)
            rust_trace = stage_events(rust_error)
            row.update(
                {
                    "go_exit": go_exit,
                    "rust_exit": rust_exit,
                    "go_stage_events": len(go_trace),
                    "rust_stage_events": len(rust_trace),
                    "stage_trace_match": go_trace == rust_trace and bool(go_trace),
                }
            )
            if go_exit == 0 and rust_exit == 0:
                try:
                    go_value = normalized(json.loads(go_output))
                    rust_value = normalized(json.loads(rust_output))
                except (json.JSONDecodeError, KeyError, TypeError) as error:
                    row.update({"passed": False, "error": f"JSON decode: {error}"})
                else:
                    row["passed"] = go_value == rust_value and row["stage_trace_match"]
                    if not row["passed"]:
                        row["go_sha256"] = hashlib.sha256(go_output).hexdigest()
                        row["rust_sha256"] = hashlib.sha256(rust_output).hexdigest()
            else:
                row.update(
                    {
                        "passed": False,
                        "go_error": go_error.decode(errors="replace"),
                        "rust_error": rust_error.decode(errors="replace"),
                    }
                )
            rows.append(row)
            print(f"{fixture}: {'PASS' if row['passed'] else 'FAIL'}", flush=True)

    report = {
        "kind": "d2-v0.9.0-tala-go-fuzz-corpus-parity",
        "scope": "pinned and explicitly listed Go FuzzAutolayout records materialized by the source-shaped generator and compared through graphjson layout plus normalized stage traces (SelectedResult is Rust-only metadata)",
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "go_oracle_sha256": digest(go),
        "rust_binary_sha256": digest(rust),
        "corpora": [
            {"path": str(path), "origin": origin, "records": sum(1 for entry in path.iterdir() if entry.is_file())}
            for path, origin in corpora
        ],
        "passed": sum(row.get("passed", False) for row in rows),
        "failed": sum(not row.get("passed", False) for row in rows),
        "total": len(rows),
        "results": rows,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("passed", "failed", "total")}, indent=2))
    return int(report["failed"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())
