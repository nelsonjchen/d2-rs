#!/usr/bin/env python3
"""Compare Dagre shape and route geometry with the pinned D2 v0.9.0 binary.

The SVG writers intentionally use different metadata, fonts, and serialization
styles. This harness compares the layout boundary: root viewBox, shape
geometry, and routes keyed by semantic edge identity, after stable numeric
normalization. It is diagnostic evidence for the in-process Dagre adapter;
it does not treat renderer byte differences as layout failures.
"""

from __future__ import annotations

import argparse
import base64
import binascii
from collections import Counter
import hashlib
import html
import json
import math
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = (
    "direction-right",
    "nested-left-cluster",
    "nested-left-cluster-external-edge",
    "nested-compound-external-edge-ranks",
    "nested-order",
    "hierarchy-external-source",
    "grid-routing",
    "class-shapes",
    "cloud-shaped-grid",
    "shaped-grid-positioning",
    "benchmark-medium",
    "features",
    "fixed-container",
    "near",
    "nested-lock",
    "right-near",
    "right-top-left",
    "routing",
    "top-left",
    "all_shapes_link",
    "nested-direction-grid",
    "sequence-trace-order",
    "tala-loop-router-work",
    "upstream-chess-dia",
)


def number(value: str) -> float | str:
    try:
        return round(float(value), 6)
    except ValueError:
        return value


def geometry(path: Path) -> tuple[str | None, list[tuple[str, tuple[tuple[str, object], ...]]]]:
    root = ET.parse(path).getroot()
    shapes: list[tuple[str, tuple[tuple[str, float | str], ...]]] = []
    parents = {child: parent for parent in root.iter() for child in parent}
    for element in root.iter():
        if element.attrib.get("class") != "shape":
            continue

        def visit(child: ET.Element) -> None:
            tag = child.tag.rsplit("}", 1)[-1]
            if tag in {"defs", "mask", "clipPath", "marker", "svg"}:
                return
            if tag in {"rect", "ellipse", "polygon", "path"}:
                attrs = child.attrib
                # Compare path coordinates as well as primitive presence.
                # Earlier this treated paths as empty boxes, which allowed
                # path-based shapes to move while the fixture still passed.
                # Convert polygons to the same command representation so
                # D2's diamond path and d2-rs's polygon can be compared.
                shape_tag = "path" if tag == "polygon" else tag
                if tag == "path":
                    values = (("d", normalized_path(attrs.get("d", ""))),)
                elif tag == "polygon":
                    coordinates = [float(value) for value in _PATH_TOKEN.findall(attrs.get("points", ""))]
                    if len(coordinates) % 2:
                        raise ValueError(f"polygon has an odd coordinate count: {attrs.get('points', '')!r}")
                    commands = [("M", tuple(coordinates[:2]))] if coordinates else []
                    commands.extend(
                        ("L", tuple(coordinates[index : index + 2]))
                        for index in range(2, len(coordinates), 2)
                    )
                    if commands:
                        commands.append(("Z", ()))
                    values = (("d", tuple(commands)),)
                else:
                    keys = ("x", "y", "width", "height", "cx", "cy", "rx", "ry")
                    values = tuple((key, number(attrs[key])) for key in keys if key in attrs)
                shapes.append((shape_tag, values))
                return
            # D2 applies shadows directly to the shape group, while d2-rs
            # nests the primitive in a filter group. Descend ordinary groups
            # but exclude SVG definitions and imported subdocuments above.
            if tag == "g":
                for nested in child:
                    visit(nested)

        for child in element:
            visit(child)

    # D2 wraps code-label backgrounds in light-code/dark-code groups rather
    # than a regular `g.shape`; compare the light variant once with Rust's
    # direct shape box and fold the group translation into its coordinates.
    for code_group in root.iter():
        if "light-code" not in code_group.attrib.get("class", "").split():
            continue
        background = next(
            (
                child
                for child in code_group.iter()
                if child.tag.rsplit("}", 1)[-1] == "rect"
                and "shape" in child.attrib.get("class", "").split()
            ),
            None,
        )
        if background is None:
            continue
        translate_x = translate_y = 0.0
        current = background
        while True:
            transform = current.attrib.get("transform")
            if transform:
                match = _SVG_TRANSLATE.fullmatch(transform)
                if match is None:
                    raise ValueError(f"unsupported code-label transform: {transform!r}")
                translate_x += float(match.group(1))
                translate_y += float(match.group(2) or 0)
            if current is code_group:
                break
            parent = parents.get(current)
            if parent is None:
                raise ValueError("code-label background is detached from its light-code group")
            current = parent
        values = (
            ("x", round(float(background.attrib.get("x", 0)) + translate_x, 6)),
            ("y", round(float(background.attrib.get("y", 0)) + translate_y, 6)),
            ("width", number(background.attrib["width"])),
            ("height", number(background.attrib["height"])),
        )
        shapes.append(("rect", values))
    return root.attrib.get("viewBox"), sorted(shapes, key=repr)


