"""elk-go Layered controls on D2-adapter-shaped graphs (crates/d2-rs-elk/src/model_order_contract.rs).

These fixtures feed elk-go the ELK request that the pinned D2 ELK adapter
(d2layouts/d2elklayout) builds for small D2 sources, sometimes edited
afterwards (root direction, cycle-breaking options, an extra loop, resized
nodes, a wrapper group). The request is captured from the pinned upstream at
recipe time, so it follows the adapter at the pin rather than the 2026-10
v0.9.0 captures these fixtures started from:

- `wires(requests)` builds tools/elk-adapter-wire-oracle in the runner's
  patched upstream tree ($D2RS_UPSTREAM) with an overlaid copy of
  d2layouts/d2elklayout/layout.go whose only change is a call to
  `D2RSWireHook(raw)` right after the ELK graph is marshalled.
- `e2e_scripts(names)` reads upstream e2e case scripts through the manifest
  patch d2-e2e-case-dump.patch (D2RS_E2E_DUMP).

Each public function returns the [{name, input, [raw_input]}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json. D2 sources and edits
were recovered from the 2026-10-16/17 self-loop, coordinate-frame and
north/south probes.
"""
import copy
import functools
import json
import os
from pathlib import Path
import subprocess

from elk_oracle import build_upstream, run
from fixture_recipe import workdir
from references import ROOT

ORACLE = ROOT / "tools/elk-adapter-wire-oracle"
LAYOUT = "d2layouts/d2elklayout/layout.go"
MARSHAL = "\traw, err := json.Marshal(elkGraph)\n\tif err != nil {\n\t\treturn err\n\t}\n"
HOOK_CALL = "\tif D2RSWireHook != nil {\n\t\tD2RSWireHook(raw)\n\t}\n"
DIRECTIONS = ["down", "right", "up", "left"]


@functools.cache
def _oracle():
    upstream = Path(os.environ["D2RS_UPSTREAM"])
    layout = (upstream / LAYOUT).read_text()
    if layout.count(MARSHAL) != 1:
        raise SystemExit(f"{LAYOUT}: the json.Marshal(elkGraph) marker moved; update scripts/elk_cases/adapter_wire.py")
    patched = workdir() / "d2rs-adapter-layout.go"
    patched.write_text(layout.replace(MARSHAL, MARSHAL + HOOK_CALL))
    return build_upstream("d2rs-elk-adapter-wire", "d2rs_elk_adapter_wire", {
        "d2rs_elk_adapter_wire/main.go": ORACLE / "main.go",
        LAYOUT: patched,
        "d2layouts/d2elklayout/d2rs_wire_hook.go": ORACLE / "hook.go.txt",
    })


def wires(requests):
    """{name: the one ELK request JSON text} for [(name, D2 source)]."""
    q = run(_oracle(), json.dumps([dict(name=n, source=s) for n, s in requests]).encode())
    if q.returncode:
        raise SystemExit(q.stderr.decode())
    out = {}
    for row in json.loads(q.stdout):
        if row.get("error") or len(row["wires"]) != 1:
            raise SystemExit(f"{row['name']}: expected one ELK request, got {len(row['wires'])}: {row.get('error')}")
        out[row["name"]] = row["wires"][0]
    return out


def e2e_scripts(names):
    """{e2e case name: script} from the upstream e2e tests (d2-e2e-case-dump.patch)."""
    binary = build_upstream("d2rs-e2e-dump", "e2etests", {}, test=True)
    dump = workdir() / "e2e-dump"
    dump.mkdir(exist_ok=True)
    groups = sorted({n.split("/")[0] for n in names})
    leaves = sorted({n.split("/")[1] for n in names})
    pattern = f"^TestE2E$/^({'|'.join(groups)})$/^({'|'.join(leaves)})$"
    subprocess.run([*binary, "-test.run", pattern, "-test.count=1"], check=True,
                   cwd=Path(os.environ["D2RS_UPSTREAM"]) / "e2etests", env={**os.environ, "D2RS_E2E_DUMP": str(dump)},
                   stdout=subprocess.DEVNULL)
    out = {}
    for name in names:
        path = dump / (name.replace("/", "__") + ".json")
        if not path.exists():
            raise SystemExit(f"e2e case {name} not found in the pinned upstream")
        out[name] = json.loads(path.read_text())["script"]
    return out


def _loads(text):
    return json.loads(text)


SQL = "a: {shape: sql_table; id: int; name: text}\nb: {shape: sql_table; id: int; name: text}\na.id -> b.id\na.name -> c\nc -> b.name"
LOOP = "a -> a: loop\na -> b\nb -> c\na -> c\nb -> a"
TRIANGLE = "a -> b: first\nb -> c: second\nc -> a: return\n"
RECIPROCAL = ("__long_edge_dummy_user: Source\nb: Target\n__long_edge_dummy_user -> b\n"
              "b -> __long_edge_dummy_user\n")
