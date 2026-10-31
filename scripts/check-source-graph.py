#!/usr/bin/env python3
"""Compare source-to-measured-graph lowering with the pinned D2 oracle.

This is deliberately a boundary check rather than an SVG check.  The Go
oracle emits d2graph's measured GraphJSON and the Rust CLI emits the shared
Graph model, so the comparator maps both representations to stable source
IDs before checking dimensions, labels, hierarchy order, shapes, and edges.
"""

import argparse
import hashlib
import json
import struct
import subprocess
from pathlib import Path

from references import ROOT, REFERENCE, MANIFEST, digest, verify_source


def bits(value):
    if value is None:
        return None
    return struct.unpack(">Q", struct.pack(">d", float(value)))[0]


def run(command, source, timeout=120):
    result = subprocess.run(
        [str(item) for item in command],
        input=source,
        capture_output=True,
        timeout=timeout,
    )
    return result.returncode, result.stdout, result.stderr


def scalar(value):
    if isinstance(value, dict):
        value = value.get("value")
    return value


def font_scalar(value):
    """Project D2's serialized font-size scalar to the Rust integer form."""
    value = scalar(value)
    if isinstance(value, str):
        try:
            return int(value)
        except ValueError:
            return value
    return value


def go_projection(document):
    objects = document.get("objects", [])
    nodes = {}
    for obj in objects:
        ident = obj.get("AbsID")
        if not ident:
            continue
        attrs = obj.get("attributes") or {}
        box = obj.get("box") or {}
        label_dims = attrs.get("labelDimensions") or {}
        style = attrs.get("style") or {}
        nodes[ident] = {
            "size": (bits(box.get("Width")), bits(box.get("Height"))),
            "label_size": (
                bits(label_dims.get("width")),
                bits(label_dims.get("height")),
            ),
            "shape": scalar(attrs.get("shape")),
            "label": scalar(attrs.get("label")),
            "font_size": font_scalar(style.get("fontSize")),
            "children": tuple(obj.get("ChildrenArray") or ()),
        }
    edges = tuple(
        (edge.get("Src"), edge.get("Dst"), edge.get("src_arrow"), edge.get("dst_arrow"))
        for edge in (document.get("edges") or [])
    )
    return {"nodes": nodes, "edges": edges, "direction": scalar((document.get("root") or {}).get("attributes", {}).get("direction"))}


def rust_projection(document):
    nodes = document.get("nodes", [])
    by_id = {str(index): node for index, node in enumerate(nodes)}
    by_name = {node.get("external_id"): (str(index), node) for index, node in enumerate(nodes)}
    children = {}
    for ident, node in by_name.values():
        parent = node.get("parent")
        children.setdefault(None if parent is None else str(parent), []).append(ident)
    projected = {}
    for ident, (numeric, node) in by_name.items():
        size = node.get("size") or {}
        label_size = node.get("label_size") or {}
        projected[ident] = {
            "size": (bits(size.get("width")), bits(size.get("height"))),
            "label_size": (
                bits(label_size.get("width")),
                bits(label_size.get("height")),
            ),
            # RealSquare is TALA's internal unit-aspect specialization; the
            # source graph boundary serializes it as the public square shape.
            "shape": "square" if str(node.get("shape", "")).lower() == "real_square" else str(node.get("shape", "")).lower(),
            # Source labels live outside Graph; graph JSON comparison only
            # qualifies their measured dimensions and leaves text to AST/target
            # boundaries.
            "label": None,
            "font_size": node.get("font_size"),
            "children": tuple(
                by_id[child]["external_id"]
                for child in children.get(numeric, ())
                if child in by_id
            ),
        }
    edges = []
    for edge in document.get("edges", []):
        source = by_id.get(str(edge.get("source")), {}).get("external_id")
        target = by_id.get(str(edge.get("target")), {}).get("external_id")
        source_arrows = document.get("edge_arrows", [])
        arrow = source_arrows[len(edges)] if len(edges) < len(source_arrows) else {}
        edges.append((source, target, arrow.get("source"), arrow.get("target")))
    return {"nodes": projected, "edges": tuple(edges), "direction": document.get("direction")}