_PATH_TOKEN = re.compile(r"[A-Za-z]|[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?")
_SVG_NUMBER = r"[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?"
_SVG_TRANSLATE = re.compile(
    rf"^\s*translate\(\s*({_SVG_NUMBER})(?:[,\s]+({_SVG_NUMBER}))?\s*\)\s*$"
)
_PATH_ARITY = {"M": 2, "L": 2, "T": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "A": 7}


def normalized_path(data: str) -> tuple[tuple[str, tuple[float, ...]], ...]:
    """Normalize SVG path commands to absolute coordinates and six decimals."""
    tokens = _PATH_TOKEN.findall(data)
    commands: list[tuple[str, tuple[float, ...]]] = []
    command: str | None = None
    values: list[float] = []
    cursor_x = cursor_y = start_x = start_y = 0.0
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if token.isalpha():
            command = token
            index += 1
            if command.upper() == "Z":
                commands.append(("Z", ()))
                cursor_x, cursor_y = start_x, start_y
                command = None
            continue
        if command is None:
            raise ValueError(f"SVG path has a number without a command: {data!r}")
        upper = command.upper()
        arity = _PATH_ARITY.get(upper)
        if arity is None:
            raise ValueError(f"unsupported SVG path command {command!r}")
        if index + arity > len(tokens) or any(part.isalpha() for part in tokens[index : index + arity]):
            raise ValueError(f"incomplete SVG path command {command!r}: {data!r}")
        raw = [float(part) for part in tokens[index : index + arity]]
        index += arity
        relative = command.islower()
        normalized_command = upper
        if upper in {"M", "L", "T"}:
            x, y = raw
            if relative:
                x += cursor_x
                y += cursor_y
            cursor_x, cursor_y = x, y
            normalized = (x, y)
            if upper == "M":
                start_x, start_y = x, y
                # SVG treats additional pairs after moveto as lineto.
                command = "l" if relative else "L"
        elif upper == "H":
            x = raw[0] + cursor_x if relative else raw[0]
            cursor_x = x
            normalized_command = "L"
            normalized = (x, cursor_y)
        elif upper == "V":
            y = raw[0] + cursor_y if relative else raw[0]
            cursor_y = y
            normalized_command = "L"
            normalized = (cursor_x, y)
        elif upper in {"C", "S", "Q"}:
            normalized_values = raw[:]
            if relative:
                for coordinate in range(0, len(normalized_values), 2):
                    normalized_values[coordinate] += cursor_x
                    normalized_values[coordinate + 1] += cursor_y
            cursor_x, cursor_y = normalized_values[-2:]
            normalized = tuple(normalized_values)
        else:  # Arc: radii/rotation/flags are invariant; only the endpoint moves.
            x, y = raw[-2:]
            if relative:
                x += cursor_x
                y += cursor_y
            cursor_x, cursor_y = x, y
            normalized = (*raw[:5], x, y)
        commands.append((normalized_command, tuple(round(value, 6) for value in normalized)))
    return tuple(commands)


def semantic_edge_id(ancestors: list[ET.Element]) -> str | None:
    for ancestor in reversed(ancestors):
        if ancestor.tag.rsplit("}", 1)[-1] != "g":
            continue
        candidates = [ancestor.attrib.get("id", ""), *ancestor.attrib.get("class", "").split()]
        for raw_id in candidates:
            if not raw_id:
                continue
            try:
                decoded = html.unescape(base64.b64decode(raw_id + "=" * (-len(raw_id) % 4), validate=True).decode("utf-8"))
            except (binascii.Error, UnicodeDecodeError):
                continue
            if "->" in decoded or "<->" in decoded or "--" in decoded:
                return decoded
    return None


def is_sequence_lifeline_id(edge_id: str) -> bool:
    """Recognize D2's synthetic `(actor -- )[index]` lifeline IDs only."""
    return re.fullmatch(r"\(.+ -- \)\[\d+\]", edge_id) is not None


