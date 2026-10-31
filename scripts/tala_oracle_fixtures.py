#!/usr/bin/env python3
"""TALA oracle fixture generators (recipes in tools/fixtures/tala-oracle.toml).

Run only through the runner, e.g.
`uv run --no-project python scripts/regen-fixtures.py --only 'tala-*'`; then
`cargo test -p d2-tala-rs --features oracle-fixtures` compares (docs/FIXTURES.md).

Every subcommand builds against the runner's patched upstream tree
($D2RS_UPSTREAM, the manifest pin), runs the Go side twice and requires
byte-identical output, and writes only its staged outputs and its workdir.

- `layout`  tools/engine-oracle `-operation layout` on upstream TALA testdata.
- `codec`   tools/engine-oracle `-operation roundtrip` (graphjson Unmarshal +
            Marshal) over arrowhead-label position/placement cases.
- `stages`  hook tools/fixtures/hooks/tala-tree-publication: the GraphJSON
            graph after each pre-placement stage.
- `route-validation`  tools/tala-route-validation-oracle (public
            d2talalayout.RouteEdges/Layout) over generated invalid routes.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from exact_json import decode_output
from fixture_recipe import go_env, output, workdir
from references import ROOT, SOURCE_COMMIT

# Go internal-package visibility follows import paths: a module whose path is
# inside TALA's subtree may import d2talalayout/internal/... of the replaced
# upstream module. The module itself lives only in the recipe workdir.
ORACLE_MODULE = "github.com/d2lang/d2/d2layouts/d2talalayout/cmd/d2rs-oracle"


def upstream() -> Path:
    value = os.environ.get("D2RS_UPSTREAM")
    if not value:
        sys.exit("D2RS_UPSTREAM is unset: this recipe needs `upstream = true`")
    return Path(value)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def build(source_dir: str, name: str, overlay: dict[str, str] | None = None) -> Path:
    """Build every non-test *.go in `source_dir` as one main package.

    `overlay` maps upstream-relative paths to file contents, written to the
    workdir and passed to `go build -overlay` (the shared tree is untouched)."""
    module = workdir() / name
    module.mkdir(parents=True)
    sources = sorted(p for p in (ROOT / source_dir).glob("*.go") if not p.name.endswith("_test.go"))
    assert sources, f"{source_dir} has no Go sources"
    for src in sources:
        shutil.copy2(src, module / src.name)
    # The upstream module's own requirements and checksums, so the build
    # selects exactly the dependency versions of the pinned tree.
    go_mod = (upstream() / "go.mod").read_text()
    go_mod, count = re.subn(r"(?m)^module .*$", f"module {ORACLE_MODULE}/{name}", go_mod, count=1)
    assert count == 1, "upstream go.mod has no module line"
    go_mod += ("\nrequire github.com/d2lang/d2 v0.0.0-00010101000000-000000000000\n"
               f"\nreplace github.com/d2lang/d2 => {upstream()}\n")
    (module / "go.mod").write_text(go_mod)
    shutil.copy2(upstream() / "go.sum", module / "go.sum")
    binary = workdir() / "bin" / name
    flags = []
    if overlay:
        replace = {}
        for rel, text in overlay.items():
            path = workdir() / f"{name}-overlay" / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
            replace[str(upstream() / rel)] = str(path)
        overlay_json = workdir() / f"{name}-overlay.json"
        overlay_json.write_text(json.dumps({"Replace": replace}))
        flags = ["-overlay", str(overlay_json)]
    subprocess.run(["go", "build", "-mod=readonly", *flags, "-o", str(binary), "."], cwd=module, env=go_env(),
                   check=True)
    return binary


def run_twice(argv: list[str], stdin: bytes) -> bytes:
    """Run the oracle twice; both runs must succeed with identical stdout."""
    runs = [subprocess.run(argv, input=stdin, stdout=subprocess.PIPE, check=True, timeout=600).stdout
            for _ in range(2)]
    assert runs[0] == runs[1], f"{argv[0]}: repeated runs differ"
    assert runs[0], f"{argv[0]}: empty output"
    return runs[0]


def write_json(logical: str, value, *, compact: bool = False, ascii: bool = True) -> None:
    path = output(logical)
    if compact:
        text = json.dumps(value, separators=(",", ":"), ensure_ascii=ascii)
    else:
        text = json.dumps(value, indent=2, ensure_ascii=ascii)
    path.write_text(text + "\n", encoding="utf-8")


def upstream_input(rel: str) -> tuple[bytes, str]:
    data = (upstream() / rel).read_bytes()
    return data, sha256(data)


FIXTURES = "crates/d2-tala-rs/tests/fixtures/"
BIG_MANY = "d2layouts/d2talalayout/internal/testdata/benchmark/big_many_subgraphs.json"
TREE_CONTAINER_INPUTS = [
    "d2layouts/d2talalayout/internal/testdata/benchmark/big_connected.json",
    "d2layouts/d2talalayout/internal/testdata/benchmark/nodes11_edges13_containers3_width4561_height4310.json",
    "d2layouts/d2talalayout/internal/testdata/layout/basic_tree/graph.input.json",
    "d2layouts/d2talalayout/internal/testdata/layout/larger_tree/graph.input.json",
    "d2layouts/d2talalayout/internal/testdata/ovg/basic_tree/graph.input.json",
    "d2layouts/d2talalayout/internal/testdata/ovg/larger_tree/graph.input.json",
    "d2layouts/d2talalayout/internal/testdata/regression/cloud_port_index.json",
]


def layout_case(binary: Path, rel: str, seed: int) -> dict:
    data, digest = upstream_input(rel)
    stdout = run_twice([str(binary), "-operation", "layout", "-seed", str(seed)], data)
    return {"input_sha256": digest, "seed": seed, "input": json.loads(data),
            "source": json.loads(stdout), "source_stdout_sha256": sha256(stdout)}


def layout(args) -> None:
    binary = build("tools/engine-oracle", "engine-oracle")
    if args.set == "label-obstacles":
        # One complete seeded layout read by two contracts: label obstacles
        # (graphjson/serialize.rs) and table-port member obstacles
        # (examples/graphjson-layout.rs).
        case = layout_case(binary, BIG_MANY, 1)
        write_json(FIXTURES + "label-obstacles-source-v090.json", {
            "source_commit": SOURCE_COMMIT,
            "cases": [{"name": BIG_MANY, "upstream": BIG_MANY, "input_sha256": case["input_sha256"],
                       "seed": case["seed"], "input": case["input"],
                       "source_stdout_sha256": case["source_stdout_sha256"], "source": case["source"]}]})
        write_json(FIXTURES + "table-port-member-obstacles-source-v090.json", {
            "upstream": BIG_MANY, "source_commit": SOURCE_COMMIT, "input_sha256": case["input_sha256"],
            "source_stdout_sha256": case["source_stdout_sha256"], "seed": case["seed"],
            "input": case["input"], "source": case["source"]})
    elif args.set == "tree-containers":
        cases = []
        for rel in TREE_CONTAINER_INPUTS:
            case = layout_case(binary, rel, 1)
            cases.append({"name": rel, "upstream": rel, "input_sha256": case["input_sha256"], "seed": case["seed"],
                          "input": case["input"], "source": case["source"],
                          "source_stdout_sha256": case["source_stdout_sha256"]})
        write_json(FIXTURES + "tree-containers-source-v090.json", {"source_commit": SOURCE_COMMIT, "cases": cases})
    elif args.set == "graphjson-owners":
        graphjson_owner_layouts(binary)
    elif args.set == "serialize":
        serialize_layouts(binary)
    elif args.set == "errors":
        layout_errors(binary)
    else:
        sys.exit(f"unknown layout set {args.set}")


TESTDATA = "d2layouts/d2talalayout/internal/testdata/"
CLOUD_CLUSTER = TESTDATA + "layout/cloud_contains_cluster/graph.input.json"
NO_SEQ_CLUSTER = TESTDATA + "layout/no-seq-in-cluster/graph.input.json"
LARGE_NESTED = TESTDATA + "layout/large_nested/graph.input.json"
# Seeded replays of two upstream inputs whose cluster vessels and sequence
# vessels go through transactions (names kept from the historical capture).
SEEDED_REPLAYS = [("replay-21", CLOUD_CLUSTER, 1), ("replay-41", NO_SEQ_CLUSTER, 1),
                  *[(f"original-021-seed{s}", CLOUD_CLUSTER, s) for s in (2, 7, 42)],
                  *[(f"original-041-seed{s}", NO_SEQ_CLUSTER, s) for s in (2, 7, 42)]]


def upstream_case(binary: Path, rel: str, seed: int, name: str | None = None) -> dict:
    case = layout_case(binary, rel, seed)
    return {"name": name or rel, "upstream": rel, "input_sha256": case["input_sha256"], "seed": seed,
            "input": case["input"], "source_stdout_sha256": case["source_stdout_sha256"], "source": case["source"]}


def single_case(binary: Path, rel: str, seed: int) -> dict:
    case = upstream_case(binary, rel, seed)
    del case["name"]
    return {**case, "source_commit": SOURCE_COMMIT}


def graphjson_owner_layouts(binary: Path) -> None:
    """Complete seeded layouts read by examples/graphjson-layout.rs."""
    clusters = TESTDATA + "layout/clusters/graph.input.json"
    write_json(FIXTURES + "cluster-hub-owner-source-v090.json",
               [single_case(binary, clusters, seed) for seed in (1, 123, 8675309)])
    for logical, rel in [
        ("container-direct-owner-source-v090.json",
         TESTDATA + "benchmark/nodes38_edges44_containers13_width11276_height12214.json"),
        ("container-refit-owner-source-v090.json",
         TESTDATA + "benchmark/nodes36_edges21_containers11_width10377_height10420.json"),
        ("cluster-owner-restore-source-v090.json", TESTDATA + "layout/expand_cluster_other_dir/graph.input.json"),
        ("packing-member-obstacles-source-v090.json", LARGE_NESTED),
    ]:
        write_json(FIXTURES + logical, single_case(binary, rel, 1))


def serialize_layouts(binary: Path) -> None:
    """Complete seeded layouts read by internal/graphjson/serialize.rs."""
    def cases(logical: str, rels: list[str], short: bool = False) -> None:
        rows = [upstream_case(binary, rel, 1, rel.split("/")[-2] if short else None) for rel in rels]
        write_json(FIXTURES + logical, {"source_commit": SOURCE_COMMIT, "cases": rows})
    cases("cluster-policy-source-v090.json", [LARGE_NESTED])
    cases("crossing-cache-source-v090.json", [TESTDATA + "layout/sequence/graph.input.json"])
    cases("hub-incidence-source-v090.json",
          [TESTDATA + f"layout/{name}/graph.input.json"
           for name in ("dense", "hierarchy_inside_containers", "only_ports_to_center")], short=True)
    cases("tree-mirror-source-v090.json",
          [TESTDATA + "layout/deterministic_tree_placement/graph.input.json",
           TESTDATA + "layout/tree-n-seq/graph.input.json",
           TESTDATA + "layout/tree_sentinel_merge/graph.input.json",
           TESTDATA + "regression/container_escape_2.json"])
    rows = [upstream_case(binary, rel, 1) for rel in [
        TESTDATA + "benchmark/18tables.json", CLOUD_CLUSTER,
        TESTDATA + "layout/dejitter_sequence/graph.input.json",
        TESTDATA + "layout/hierarchy_4_levels_1_source/graph.input.json",
        TESTDATA + "layout/no_cluster_straight_edge/graph.input.json"]]
    # Hand-built graphs: nullable identities, styles, zero edge ids, label
    # text, directions and nears that the upstream testdata does not cover.
    for seed_case in json.loads((ROOT / "tools/fixtures/seeds/tala-live-graphjson.json").read_text()):
        stdout = run_twice([str(binary), "-operation", "layout", "-seed", str(seed_case["seed"])],
                           json.dumps(seed_case["input"]).encode())
        rows.append({**seed_case, "source": json.loads(stdout)})
    write_json(FIXTURES + "live-graphjson-source-v090.json", {"source_commit": SOURCE_COMMIT, "cases": rows})
    write_json(FIXTURES + "seeded-cluster-sequence-publication-source-v090.json", {
        "source_commit": SOURCE_COMMIT,
        "cases": [upstream_case(binary, rel, seed, name) for name, rel, seed in SEEDED_REPLAYS]})


def layout_errors(binary: Path) -> None:
    """Single-node layouts at and beyond the Prescale dimension limit: exit
    status, stdout and stderr of the source oracle for two seeds."""
    rows = []
    for dimension in ("width", "height"):
        for value in (100, 30000, 30001, 60000):
            for seed in (1, 42):
                node = {"id": 1, "width": 20, "height": 20, "topLeft": {"x": 0, "y": 0}, "shapeType": "rectangle"}
                node[dimension] = value
                graph = {"nodes": [node], "edges": []}
                argv = [str(binary), "-operation", "layout", "-seed", str(seed)]
                runs = [subprocess.run(argv, input=json.dumps(graph).encode(), capture_output=True, timeout=600)
                        for _ in range(2)]
                assert all((r.returncode, r.stdout, r.stderr) == (runs[0].returncode, runs[0].stdout, runs[0].stderr)
                           for r in runs), f"{dimension}-{value}-seed{seed}: repeated runs differ"
                rows.append({"name": f"{dimension}-{value}-seed{seed}", "seed": seed, "input": graph,
                             "source_exit": runs[0].returncode, "source_stdout": runs[0].stdout.decode(),
                             "source_stderr": runs[0].stderr.decode()})
    write_json(FIXTURES + "graphjson-layout-errors-source-v090.json", rows)


def codec_inputs() -> list[dict]:
    """Arrowhead-label positions: every canonical label.Position name parsed
    by lib/label FromString at the pin, plus unknown/near-miss spellings, on
    source, target or both arrowheads; and absent/null/empty labels."""
    source = (upstream() / "lib/label/label.go").read_text()
    source = source.split("func FromString", 1)[1].split("func (position", 1)[0]
    canonical = re.findall(r'case "([A-Z_]+)":', source)
    assert len(canonical) == 36, len(canonical)
    positions = ["UNSET", *canonical, "", "unknown", "outside_top_left", "OutsideTopCenter", "inside-middle-center",
                 " UNLOCKED_TOP", "UNLOCKED_BOTTOM ", "none", "0", "∅"]
    graph = {"nodes": [{"id": 1, "d2id": "left", "width": 100, "height": 100, "shapeType": "Square",
                        "topLeft": {"x": 0, "y": 0}},
                       {"id": 2, "d2id": "right", "width": 100, "height": 100, "shapeType": "Square",
                        "topLeft": {"x": 300, "y": 200}}],
             "edges": [{"id": 9, "from": 1, "to": 2, "sourceArrowhead": "triangle", "targetArrowhead": "diamond",
                        "points": [{"x": 100, "y": 50}, {"x": 120, "y": 50}, {"x": 120, "y": 250},
                                   {"x": 300, "y": 250}]}],
             "containers": {"0": [1, 2]}}
    rows = []
    for placement in ["source", "target", "both"]:
        for index, position in enumerate(positions):
            g = copy.deepcopy(graph)
            for side in (["source", "target"] if placement == "both" else [placement]):
                g["edges"][0][side + "ArrowheadLabel"] = {"Text": "a→b\n✓", "Position": position,
                                                         "Width": 20.25, "Height": 8.5}
            rows.append({"name": f"{placement}-{index:02}", "graph": g})
    for placement in ["absent", "null", "empty"]:
        g = copy.deepcopy(graph)
        if placement != "absent":
            for side in ["source", "target"]:
                g["edges"][0][side + "ArrowheadLabel"] = None if placement == "null" else {}
        rows.append({"name": placement, "graph": g})
    return rows


def codec(_args) -> None:
    binary = build("tools/engine-oracle", "engine-oracle")
    cases = []
    for request in codec_inputs():
        stdout = run_twice([str(binary), "-operation", "roundtrip"], json.dumps(request["graph"]).encode())
        cases.append({"name": request["name"], "input": request["graph"], "source": decode_output(stdout)})
    write_json(FIXTURES + "arrowhead-label-codec-source-v090.json", {
        "source_commit": SOURCE_COMMIT,
        "source_binary_sha256": sha256(binary.read_bytes()),
        "scope": "Original Go graphjson.Unmarshal followed by graphjson.Marshal; full graph records, "
                 "both repeated source runs identical.",
        "cases": cases}, ascii=False)


def capture_stages(rel: str, seed: int, last: str) -> list[dict]:
    """GraphJSON graph after each default-plan stage up to `last` (hook
    tools/fixtures/hooks/tala-tree-publication), captured twice."""
    hook = ROOT / "tools/fixtures/hooks/tala-tree-publication/d2rs_tree_publication_test.go"
    package = "d2layouts/d2talalayout/internal/engine"
    overlay = workdir() / "stages-overlay.json"
    overlay.write_text(json.dumps({"Replace": {str(upstream() / package / hook.name): str(hook)}}))
    runs = []
    for attempt in range(2):
        out = workdir() / f"stages-{attempt}.json"
        env = {**go_env(), "D2RS_TALA_STAGES_OUTPUT": str(out), "D2RS_TALA_STAGES_INPUT": str(upstream() / rel),
               "D2RS_TALA_STAGES_SEED": str(seed), "D2RS_TALA_STAGES_LAST": last}
        subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay),
                        "-run", "^TestD2RSTreePublication$", f"./{package}"],
                       cwd=upstream(), env=env, check=True, timeout=1200)
        runs.append(out.read_bytes())
        out.unlink()
    assert runs[0] == runs[1], f"{rel} seed {seed}: repeated stage captures differ"
    return json.loads(runs[0])


def stage_graph(records: list[dict], name: str) -> dict:
    matches = [r["graph"] for r in records if r["stage"] == name]
    assert len(matches) == 1, f"stage {name}: {len(matches)} records"
    return matches[0]


def stages(args) -> None:
    data, digest = upstream_input(args.input)
    records = capture_stages(args.input, args.seed, args.last)
    write_json(FIXTURES + "tree-publication-source-v090.json", {
        "source_commit": SOURCE_COMMIT, "source_input": args.input, "source_input_sha256": digest,
        "seed": args.seed, "input": json.loads(data), "stages": records}, compact=True)


PREPROCESS_STAGES = ["Prescale", "PreprocessSequences", "Preprocess", "PreprocessTrees",
                     "PreprocessHierarchies", "PreprocessClusters", "PreprocessHubs"]


def stage_sets(args) -> None:
    if args.set == "cluster-creation":
        # cloud_contains_cluster: cluster vessel creation and edge abduction
        # (PreprocessClusters) and the following hub stage, four seeds. The
        # input is the upstream file itself.
        data, _ = upstream_input(CLOUD_CLUSTER)
        output(FIXTURES + "cluster-creation/input.json").write_bytes(data)
        for seed in (1, 2, 7, 42):
            records = capture_stages(CLOUD_CLUSTER, seed, "PreprocessHubs")
            for name in ("PreprocessClusters", "PreprocessHubs"):
                write_json(FIXTURES + f"cluster-creation/{seed}-{name}.json", stage_graph(records, name))
    elif args.set == "preprocess":
        rows = []
        for name, rel, seed in SEEDED_REPLAYS:
            data, _ = upstream_input(rel)
            records = capture_stages(rel, seed, "PreprocessHubs")
            rows.append({"name": name, "seed": seed, "upstream": rel, "input": json.loads(data),
                         "stages": [{"event": "full-graphjson-stage", "graph": stage_graph(records, stage),
                                     "stage": stage} for stage in PREPROCESS_STAGES]})
        write_json(FIXTURES + "preprocess-stage-serialization-source-v090.json", rows)
    elif args.set == "nested-cluster-initialization":
        data, _ = upstream_input(LARGE_NESTED)
        write_json(FIXTURES + "nested-cluster-initialization-source-v090.json", {
            "source_commit": SOURCE_COMMIT, "upstream": LARGE_NESTED, "input_text": data.decode(),
            "cases": [{"seed": seed, "source": stage_graph(capture_stages(LARGE_NESTED, seed, "NodePlacement"),
                                                           "NodePlacement")} for seed in (1, 42)]})
    else:
        sys.exit(f"unknown stage set {args.set}")


def route_validation_inputs() -> list[dict]:
    """Two-node graphs whose edge routes are missing, single-point,
    degenerate, diagonal, non-finite or beyond the source resource limits,
    routed through RouteEdges (selected edges, duplicates) or Layout, with
    and without a cancelled context; plus error-precedence probes."""
    def route(points=None, count=0, tail=None):
        return {"points": points or [], "count": count, "tail": tail}
    valid = route([["110", "50"], ["250", "50"]])
    variants = [("valid", [valid, valid]), ("missing-first", [route(), valid]), ("missing-second", [valid, route()]),
                ("missing-both", [route(), route()]), ("single-first", [route([["110", "50"]]), valid]),
                ("single-second", [valid, route([["110", "50"]])]),
                ("degenerate", [route([["110", "50"], ["110", "50"]]), valid]),
                ("signed-zero-degenerate", [route([["0", "-0"], ["-0", "0"]]), valid]),
                ("nonadjacent-repeat", [route([["110", "50"], ["120", "50"], ["110", "50"]]), valid]),
                ("diagonal", [route([["110", "50"], ["250", "51"]]), valid])]
    for axis in [0, 1]:
        for point in [0, 1]:
            for value in ["NaN", "+Inf", "-Inf", "1000000000.5", "-1000000000.5", "1000000000", "-1000000000"]:
                points = copy.deepcopy(valid["points"])
                points[point][axis] = value
                variants.append((f"point-{point}-axis-{axis}-{value}", [route(points), valid]))
    variants += [("xy-order", [route([["NaN", "+Inf"], ["250", "50"]]), valid]),
                 ("edge-order", [route([["110", "50"]]), route([["NaN", "50"], ["250", "50"]])])]
    inputs = []
    for name, routes in variants:
        for mode, selected in [("empty", []), ("first", [0]), ("second", [1]), ("all", [0, 1]),
                               ("duplicate", [0, 0]), ("layout", [])]:
            for cancel in [False, True]:
                inputs.append({"name": f"{name}-{mode}-{cancel}", "routes": routes, "requested": selected,
                               "cancel": cancel, "layout": mode == "layout", "arrowhead": "",
                               "missing_position": False})
    # Resource-limit and ordering probes stop at validation; exactly-at-limit
    # controls carry an invalid last point, so no million-point layout runs.
    resources = [("over-first", [route(count=1000001, tail=["NaN", "0"]), valid]),
                 ("over-aggregate", [route(count=999998), route(count=3)]),
                 ("limit-tail", [route(count=999998), route(count=2, tail=["NaN", "0"])]),
                 ("limit-singleton", [route(count=999999), route(count=1)]),
                 ("earlier-error-before-total", [route([["NaN", "0"], ["1", "0"]]), route(count=1000001)])]
    for name, routes in resources:
        for layout_mode in [False, True]:
            for cancel in [False, True]:
                inputs.append({"name": f"{name}-layout{layout_mode}-cancel{cancel}", "routes": routes,
                               "requested": [], "cancel": cancel, "layout": layout_mode, "arrowhead": "",
                               "missing_position": False})
    for arrowhead, missing in [("mystery", False), ("", True)]:
        for selected in [[], [0, 0]]:
            inputs.append({"name": f"precedence-{arrowhead}-{missing}-{selected}",
                           "routes": [route([["NaN", "0"], ["1", "0"]]), valid], "requested": selected,
                           "cancel": False, "layout": False, "arrowhead": arrowhead, "missing_position": missing})
    return inputs


def route_validation(_args) -> None:
    binary = build("tools/tala-route-validation-oracle", "route-validation")
    inputs = route_validation_inputs()
    stdout = run_twice([str(binary)], (json.dumps(inputs, indent=2) + "\n").encode())
    rows = json.loads(stdout)
    assert len(rows) == len(inputs)
    write_json(FIXTURES + "route-validation-source-v090.json",
               {"source_commit": SOURCE_COMMIT, "source_sha256": sha256(stdout), "cases": rows})


def requested_routing_inputs() -> list[dict]:
    """Requested-edge lists for RouteEdges on a graph of N routed edges:
    order, repeats, a foreign edge (index N), more requests than edges, the
    50000 request limit and the 50000 graph-edge limit, each with and without
    a cancelled context."""
    variants = [("empty", 2, [], False), ("first", 2, [0], False), ("both", 2, [0, 1], False),
                ("reverse", 2, [1, 0], False), ("duplicate", 2, [0, 0], False),
                ("duplicate-foreign", 2, [2, 2], False), ("foreign-first", 2, [2, 0], False),
                ("foreign-last", 2, [0, 2], False), ("exceeds-inventory", 2, [0, 1, 2], False),
                ("duplicate-exceeds-inventory", 2, [0, 1, 0], False), ("exceeds-limit", 2, [], True),
                ("empty-inventory", 0, [0], False), ("oversized-graph-empty", 50001, [], False),
                ("oversized-graph-duplicate", 50001, [0, 0], False), ("oversized-graph-first", 50001, [0], False)]
    return [{"name": f"{name}-{'cancel' if cancel else 'control'}", "graph_edges": edges, "requested": requested,
             "oversized": oversized, "cancel": cancel}
            for name, edges, requested, oversized in variants for cancel in (False, True)]


def route_geometry_inputs() -> list[dict]:
    """Object geometry variants (tools/fixtures/seeds/tala-route-geometry.json)
    routed with all/no/repeated requested edges, with and without
    cancellation; coordinate-limit boundaries; and seed lists RouteEdges
    must ignore."""
    variants = json.loads((ROOT / "tools/fixtures/seeds/tala-route-geometry.json").read_text())
    inputs = []
    for variant in variants:
        for mode, requested in [("all", [0]), ("empty", []), ("duplicate", [0, 0])]:
            for cancel in (False, True):
                inputs.append({"name": f"{variant['name']}-{mode}-{'cancel' if cancel else 'control'}",
                               "nodes": variant["nodes"], "visit_order": variant["visit_order"],
                               "requested": requested, "seeds": [1], "cancel": cancel})
    def nodes(a_position):
        return [{"id": "a", "position": a_position, "width": "100", "height": "60"},
                {"id": "b", "position": ["250", "20"], "width": "100", "height": "60"}]
    for name, position in [("max-x", ["999999900", "20"]), ("min-x", ["-1000000000", "20"]),
                           ("max-y", ["10", "999999940"]), ("min-y", ["10", "-1000000000"])]:
        inputs.append({"name": name, "nodes": nodes(position), "visit_order": [0, 1], "requested": [],
                       "seeds": [1], "cancel": False})
    for name, seeds in [("empty", []), ("oversized", [1] * 65), ("unique", list(range(1, 18))), ("alternate", [7])]:
        for mode, requested in [("empty", []), ("all", [0])]:
            inputs.append({"name": f"ignored-seeds-{name}-{mode}", "nodes": nodes(["10", "20"]),
                           "visit_order": [0, 1], "requested": requested, "seeds": seeds, "cancel": False})
    return inputs


def route_geometry(_args) -> None:
    binary = build("tools/tala-route-geometry-oracle", "route-geometry")
    request = {"requested_routing": requested_routing_inputs(), "route_geometry": route_geometry_inputs()}
    stdout = run_twice([str(binary)], (json.dumps(request, indent=2) + "\n").encode())
    rows = json.loads(stdout)
    for key, logical in [("requested_routing", "requested-routing-source-v090.json"),
                         ("route_geometry", "route-geometry-source-v090.json")]:
        assert len(rows[key]) == len(request[key]), key
        write_json(FIXTURES + logical, {"source_commit": SOURCE_COMMIT, "source_sha256": sha256(stdout),
                                        "cases": rows[key]})


AUTHORED_OBJECT_IDS = [b"a", b'"a.b"', b'"a\\"b"', b"bad\xff"]
AUTHORED_VALUES = [
    b"", b"0", b"+0", b"-0", b"1", b"+1", b"01", b"-1", b"1000000000", b"1000000001", b"-1000000000",
    b"-1000000001", b"9223372036854775807", b"9223372036854775808", b"-9223372036854775808",
    b"-9223372036854775809", b"1.5", b"1e2", b"0x10", b"1_000", b" 1", b"1 ", b"1\n", b"+", b"--1", b"\xff",
    b"\x00", "１２".encode(), b"0" * 100, b"0" * 100 + b"1",
]


def authored_geometry_requests() -> None:
    """Every D2 width/height/top/left attribute spelling above on four object
    ids (top and left paired with a valid partner), plus pairing/order
    precedence probes; bytes as integer lists (Go strings need not be UTF-8)."""
    def ints(value: bytes | None):
        return None if value is None else list(value)
    rows = []
    for object_id in AUTHORED_OBJECT_IDS:
        for field in ("Width", "Height", "Top", "Left"):
            for index, value in enumerate(AUTHORED_VALUES):
                request = {"Name": f"{object_id.hex()}-{field}-{index:02}", "ObjectID": ints(object_id),
                           "Width": None, "Height": None, "Top": None, "Left": None}
                request[field] = ints(value)
                partner = {"Top": "Left", "Left": "Top"}.get(field)
                if partner:
                    request[partner] = ints(b"1")
                rows.append(request)
    for index, (top, left, width, height) in enumerate([
            (None, None, None, None), (b"1", None, b"0", b"0"), (None, b"1", b"0", b"0"),
            (b"bad", b"0", b"0", b"0"), (b"0", b"bad", b"0", b"0"), (None, None, b"0", b"bad"),
            (None, None, b"1", b"0")]):
        rows.append({"Name": f"precedence-{index}", "ObjectID": ints(b"object"), "Top": ints(top),
                     "Left": ints(left), "Width": ints(width), "Height": ints(height)})
    path = Path(os.environ["D2RS_RECIPE_OUTPUT"])
    path.write_text(json.dumps(rows, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("layout")
    p.add_argument("set", choices=["label-obstacles", "tree-containers", "graphjson-owners", "serialize", "errors"])
    p.set_defaults(func=layout)
    p = sub.add_parser("stage-sets")
    p.add_argument("set", choices=["cluster-creation", "preprocess", "nested-cluster-initialization"])
    p.set_defaults(func=stage_sets)
    sub.add_parser("codec").set_defaults(func=codec)
    p = sub.add_parser("stages")
    p.add_argument("--input", required=True, help="upstream-relative GraphJSON input")
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--last", required=True, help="last default-plan stage to run")
    p.set_defaults(func=stages)
    sub.add_parser("route-validation").set_defaults(func=route_validation)
    sub.add_parser("route-geometry").set_defaults(func=route_geometry)
    sub.add_parser("authored-geometry-requests").set_defaults(func=lambda _args: authored_geometry_requests())
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
