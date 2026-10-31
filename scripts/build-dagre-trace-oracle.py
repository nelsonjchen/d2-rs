#!/usr/bin/env python3
"""Build durable, diagnostic-only Dagre references without modifying checkouts.

Usage: uv run --no-project python scripts/build-dagre-trace-oracle.py
Set DAGRE_TRACE_OBJECTS and DAGRO_TRACE_RANKS/ORDER/POS to emit observations on
stderr. The ordinary and instrumented results are checked against both pinned
official/source binaries before this script reports success.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess

from references import ROOT, REFERENCE, MANIFEST, digest, host_name, release, source_builds, verify_source


TRACE_FLAGS = ("DAGRE_TRACE_OBJECTS", "DAGRO_TRACE_RANKS", "DAGRO_TRACE_ORDER", "DAGRO_TRACE_POS",
               "DAGRO_TRACE_RANK_OPS", "DAGRE_TRACE_CROSS_MAP", "DAGRE_TRACE_RAW_POINTS")
FIXTURES = ("icon_positions", "sequence-trace-order")
DEPENDENCY = "github.com/d2lang/dagro"
ADAPTER = "d2layouts/d2dagrelayout/layout.go"


def execute(command: list[str], *, cwd: Path, environment: dict, timeout: float = 600) -> str:
    completed = subprocess.run(command, cwd=cwd, env=environment, text=True,
                               capture_output=True, timeout=timeout)
    if completed.returncode:
        raise RuntimeError(f"Command failed ({completed.returncode}): {command!r}\n{completed.stdout}\n{completed.stderr}")
    return completed.stdout.strip()


def apply_patch(patch: Path, directory: Path, environment: dict) -> None:
    execute(["patch", "--batch", "--forward", "-p1", "-i", str(patch)], cwd=directory, environment=environment)


def tree_hashes(directory: Path) -> dict[str, str]:
    return {path.relative_to(directory).as_posix(): digest(path)
            for path in sorted(directory.rglob("*")) if path.is_file()}


def compare_outputs(binary: Path, source_binary: Path, official_binary: Path,
                    environment: dict, timeout: float) -> dict:
    comparator_path = ROOT / "scripts/check-dagre-parity.py"
    spec = importlib.util.spec_from_file_location("dagre_geometry", comparator_path)
    comparator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(comparator)
    witness = REFERENCE / "dagre-trace-validation"
    witness.mkdir(parents=True, exist_ok=True)
    results = []
    for fixture in FIXTURES:
        fixture_path = ROOT / "tests/d2" / f"{fixture}.d2"
        runs = {}
        for name, executable, tracing in (("official", official_binary, False),
                                          ("source", source_binary, False),
                                          ("instrumented", binary, False),
                                          ("instrumented_trace", binary, True)):
            svg = witness / f"{fixture}-{name}.svg"
            svg.unlink(missing_ok=True)
            run_environment = {**environment, **({flag: "1" for flag in TRACE_FLAGS} if tracing else {})}
            completed = subprocess.run([str(executable), "--layout=dagre", str(fixture_path), str(svg)],
                                       cwd=ROOT, env=run_environment, capture_output=True, timeout=timeout)
            stderr = witness / f"{fixture}-{name}.stderr"
            stderr.write_bytes(completed.stderr)
            runs[name] = {"binary_sha256": digest(executable), "exit_code": completed.returncode,
                          "stdout_sha256": hashlib.sha256(completed.stdout).hexdigest(),
                          "stderr": str(stderr.relative_to(ROOT)), "stderr_sha256": digest(stderr)}
            if completed.returncode != 0:
                runs[name]["error"] = completed.stderr.decode("utf-8", errors="replace")
                continue
            runs[name].update({"svg": str(svg.relative_to(ROOT)), "svg_sha256": digest(svg),
                               "geometry": comparator.geometry(svg)})
        successful = all(run["exit_code"] == 0 for run in runs.values())
        geometry_equal = successful and all(run["geometry"] == runs["official"]["geometry"] for run in runs.values())
        svg_equal = successful and all(run["svg_sha256"] == runs["official"]["svg_sha256"] for run in runs.values())
        instrumentation_svg_equal = successful and runs["instrumented"]["svg_sha256"] == runs["instrumented_trace"]["svg_sha256"]
        stdout_equal = all(run["stdout_sha256"] == runs["official"]["stdout_sha256"] for run in runs.values())
        trace_text = (witness / f"{fixture}-instrumented_trace.stderr").read_text()
        required_trace_prefixes = ("GO_MAP ", "GO_PHASE ", "GO_EDGESET ", "GO_DAGRO_PHASE ", "GO_DAGRO_LAYERS ", "GO_POS ")
        trace_events = {prefix: trace_text.count(prefix) for prefix in (*required_trace_prefixes, "GO_EDGE ")}
        # An edgeless fixture still needs an explicit empty edge set per phase.
        trace_complete = (all(trace_events[prefix] for prefix in required_trace_prefixes)
                          and trace_events["GO_EDGESET "] == trace_events["GO_PHASE "])
        ordinary_text = (witness / f"{fixture}-instrumented.stderr").read_text()
        ordinary_has_trace = any(prefix in ordinary_text for prefix in trace_events)
        results.append({"fixture": fixture, "fixture_sha256": digest(fixture_path),
                        "geometry_equal": geometry_equal, "svg_bytes_equal": svg_equal,
                        "instrumentation_svg_bytes_equal": instrumentation_svg_equal, "stdout_equal": stdout_equal,
                        "ordinary_has_trace": ordinary_has_trace, "trace_event_counts": trace_events,
                        "passed": geometry_equal and instrumentation_svg_equal and stdout_equal and not ordinary_has_trace and trace_complete,
                        "runs": runs})
    record = {"comparator_sha256": digest(comparator_path), "scope": "existing Dagre geometry comparator plus exact SVG byte observation",
              "passed": all(result["passed"] for result in results), "results": results}
    (witness / "report.json").write_text(json.dumps(record, indent=2) + "\n")
    return record


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=120, help="seconds per verification invocation")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    source = verify_source()
    source_builds()
    environment = {key: value for key, value in os.environ.items() if key not in TRACE_FLAGS}
    environment.update({"GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"],
                        "GOMODCACHE": str(REFERENCE / "go-mod/pkg/mod"), "GOMAXPROCS": "2"})
    version = MANIFEST["dependencies"][DEPENDENCY]
    module = REFERENCE / "dagre-trace-module"
    module.mkdir(parents=True, exist_ok=True)
    modfile = module / "oracle.mod"
    shutil.copy2(source / "go.mod", modfile)
    shutil.copy2(source / "go.sum", modfile.with_suffix(".sum"))
    dependency_info = json.loads(execute(["go", "mod", "download", "-modfile", str(modfile), "-json", f"{DEPENDENCY}@{version}"],
                                         cwd=source, environment=environment))
    expected_sum = next(line.split()[2] for line in (source / "go.sum").read_text().splitlines()
                        if line.startswith(f"{DEPENDENCY} {version} "))
    if dependency_info["Sum"] != expected_sum:
        raise RuntimeError("Dagro module checksum differs from pinned D2 go.sum")
    execute(["go", "mod", "verify", "-modfile", str(modfile)], cwd=source, environment=environment)
    pristine_dependency = Path(dependency_info["Dir"])
    destination = REFERENCE / "dagro-trace"
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(pristine_dependency, destination)
    for path in [destination, *destination.rglob("*")]:
        path.chmod(path.stat().st_mode | stat.S_IWUSR)
    dagro_patch = ROOT / "tools/patches/dagro-v021-diagnostic-trace.patch"
    dagro_rank_ops_patch = ROOT / "tools/patches/dagro-v021-rank-ops-trace.patch"
    adapter_patch = ROOT / "tools/patches/d2-v090-dagre-adapter-trace.patch"
    raw_route_patch = ROOT / "tools/patches/d2-v090-dagre-raw-trace.patch"
    scope_ids_patch = ROOT / "tools/patches/d2-v090-dagre-scope-ids.patch"
    cross_map_patch = ROOT / "tools/patches/d2-v090-dagre-cross-map-trace.patch"
    trace_scope_patch = ROOT / "tools/patches/d2-v090-trace-object-scope.patch"
    apply_patch(dagro_patch, destination, environment)
    apply_patch(dagro_rank_ops_patch, destination, environment)
    overlay_source = REFERENCE / "dagre-trace-source"
    patched_adapter = overlay_source / ADAPTER
    patched_adapter.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source / ADAPTER, patched_adapter)
    apply_patch(adapter_patch, overlay_source, environment)
    apply_patch(raw_route_patch, overlay_source, environment)
    apply_patch(scope_ids_patch, overlay_source, environment)
    apply_patch(cross_map_patch, overlay_source, environment)
    patched_graph_model = overlay_source / "d2graph/d2graph.go"
    patched_nested_layout = overlay_source / "d2layouts/d2layouts.go"
    for destination_path, source_path in (
        (patched_graph_model, source / "d2graph/d2graph.go"),
        (patched_nested_layout, source / "d2layouts/d2layouts.go"),
    ):
        destination_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_path, destination_path)
    apply_patch(trace_scope_patch, overlay_source, environment)
    overlay = module / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(source / ADAPTER): str(patched_adapter),
        str(source / "d2graph/d2graph.go"): str(patched_graph_model),
        str(source / "d2layouts/d2layouts.go"): str(patched_nested_layout),
    }}, indent=2) + "\n")
    with modfile.open("a") as manifest:
        manifest.write(f"\nreplace {DEPENDENCY} => {destination}\n")
    binary = REFERENCE / "bin/d2-v0.9.0-dagre-trace"
    binary.parent.mkdir(parents=True, exist_ok=True)
    build_command = ["go", "build", "-p", "2", "-mod=readonly", "-modfile", str(modfile),
                     "-overlay", str(overlay), "-ldflags", "-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
                     "-o", str(binary), "."]
    execute(build_command, cwd=source, environment=environment)
    verify_source()
    if execute([str(binary), "--version"], cwd=ROOT, environment=environment) != "v0.9.0":
        raise RuntimeError("Diagnostic binary reports an unexpected version")
    metadata = execute(["go", "version", "-m", str(binary)], cwd=ROOT, environment=environment)
    if MANIFEST["d2"]["go_toolchain"] not in metadata.splitlines()[0]:
        raise RuntimeError("Diagnostic binary has an unexpected Go toolchain")
    record = {
        "schema_version": 1, "source_commit": MANIFEST["d2"]["commit"], "source_checkout": str(source),
        "source_change": "Pristine D2 with diagnostic adapter, nested-scope identity overlays and a patched dependency copy; no checkout modified",
        "go_toolchain": MANIFEST["d2"]["go_toolchain"], "go_build_metadata": metadata,
        "binary_sha256": digest(binary), "builder_sha256": digest(Path(__file__)),
                               "trace_patch_sha256": {str(path.relative_to(ROOT)): digest(path)
                               for path in (dagro_patch, dagro_rank_ops_patch, adapter_patch,
                                            raw_route_patch, scope_ids_patch, cross_map_patch,
                                            trace_scope_patch)},
        "source_adapter_sha256": digest(source / ADAPTER), "instrumented_adapter_sha256": digest(patched_adapter),
        "dependency": {"module": DEPENDENCY, "version": version, "sum": expected_sum,
                       "zip_sha256": digest(Path(dependency_info["Zip"])),
                       "pristine_files_sha256": tree_hashes(pristine_dependency),
                       "instrumented_files_sha256": tree_hashes(destination)},
        "modfile_sha256": digest(modfile), "sumfile_sha256": digest(modfile.with_suffix(".sum")),
        "overlay_sha256": digest(overlay), "build_command": build_command,
        "build_environment": {key: environment[key] for key in ("GOTOOLCHAIN", "GOMODCACHE", "GOMAXPROCS")},
        "trace_flags": TRACE_FLAGS,
    }
    provenance = binary.with_name(binary.name + ".provenance.json")
    provenance.write_text(json.dumps(record, indent=2) + "\n")
    source_binary = ROOT / MANIFEST["source_builds"]["d2-v0.9.0-go1.27.0"]["binary"]
    record["equivalence"] = compare_outputs(binary, source_binary, release(host_name(), False), environment, args.timeout)
    provenance.write_text(json.dumps(record, indent=2) + "\n")
    if not record["equivalence"]["passed"]:
        raise RuntimeError(f"Diagnostic equivalence failed; inspect {provenance}")
    print(f"Built {binary}; both fixtures match official/source geometry with tracing disabled and enabled")


if __name__ == "__main__":
    main()