def connection_geometry(path: Path) -> dict:
    """Return routes keyed by the edge identity shared by both SVG writers.

    D2 emits sequence lifelines as paths while d2-rs can emit them as lines;
    the semantic group id makes those two encodings comparable.
    """
    root = ET.parse(path).getroot()
    routes: dict[str, list[tuple[tuple[str, tuple[float, ...]], ...]]] = {}
    lifelines: list[tuple[tuple[str, tuple[float, ...]], ...]] = []

    def visit(element: ET.Element, ancestors: list[ET.Element], inside_marker: bool = False) -> None:
        tag = element.tag.rsplit("}", 1)[-1]
        classes = set(element.attrib.get("class", "").split())
        edge_id = semantic_edge_id(ancestors)
        route = None
        if not inside_marker and edge_id is not None and tag == "path" and "connection" in classes:
            route = normalized_path(element.attrib.get("d", ""))
            if is_sequence_lifeline_id(edge_id):
                lifelines.append(route)
                route = None
        elif not inside_marker and edge_id is not None and tag == "line" and "stroke-dasharray" in element.attrib:
            route = (
                ("M", (round(float(element.attrib["x1"]), 6), round(float(element.attrib["y1"]), 6))),
                ("L", (round(float(element.attrib["x2"]), 6), round(float(element.attrib["y2"]), 6))),
            )
            lifelines.append(route)
            route = None
        elif not inside_marker and tag == "line" and "stroke-dasharray" in element.attrib:
            route = (
                ("M", (round(float(element.attrib["x1"]), 6), round(float(element.attrib["y1"]), 6))),
                ("L", (round(float(element.attrib["x2"]), 6), round(float(element.attrib["y2"]), 6))),
            )
            lifelines.append(route)
            route = None
        if route is not None:
            routes.setdefault(edge_id, []).append(route)
        for child in element:
            visit(child, ancestors + [element], inside_marker or tag == "marker")

    visit(root, [])
    return {
        "edges": [
            {"edge": edge_id, "routes": routes[edge_id]}
            for edge_id in sorted(routes)
        ],
        "lifelines": sorted(lifelines, key=repr),
    }


def digest(path: Path) -> str | None:
    try:
        with path.open("rb") as source:
            return hashlib.file_digest(source, "sha256").hexdigest()
    except OSError:
        # Execution below retains a missing/unreadable executable as a failure.
        return None


