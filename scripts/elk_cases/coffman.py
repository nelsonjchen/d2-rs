"""elk-go Layered Coffman-Graham matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 Coffman-Graham probes (probe-cg-matrix.py,
probe-extra-controls.py).
"""
import copy

from elk_cases import curated

DIRECTIONS = ["RIGHT", "LEFT", "DOWN", "UP"]
PROFILES = {"chain": (4, [(0, 1), (1, 2), (2, 3), (0, 3)]),
            "diamond": (4, [(0, 1), (0, 2), (1, 3), (2, 3), (0, 3)]),
            "parallel": (4, [(0, 1), (0, 1), (1, 2), (0, 2), (2, 3)]),
            "disconnected": (6, [(0, 1), (1, 2), (3, 4), (4, 5)]),
            "edgeless": (5, []),
            "labels": (4, [(0, 1), (0, 2), (1, 3), (2, 3), (0, 3)]),
            "nested": (3, [(0, 1), (1, 2), (0, 2)])}


def curated_coffman_graham():
    """elk-go curated `15_coffman_graham`: 11 nodes, 16 edges, string-valued spacing options."""
    return curated.load("15_coffman_graham")


def _matrix(profiles):
    for profile in profiles:
        count, pairs = PROFILES[profile]
        for direction in DIRECTIONS:
            for bound in [None, 1, 3]:
                for seed in [1, 42]:
                    for mode in ["NONE", "NODES_AND_EDGES"]:
                        for hierarchy in ([True] if profile == "nested" else [False, True]):
                            opts = {"elk.algorithm": "layered", "elk.direction": direction,
                                    "elk.layered.layering.strategy": "COFFMAN_GRAHAM",
                                    "elk.layered.considerModelOrder.strategy": mode,
                                    "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER",
                                    "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED", "elk.randomSeed": seed}
                            if hierarchy:
                                opts["elk.hierarchyHandling"] = "INCLUDE_CHILDREN"
                            if bound is not None:
                                opts["elk.layered.layering.coffmanGraham.layerBound"] = bound
                            ids = [f"box.n{i}" if profile == "nested" else f"n{i}" for i in range(count)]
                            nodes = [{"id": name, "width": 30 + 4 * i, "height": 24 + 2 * i} for i, name in enumerate(ids)]
                            edges = [{"id": f"e{i}", "sources": [ids[a]], "targets": [ids[b]]}
                                     for i, (a, b) in enumerate(pairs)]
                            if profile == "labels":
                                for i, e in enumerate(edges):
                                    e["labels"] = [{"text": f"l{i}", "width": 14 + 2 * i, "height": 8 + 2 * i,
                                                    "layoutOptions": {"elk.edgeLabels.inline": seed == 1}}]
                            if profile == "nested":
                                nodes = [{"id": "box", "width": 80, "height": 60, "children": nodes,
                                          "layoutOptions": copy.deepcopy(opts)}, {"id": "outside", "width": 40, "height": 30}]
                                edges.insert(0, {"id": "entry", "sources": ["outside"], "targets": [ids[0]]})
                            yield dict(name=f"cg-{profile}-{direction.lower()}-bound{bound}-seed{seed}-{mode}-hier{hierarchy}",
                                       input={"id": "root", "layoutOptions": opts, "children": nodes, "edges": edges})


def native_coffman_graham_controls():
    return list(_matrix(PROFILES))


def coffman_option_forms():
    rows = []
    base = curated_coffman_graham()
    for direction in DIRECTIONS:
        for key in ["org.eclipse.elk.layered.layering.strategy", "elk.layered.layering.strategy",
                    "layered.layering.strategy"]:
            for form in ["COFFMAN_GRAHAM", "3", 3]:
                for suffixed in [False, True]:
                    inp = copy.deepcopy(base)
                    opts = inp["layoutOptions"]
                    opts.pop("elk.layered.layering.strategy")
                    opts[key] = form
                    opts["elk.direction"] = direction
                    opts["elk.layered.layering.coffmanGraham.layerBound"] = " 3 " if suffixed else 3
                    if suffixed:
                        for name in ["elk.layered.spacing.edgeEdgeBetweenLayers", "elk.spacing.edgeNode",
                                     "elk.spacing.nodeSelfLoop", "spacing.edgeNodeBetweenLayers",
                                     "spacing.nodeNodeBetweenLayers"]:
                            opts[name] = f" {float(opts[name])}D "
                    rows.append(dict(name=f"cg-option-{direction.lower()}-{key}-form{repr(form)}-suffix{suffixed}",
                                     input=inp))
    return rows


def component_single_transfer():
    rows = []
    for row in _matrix(["parallel"]):
        inp = copy.deepcopy(row["input"])
        inp["layoutOptions"]["elk.layered.layering.strategy"] = "NETWORK_SIMPLEX"
        rows.append(dict(name="ns-" + row["name"], input=inp))
    for direction in DIRECTIONS:
        for strategy in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]:
            for label_count in [1, 3]:
                for seed in [1, 42]:
                    inp = {"id": "root",
                           "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction,
                                             "elk.layered.layering.strategy": strategy, "elk.randomSeed": seed,
                                             "elk.spacing.nodeSelfLoop": 15.73, "elk.spacing.edgeLabel": 2.37,
                                             "elk.spacing.labelLabel": 1.13},
                           "children": [{"id": "a", "width": 42.57, "height": 37.13}],
                           "edges": [{"id": "loop", "sources": ["a"], "targets": ["a"],
                                      "labels": [{"text": f"L{i}", "width": 11.23 + 1.5 * i, "height": 3.57 + 0.25 * i}
                                                 for i in range(label_count)]}]}
                    rows.append(dict(name=f"single-loop-{direction.lower()}-{strategy}-labels{label_count}-seed{seed}",
                                     input=inp))
    return rows