def compare(go, rust):
    left = go_projection(go)
    right = rust_projection(rust)
    differences = []
    if left["direction"] not in (None, "") and left["direction"] != right["direction"]:
        differences.append(f"direction {left['direction']!r} != {right['direction']!r}")
    left_ids = set(left["nodes"])
    right_ids = set(right["nodes"])
    for ident in sorted(left_ids - right_ids):
        differences.append(f"missing node {ident!r}")
    for ident in sorted(right_ids - left_ids):
        differences.append(f"extra node {ident!r}")
    for ident in sorted(left_ids & right_ids):
        for field in ("size", "label_size", "shape", "font_size", "children"):
            # d2graph omits the default font size from GraphJSON. The Rust
            # graph keeps its effective default for the layout engine, so an
            # omitted Go value is semantically equal to any Rust default.
            if field == "font_size" and left["nodes"][ident][field] is None:
                continue
            if left["nodes"][ident][field] != right["nodes"][ident][field]:
                differences.append(
                    f"node {ident!r} {field}: {left['nodes'][ident][field]!r} != {right['nodes'][ident][field]!r}"
                )
    if left["edges"] != right["edges"]:
        differences.append(f"edges: {left['edges']!r} != {right['edges']!r}")
    return differences


def fixture_name(path):
    """Give reports a stable name for local and pinned-reference fixtures."""
    for base in (ROOT, ROOT.parent / "d2"):
        try:
            return str(path.relative_to(base))
        except ValueError:
            continue
    return str(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", action="append", help="compare only this path")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "oracle-cache" / "reports" / "source-graph-parity.json",
        help="report path (default: oracle-cache/reports/source-graph-parity.json)",
    )
    args = parser.parse_args()
    source = verify_source()
    oracle = REFERENCE / "bin" / "d2-oracle"
    rust = ROOT / "target" / "release" / "d2-rs"
    if not rust.exists():
        subprocess.run(["cargo", "build", "--release", "--locked", "-p", "d2-rs-cli", "--bin", "d2-rs"], cwd=ROOT, check=True)
    if args.fixture:
        paths = [Path(item).resolve() for item in args.fixture]
    else:
        # The qualification corpus is the complete pinned v0.9.0 reference
        # checkout. Local fixtures remain useful for focused runs, but using
        # them as the default made it possible to overwrite the report with a
        # narrower, misleading 33-case snapshot.
        reference_root = ROOT.parent / "d2"
        paths = sorted(reference_root.rglob("*.d2")) if reference_root.exists() else sorted(
            path
            for root in (
                ROOT / "tests" / "d2",
                ROOT / "tests" / "upstream" / "d2",
                ROOT / "crates" / "d2-rs-lib" / "tests" / "fixtures",
                ROOT / "crates" / "d2-rs-cli" / "tests" / "fixtures",
            )
            if root.exists()
            for path in root.rglob("*.d2")
        )
    rows = []
    for path in paths:
        data = path.read_bytes()
        code, out, err = run([oracle, "measure", "--path", str(path)], data, args.timeout)
        row = {"fixture": fixture_name(path), "source_sha256": hashlib.sha256(data).hexdigest()}
        if code:
            row.update({"matched": False, "kind": "oracle-error", "error": err.decode(errors="replace")})
            rows.append(row)
            continue
        try:
            go = json.loads(out)
        except json.JSONDecodeError as error:
            row.update({"matched": False, "kind": "oracle-json", "error": str(error)})
            rows.append(row)
            continue
        code, out, err = run([rust, "graph", f"--path={path}"], data, args.timeout)
        if code:
            row.update({"matched": False, "kind": "rust-error", "error": err.decode(errors="replace")})
            rows.append(row)
            continue
        try:
            rust_graph = json.loads(out)
        except json.JSONDecodeError as error:
            row.update({"matched": False, "kind": "rust-json", "error": str(error)})
            rows.append(row)
            continue
        differences = compare(go, rust_graph)
        row.update({"matched": not differences, "differences": differences})
        rows.append(row)
        if differences:
            print(row["fixture"], "DIFF", differences[0])
    report = {
        "kind": "d2-v0.9.0-source-graph-boundary",
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "oracle_sha256": digest(oracle),
        "rust_sha256": digest(rust),
        "cases": len(rows),
        "matched": sum(row.get("matched") is True for row in rows),
        "rows": rows,
        "scope": "measured node dimensions/label dimensions, shape, font size, hierarchy child order, direction, and edge identities/arrows; SVG/target serialization is separate",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("cases", "matched", "oracle_sha256", "rust_sha256")}, indent=2))
    if report["matched"] != report["cases"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