def run(binary: Path, source: Path, output: Path, timeout: float) -> tuple[int | None, str, str | None]:
    try:
        completed = subprocess.run(
            [str(binary), "--layout=dagre", str(source), str(output)],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        stderr = error.stderr or ""
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        return None, stderr.strip(), f"timed out after {timeout:g} seconds"
    except OSError as error:
        return None, "", f"could not execute: {error}"
    if completed.returncode < 0:
        return completed.returncode, completed.stderr.strip(), f"terminated by signal {-completed.returncode}"
    return completed.returncode, completed.stderr.strip(), None


def generated_svgs(output: Path) -> dict[str, Path]:
    """Return the SVG file or board tree produced for one requested output."""
    if output.is_file():
        return {".": output}
    output_directory = output.with_suffix("")
    if not output_directory.is_dir():
        return {}
    return {
        path.relative_to(output_directory).as_posix(): path
        for path in sorted(output_directory.rglob("*.svg"))
        if path.is_file()
    }


def comparable_diagnostics(stderr: str, output: Path) -> str:
    """Normalize only success-line elapsed time and the harness output root."""
    lines = []
    for line in stderr.splitlines():
        if line.startswith("success: successfully compiled "):
            line = line.replace(str(output.with_suffix("")), "<OUTPUT>")
            line = re.sub(r" in (?:[0-9]+(?:\.[0-9]+)?(?:ns|µs|μs|us|ms|s|m|h))+$", " in <ELAPSED>", line)
        lines.append(line)
    return "\n".join(lines)


def compare_fixture(
    rust: Path,
    go: Path,
    fixture: str,
    source: Path,
    temporary: Path,
    timeout: float,
) -> dict:
    result = {"fixture": fixture, "source": str(source), "status": "failed"}
    if not source.is_file():
        result["reason"] = "fixture does not exist"
        return result
    result["source_sha256"] = digest(source)
    rust_svg = temporary / f"rust-{fixture}.svg"
    go_svg = temporary / f"go-{fixture}.svg"
    rust_exit, rust_error, rust_failure = run(rust, source, rust_svg, timeout)
    go_exit, go_error, go_failure = run(go, source, go_svg, timeout)
    result.update({
        "rust_exit": rust_exit,
        "go_exit": go_exit,
        "rust_error": rust_error if rust_exit or rust_failure else None,
        "go_error": go_error if go_exit or go_failure else None,
    })
    if rust_failure or go_failure:
        result["reason"] = "; ".join(
            f"{name}: {failure}" for name, failure in (("rust", rust_failure), ("go", go_failure)) if failure
        )
    elif go_exit or rust_exit:
        result["error_exact"] = rust_exit == go_exit and rust_error == go_error
        result["diagnostic_equivalent"] = rust_exit == go_exit and comparable_diagnostics(rust_error, rust_svg) == comparable_diagnostics(go_error, go_svg)
        if go_exit and rust_exit and result["diagnostic_equivalent"]:
            result.update({
                "status": "inapplicable",
                "reason": "both implementations rejected the source with the same exit code and diagnostic after documented incidental normalization; layout geometry is unavailable",
            })
        else:
            result.update({
                "status": "divergent",
                "reason": "source acceptance, exit code, or diagnostic differs",
            })
    else:
        try:
            rust_outputs = generated_svgs(rust_svg)
            go_outputs = generated_svgs(go_svg)
            if not rust_outputs:
                raise OSError(f"no SVG file or board directory at {rust_svg}")
            if not go_outputs:
                raise OSError(f"no SVG file or board directory at {go_svg}")
            board_results = []
            board_differences = {}
            aggregate = {
                "rust_shape_count": 0,
                "go_shape_count": 0,
                "rust_connection_count": 0,
                "go_connection_count": 0,
            }
            for board_path in sorted(rust_outputs.keys() & go_outputs.keys()):
                rust_geometry = geometry(rust_outputs[board_path])
                go_geometry = geometry(go_outputs[board_path])
                rust_connections = connection_geometry(rust_outputs[board_path])
                go_connections = connection_geometry(go_outputs[board_path])
                rust_connection_count = sum(
                    len(item["routes"]) for item in rust_connections["edges"]
                ) + len(rust_connections["lifelines"])
                go_connection_count = sum(
                    len(item["routes"]) for item in go_connections["edges"]
                ) + len(go_connections["lifelines"])
                aggregate["rust_shape_count"] += len(rust_geometry[1])
                aggregate["go_shape_count"] += len(go_geometry[1])
                aggregate["rust_connection_count"] += rust_connection_count
                aggregate["go_connection_count"] += go_connection_count
                geometry_exact = rust_geometry == go_geometry
                routes_exact = rust_connections == go_connections
                board_result = {
                    "path": board_path,
                    "status": "matched" if geometry_exact and routes_exact else "divergent",
                    "rust_view_box": rust_geometry[0],
                    "go_view_box": go_geometry[0],
                    "rust_shape_count": len(rust_geometry[1]),
                    "go_shape_count": len(go_geometry[1]),
                    "rust_connection_count": rust_connection_count,
                    "go_connection_count": go_connection_count,
                }
                if not geometry_exact or not routes_exact:
                    rust_edge_map = {item["edge"]: item["routes"] for item in rust_connections["edges"]}
                    go_edge_map = {item["edge"]: item["routes"] for item in go_connections["edges"]}
                    different_edges = {
                        edge_id
                        for edge_id in rust_edge_map.keys() | go_edge_map.keys()
                        if rust_edge_map.get(edge_id) != go_edge_map.get(edge_id)
                    }
                    board_differences[board_path] = {
                        "rust_only_shapes": list(
                            (Counter(rust_geometry[1]) - Counter(go_geometry[1])).elements()
                        ),
                        "go_only_shapes": list(
                            (Counter(go_geometry[1]) - Counter(rust_geometry[1])).elements()
                        ),
                        "rust_connections": {
                            "edges": [item for item in rust_connections["edges"] if item["edge"] in different_edges],
                            "lifelines": list(
                                (Counter(rust_connections["lifelines"]) - Counter(go_connections["lifelines"])).elements()
                            ),
                        },
                        "go_connections": {
                            "edges": [item for item in go_connections["edges"] if item["edge"] in different_edges],
                            "lifelines": list(
                                (Counter(go_connections["lifelines"]) - Counter(rust_connections["lifelines"])).elements()
                            ),
                        },
                    }
                board_results.append(board_result)
        except (OSError, ET.ParseError, ValueError) as error:
            result["reason"] = f"could not read generated SVG output: {error}"
            return result
        missing_rust_boards = sorted(go_outputs.keys() - rust_outputs.keys())
        missing_go_boards = sorted(rust_outputs.keys() - go_outputs.keys())
        result.update(aggregate)
        result["board_count"] = len(rust_outputs)
        result["boards"] = board_results
        result["status"] = (
            "matched"
            if not missing_rust_boards
            and not missing_go_boards
            and all(board["status"] == "matched" for board in board_results)
            else "divergent"
        )
        if missing_rust_boards or missing_go_boards:
            result["board_path_difference"] = {
                "rust_missing": missing_rust_boards,
                "go_missing": missing_go_boards,
            }
        if board_differences:
            if len(rust_outputs) == 1 and len(go_outputs) == 1:
                result["difference"] = board_differences["."]
            else:
                result["board_differences"] = board_differences
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "fixtures",
        nargs="*",
        default=None,
        help="test/d2 fixture name or label=source-path",
    )
    parser.add_argument(
        "--source-dir",
        type=Path,
        help="recursively compare every .d2 file below this source directory",
    )
    parser.add_argument(
        "--rust",
        type=Path,
        default=ROOT / "target/release/d2-rs",
    )
    parser.add_argument(
        "--go",
        type=Path,
        default=ROOT / ".reference/bin/d2-v0.9.0-source-go1.27",
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=float, default=60, help="seconds allowed for each binary/fixture invocation (default: 60)")
    parser.add_argument("--summary", action="store_true", help="print only counts; --output still receives the full JSON report")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be a finite positive number")
    args.rust = args.rust.resolve()
    args.go = args.go.resolve()
    rust_digest = digest(args.rust)
    go_digest = digest(args.go)

    if args.source_dir is not None and args.fixtures:
        parser.error("positional fixtures cannot be combined with --source-dir")

    fixtures = []
    if args.source_dir is not None:
        source_root = args.source_dir.resolve()
        if not source_root.is_dir():
            parser.error(f"source directory does not exist: {source_root}")
        for source in sorted(source_root.rglob("*.d2")):
            relative = source.relative_to(source_root).as_posix()
            label = "__".join(Path(relative).with_suffix("").parts)
            label += "-" + hashlib.sha256(relative.encode()).hexdigest()[:8]
            fixtures.append((label, source))
    else:
        requested_fixtures = args.fixtures if args.fixtures else list(DEFAULT_FIXTURES)
        for value in requested_fixtures:
            if "=" in value:
                label, raw_source = value.split("=", 1)
                if (
                    not label
                    or not raw_source
                    or Path(label).name != label
                    or label in {".", ".."}
                ):
                    parser.error(f"invalid fixture {value!r}; expected label=source-path")
                fixtures.append((label, Path(raw_source).resolve()))
            else:
                fixtures.append((value, ROOT / "tests/d2" / f"{value}.d2"))

    results = []
    with tempfile.TemporaryDirectory(prefix="d2-rs-dagre-") as temporary:
        temporary_path = Path(temporary)
        for fixture, source in fixtures:
            results.append(
                compare_fixture(
                    args.rust, args.go, fixture, source, temporary_path, args.timeout
                )
            )

    report = {
        "schema_version": 3,
        "kind": "d2-v0.9.0-dagre-layout-and-route-geometry",
        "authority": "selected reference executable (default: pinned D2 v0.9.0 source build with Go 1.27.0)",
        "rust_binary": str(args.rust),
        "go_binary": str(args.go),
        "rust_binary_sha256": rust_digest,
        "go_binary_sha256": go_digest,
        "timeout_seconds": args.timeout,
        "diagnostic_normalization": "Success-line elapsed durations and harness-specific output roots only; raw errors retained; no reordering or removed events.",
        "matched": sum(result["status"] == "matched" for result in results),
        "divergent": sum(result["status"] == "divergent" for result in results),
        "inapplicable": sum(result["status"] == "inapplicable" for result in results),
        "failed": sum(result["status"] == "failed" for result in results),
        "results": results,
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered)
    if args.summary:
        print(
            f"Dagre geometry and routes: {report['matched']} matched, {report['divergent']} divergent, "
            f"{report['failed']} failed, {report['inapplicable']} upstream-inapplicable"
            + (f"; wrote {args.output}" if args.output else "")
        )
    else:
        print(rendered, end="")
    return 1 if report["divergent"] or report["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