TABLE_CYCLE = "t: {shape: sql_table; id: int; name: text}\nb: Worker\nt.id -> b: send\nb -> t.name: return\n"
# Reduced witness for edges between nested containers (original_nested_edge_container.json).
NESTED_CONTAINER = "a: {\n  k.t -> f.i\n}\nx -> a\n"
ANCESTOR_E2E = [("ancestor-to-child-with-labels", "todo/container_child_edge"),
                ("child-parent-edges", "todo/child_parent_edges"),
                ("descendant-to-ancestor-import-order", "stable/chaos1")]


def _direction(d):
    return f"direction: {d}\n"


def _cycle_requests():
    return [(f"{d}-{name}", _direction(d) + text) for d in DIRECTIONS
            for name, text in [("triangle", TRIANGLE), ("dummy-name-reciprocal", RECIPROCAL),
                               ("table-cycle", TABLE_CYCLE)]]


def _expanded(kind):
    return [(f"expanded__{d}-{kind}", _direction(d) + (SQL if kind == "sql" else LOOP)) for d in DIRECTIONS]


def cycle_geometry():
    graphs = wires(_cycle_requests())
    return [dict(name=name, input=_loads(graphs[name])) for name, _ in _cycle_requests()]


def fixed_sql_ports():
    tables = [(f"{d}-table-cycle", _direction(d) + TABLE_CYCLE) for d in DIRECTIONS]
    requests = tables + _expanded("sql")
    graphs = wires(requests)
    return [dict(name=name, input=_loads(graphs[name])) for name, _ in requests]


def mixed_loop_geometry():
    graphs = wires(_expanded("loop"))
    return [dict(name=name, input=_loads(graphs[name])) for name, _ in _expanded("loop")]


def broader_loop_geometry():
    """The loop graph with a second labelled loop, enlarged nodes and label, or inside a wrapper group."""
    graphs = wires(_expanded("loop"))
    rows = []
    for d in DIRECTIONS:
        base = _loads(graphs[f"expanded__{d}-loop"])
        for kind in ["second-loop", "resized", "nested"]:
            graph = copy.deepcopy(base)
            if kind == "second-loop":
                edge = copy.deepcopy(graph["edges"][0])
                edge["id"] = "extra-loop"
                edge["labels"][0].update(text="extra", width=37, height=25)
                graph["edges"].append(edge)
            elif kind == "resized":
                for node in graph["children"]:
                    node["width"] += 31
                    node["height"] += 17
                graph["edges"][0]["labels"][0].update(text="larger loop", width=71, height=33)
            else:
                graph["children"] = [dict(id="group", width=0, height=0, children=graph["children"])]
            rows.append(dict(name=f"{d}-{kind}", input=graph))
    return rows


def greedy_cycle_geometry():
    """The RIGHT loop graph under GREEDY cycle breaking: seeds, model order on/off, edge direction priority."""
    base = _loads(wires(_expanded("loop"))["expanded__right-loop"])
    rows = []
    variants = [(seed, model, None) for seed in [1, 2, 3, 42] for model in [0, 1]] + [(1, 0, 1), (1, 0, 4)]
    for seed, model, priority in variants:
        graph = copy.deepcopy(base)
        options = graph["layoutOptions"]
        if not model:
            options["elk.layered.considerModelOrder.strategy"] = "NONE"
        options["elk.layered.cycleBreaking.strategy"] = "GREEDY"
        options["elk.randomSeed"] = seed
        if priority is not None:
            graph["edges"][priority]["layoutOptions"] = {"elk.layered.priority.direction": 3}
        rows.append(dict(name=f"seed{seed}-model{model}-priority{priority}", input=graph))
    return rows


def nested_edge_container():
    base = _loads(wires([("nested", NESTED_CONTAINER)])["nested"])
    rows = []
    for d in DIRECTIONS:
        graph = copy.deepcopy(base)
        graph["layoutOptions"]["elk.direction"] = d.upper()
        rows.append(dict(name=f"nested-cross-hierarchy-container-{d}", input=graph))
    return rows


def ancestor_descendant_edges():
    scripts = e2e_scripts([case for _, case in ANCESTOR_E2E])
    graphs = wires([(case, scripts[case]) for _, case in ANCESTOR_E2E])
    return [dict(name=f"{name} ({case})", input=_loads(graphs[case]), raw_input=graphs[case])
            for name, case in ANCESTOR_E2E]
