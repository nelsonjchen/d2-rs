#!/usr/bin/env python3
"""Measure Go map-order variation in D2 v0.9.0 Dagre cross-spacing.

The Go oracle must be built with ``build-dagre-trace-oracle.py`` and the Rust
diagnostic binary with ``--features diagnostic-traces``. This checker preserves
each observed order, then reports whether it changes the SVG on the selected
fixtures. It does not sort or normalize map events.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURES = (
    "icon_positions",
    "benchmark-medium",
    "dagre-compound-wide-label",
    "nested-direction-grid",
)
BEGIN = re.compile(
    r'^(?:GO|RUST)_CROSS_MAP_BEGIN root=("(?:\\.|[^"])*") '
    r"start_bits=(\w+) distance_bits=(\w+) horizontal=(\w+) "
    r"margin=(\w+) count=(\d+)$"
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def parse_events(stderr: bytes, prefix: str) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    current: dict[str, object] | None = None
    for line in stderr.decode("utf-8", errors="replace").splitlines():
        if line.startswith(f"{prefix}_CROSS_MAP_BEGIN "):
            if current is not None:
                events.append(current)
            match = BEGIN.match(line)
            if match is None:
                raise ValueError(f"malformed cross-map header: {line}")
            current = {
                "context": (
                    json.loads(match.group(1)),
                    match.group(2),
                    match.group(3),
                    match.group(4),
                    match.group(5),
                    int(match.group(6)),
                ),
                "order": [],
            }
        elif line.startswith(f"{prefix}_CROSS_MAP_SEEN "):
            if current is None:
                raise ValueError("cross-map member appeared without a header")
            current["order"].append(json.loads(line.split("object=", 1)[1]))
        elif line == f"{prefix}_CROSS_MAP_END":
            if current is None:
                raise ValueError("cross-map end appeared without a header")
            events.append(current)
            current = None
    if current is not None:
        events.append(current)
    for event in events:
        if len(event["order"]) != event["context"][5]:
            raise ValueError("cross-map member count disagrees with its header")
    return events


def run_cli(binary: Path, fixture: Path, output: Path, *, trace: bool) -> tuple[bytes, bytes]:
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("DAGRE_") and not key.startswith("DAGRO_")
    }
    if trace:
        env["DAGRE_TRACE_CROSS_MAP"] = "1"
    command = [str(binary), "--layout=dagre", str(fixture), str(output)]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        capture_output=True,
        timeout=120,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(
            f"{binary.name} failed on {fixture.name}: "
            f"{completed.stderr.decode('utf-8', errors='replace')}"
        )
    return output.read_bytes(), completed.stderr


def fixture_report(
    name: str,
    runs: int,
    go: Path,
    go_clean: Path,
    official: Path,
    rust: Path,
    scratch: Path,
) -> dict[str, object]:
    fixture = ROOT / "tests/d2" / f"{name}.d2"
    if not fixture.is_file():
        raise FileNotFoundError(fixture)
    go_base_svg, _ = run_cli(go, fixture, scratch / f"{name}.go-base.svg", trace=False)
    go_clean_svg, _ = run_cli(
        go_clean, fixture, scratch / f"{name}.go-clean.svg", trace=False
    )
    official_svg, _ = run_cli(
        official, fixture, scratch / f"{name}.official.svg", trace=False
    )
    rust_base_svg, _ = run_cli(rust, fixture, scratch / f"{name}.rust-base.svg", trace=False)
    go_runs = []
    for index in range(runs):
        svg, stderr = run_cli(
            go, fixture, scratch / f"{name}.go-{index}.svg", trace=True
        )
        go_runs.append({"svg_sha256": sha256_bytes(svg), "events": parse_events(stderr, "GO")})
    rust_trace_svg, rust_stderr = run_cli(
        rust, fixture, scratch / f"{name}.rust-trace.svg", trace=True
    )
    rust_events = parse_events(rust_stderr, "RUST")

    reference_events = go_runs[0]["events"]
    stable_contexts = all(
        len(run["events"]) == len(reference_events)
        and all(
            left["context"] == right["context"]
            for left, right in zip(reference_events, run["events"])
        )
        for run in go_runs
    )
    stable_members = stable_contexts and all(
        set(reference["order"]) == set(observed["order"])
        for run in go_runs
        for reference, observed in zip(reference_events, run["events"])
    )
    variant_calls = 0
    if stable_contexts:
        variant_calls = sum(
            len({tuple(run["events"][index]["order"]) for run in go_runs}) > 1
            for index in range(len(reference_events))
        )
    rust_context_matches = 0
    rust_members_match = 0
    rust_order_matches_first_go = 0
    scope_prefixes = set()
    normalized_rust_orders = []
    for go_event, rust_event in zip(reference_events, rust_events):
        go_context = go_event["context"]
        rust_context = rust_event["context"]
        go_root, rust_root = go_context[0], rust_context[0]
        if rust_root == go_root:
            scope_prefix = ""
        elif rust_root.endswith(f".{go_root}"):
            scope_prefix = rust_root[: -len(go_root)]
        else:
            normalized_rust_orders.append(None)
            continue
        scope_prefixes.add(scope_prefix)
        normalized_context = (go_root, *rust_context[1:])
        if normalized_context != go_context:
            normalized_rust_orders.append(None)
            continue
        rust_context_matches += 1
        rust_order = [
            object_id[len(scope_prefix) :]
            if scope_prefix and object_id.startswith(scope_prefix)
            else object_id
            for object_id in rust_event["order"]
        ]
        normalized_rust_orders.append(rust_order)
        if set(go_event["order"]) == set(rust_order):
            rust_members_match += 1
        if go_event["order"] == rust_order:
            rust_order_matches_first_go += 1
    go_hashes = sorted({run["svg_sha256"] for run in go_runs})
    order_examples = []
    if stable_contexts:
        for index, reference in enumerate(reference_events):
            distinct_orders = []
            for run in go_runs:
                order = run["events"][index]["order"]
                if order not in distinct_orders:
                    distinct_orders.append(order)
                if len(distinct_orders) == 2:
                    break
            if len(distinct_orders) > 1:
                context = reference["context"]
                order_examples.append({
                    "call_index": index,
                    "context": {
                        "root": context[0],
                        "start_bits": context[1],
                        "distance_bits": context[2],
                        "horizontal": context[3] == "true",
                        "margin": context[4] == "true",
                        "count": context[5],
                    },
                    "first_observed_go_order": distinct_orders[0],
                    "second_observed_go_order": distinct_orders[1],
                    "rust_queue_discovery_order": normalized_rust_orders[index]
                    if index < len(normalized_rust_orders)
                    else None,
                })
    return {
        "fixture": fixture.relative_to(ROOT).as_posix(),
        "fixture_sha256": sha256_file(fixture),
        "runs": runs,
        "go_cross_map_calls": len(reference_events),
        "go_contexts_stable": stable_contexts,
        "go_seen_members_stable": stable_members,
        "go_map_order_varied_calls": variant_calls,
        "go_order_examples": order_examples,
        "go_svg_hash_count_with_trace": len(go_hashes),
        "official_vs_clean_source_svg_equal": sha256_bytes(official_svg)
        == sha256_bytes(go_clean_svg),
        "clean_source_vs_instrumented_trace_off_svg_equal": sha256_bytes(go_clean_svg)
        == sha256_bytes(go_base_svg),
        "go_trace_off_vs_on_svg_equal": sha256_bytes(go_base_svg) == go_runs[0]["svg_sha256"],
        "rust_cross_map_calls": len(rust_events),
        "rust_scope_prefixes_used": sorted(scope_prefixes),
        "rust_contexts_match_go": rust_context_matches == len(reference_events) == len(rust_events),
        "rust_seen_members_match_go": rust_members_match == len(reference_events) == len(rust_events),
        "rust_seen_member_match_calls": rust_members_match,
        "rust_order_matches_first_go_calls": rust_order_matches_first_go,
        "rust_trace_off_vs_on_svg_equal": sha256_bytes(rust_base_svg) == sha256_bytes(rust_trace_svg),
        "go_svg_sha256": go_hashes[0] if len(go_hashes) == 1 else None,
        "rust_svg_sha256": sha256_bytes(rust_trace_svg),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-dagre-trace")
    parser.add_argument("--go-clean", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-source-go1.27")
    parser.add_argument("--official", type=Path, default=ROOT / ".reference/bin/d2-v0.9.0-official")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--runs", type=int, default=20)
    parser.add_argument("--output", type=Path, default=ROOT / "oracle-cache/reports/dagre-cross-map-order-20260924.json")
    parser.add_argument("fixtures", nargs="*", default=DEFAULT_FIXTURES)
    args = parser.parse_args()
    if args.runs < 2:
        parser.error("--runs must be at least 2 to observe order variation")
    if not all(path.is_file() for path in (args.go, args.go_clean, args.official, args.rust)):
        parser.error("build or fetch the pinned Go binaries and Rust diagnostic binary first")
    with tempfile.TemporaryDirectory(prefix="d2-rs-cross-map-") as temporary:
        scratch = Path(temporary)
        fixtures = [
            fixture_report(name, args.runs, args.go, args.go_clean, args.official, args.rust, scratch)
            for name in args.fixtures
        ]
    report = {
        "schema_version": 1,
        "kind": "d2-v0.9.0-dagre-cross-spacing-map-order",
        "reference": {
            "d2_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
            "go_toolchain": "go1.27.0",
            "go_binary": str(args.go),
            "go_binary_sha256": sha256_file(args.go),
            "go_clean_source_binary": str(args.go_clean),
            "go_clean_source_binary_sha256": sha256_file(args.go_clean),
            "official_binary": str(args.official),
            "official_binary_sha256": sha256_file(args.official),
            "rust_binary": str(args.rust),
            "rust_binary_sha256": sha256_file(args.rust),
            "checker_sha256": sha256_file(Path(__file__)),
            "cross_map_trace_patch_sha256": sha256_file(
                ROOT / "tools/patches/d2-v090-dagre-cross-map-trace.patch"
            ),
            "rust_diagnostic_feature": "diagnostic-traces",
            "rust_profile": "diagnostic",
            "map_order_is_not_normalized": True,
            "metric": "ordered map observations and exact SVG byte hashes",
        },
        "fixtures": fixtures,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({
        "fixtures": len(fixtures),
        "go_calls": sum(item["go_cross_map_calls"] for item in fixtures),
        "go_order_varied_calls": sum(item["go_map_order_varied_calls"] for item in fixtures),
        "all_go_svg_runs_stable": all(item["go_svg_hash_count_with_trace"] == 1 for item in fixtures),
        "all_pinned_go_binaries_equal": all(
            item["official_vs_clean_source_svg_equal"]
            and item["clean_source_vs_instrumented_trace_off_svg_equal"]
            for item in fixtures
        ),
        "all_trace_off_on_outputs_equal": all(
            item["go_trace_off_vs_on_svg_equal"] and item["rust_trace_off_vs_on_svg_equal"]
            for item in fixtures
        ),
    }))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
