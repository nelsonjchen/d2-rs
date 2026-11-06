"""elk-go Layered option/hierarchy matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each function returns the [{name, input}] cases of one
crates/d2-rs-elk/src/fixtures/original_<function>.json fixture, in fixture
order. They are recovered from the 2026-10-16..19 ELK qualification probes
(.reference/elk-*-2026101x/probe-*.py); where a probe built on an earlier
probe's inputs, the earlier generator is inlined here.
"""
import copy
import random

DIRECTIONS = ["RIGHT", "LEFT", "DOWN", "UP"]


def _dense_base(seed, direction="RIGHT", strategy="NETWORK_SIMPLEX", model="NODES_AND_EDGES", count=6):
    """Seeded DAG (odd seeds) or cyclic graph (even seeds): 6 nodes (dense probe) or 10/14 (large probe)."""
    rng = random.Random(seed)
    density = 0.35 if count == 6 else (0.20 if seed % 2 == 0 else 0.30)
    pairs = [(i, j) for i in range(count) for j in range(count) if i != j and (seed % 2 == 0 or i < j)
             and rng.random() < density]
    return {"id": "root",
            "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction,
                              "elk.layered.layering.strategy": strategy,
                              "elk.layered.layering.coffmanGraham.layerBound": 3,
                              "elk.layered.considerModelOrder.strategy": model, "elk.randomSeed": 42},
            "children": [{"id": f"n{i}", "width": 30 + i * 3.25, "height": 20 + i * 2.75} for i in range(count)],
            "edges": [{"id": f"e{k}", "sources": [f"n{i}"], "targets": [f"n{j}"]} for k, (i, j) in enumerate(pairs)]}


def dense_component_trials():
    rows = []
    for seed in range(1, 9):
        for direction in DIRECTIONS:
            for strategy in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]:
                for model in ["NONE", "NODES_AND_EDGES"]:
                    rows.append(dict(name=f"dense-seed{seed}-{direction.lower()}-{strategy}-{model}",
                                     input=_dense_base(seed, direction, strategy, model)))
    return rows


def component_trial_limits():
    rows = []
    for included in [False, True]:
        for seed in [4, 7]:
            base = _dense_base(seed)
            for direction in DIRECTIONS:
                for strategy in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]:
                    for model in ["NONE", "NODES_AND_EDGES"]:
                        for thoroughness in [0, 1, 2, 3, 7]:
                            inp = copy.deepcopy(base)
                            inp["layoutOptions"].update({"elk.direction": direction,
                                                         "elk.layered.layering.strategy": strategy,
                                                         "elk.layered.considerModelOrder.strategy": model,
                                                         "elk.layered.thoroughness": thoroughness})
                            name = f"trials-seed{seed}-{direction.lower()}-{strategy}-{model}-t{thoroughness}"
                            if included:
                                inp["layoutOptions"]["elk.hierarchyHandling"] = "INCLUDE_CHILDREN"
                                name = "included-" + name
                            rows.append(dict(name=name, input=inp))
    return rows


def p1_component_discovery():
    rows = []
    for seed in range(1, 9):
        count = 10 if seed <= 4 else 14
        for direction in DIRECTIONS:
            for strategy in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]:
                for model in ["NONE", "NODES_AND_EDGES"]:
                    rows.append(dict(name=f"large-n{count}-seed{seed}-{direction.lower()}-{strategy}-{model}",
                                     input=_dense_base(seed, direction, strategy, model, count)))
    return rows


def _pair(family, base, direction, layering, cycle, seed, interleaved):
    """Two prefixed copies (a., b.) of one component, listed in sequence or interleaved."""
    edges = []
    groups = []
    for prefix in ["a.", "b."]:
        nodes = copy.deepcopy(base["children"])
        for n in nodes:
            n["id"] = prefix + n["id"]
        groups.append(nodes)
        for e in copy.deepcopy(base["edges"]):
            e["id"] = prefix + e["id"]
            e["sources"] = [prefix + x for x in e["sources"]]
            e["targets"] = [prefix + x for x in e["targets"]]
            edges.append(e)
    children = [n for pair in zip(*groups) for n in pair] if interleaved else groups[0] + groups[1]
    inp = {"id": "root", "layoutOptions": dict(base["layoutOptions"]) | {
        "elk.direction": direction, "elk.layered.layering.strategy": layering,
        "elk.layered.cycleBreaking.strategy": cycle, "elk.randomSeed": seed}, "children": children, "edges": edges}
    return dict(name=f"pair-{family}-{direction.lower()}-{layering}-{cycle}-seed{seed}-interleave{interleaved}", input=inp)


PAIR_BASES = {"six": lambda: _dense_base(4), "ten": lambda: _dense_base(4, count=10)}


def component_lifecycle():
    return [_pair(family, PAIR_BASES[family](), direction, layering, cycle, seed, interleaved)
            for family in ["six", "ten"]
            for direction in ["RIGHT", "DOWN"]
            for layering in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]
            for cycle in ["GREEDY", "GREEDY_MODEL_ORDER"]
            for seed in [1, 42]
            for interleaved in [False, True]]


def mixed_component_lifecycles():
    bases = [_pair(family, PAIR_BASES[family](), "RIGHT", "NETWORK_SIMPLEX", "GREEDY", 1, False)["input"]
             for family in ["six", "ten"]]
    rows = []
    for direction in ["RIGHT", "DOWN", "LEFT", "UP"]:
        for layering in ["NETWORK_SIMPLEX", "COFFMAN_GRAHAM"]:
            for cycle in ["GREEDY", "GREEDY_MODEL_ORDER"]:
                for seed in [1, 42]:
                    for interleaved in [False, True]:
                        for decorated in [False, True]:
                            groups = []
                            edges = []
                            for prefix, base in zip(["a.", "b."], bases):
                                nodes = [copy.deepcopy(n) for n in base["children"] if n["id"].startswith("a.")]
                                es = [copy.deepcopy(e) for e in base["edges"] if e["id"].startswith("a.")]
                                for n in nodes:
                                    n["id"] = prefix + n["id"][2:]
                                for e in es:
                                    e["id"] = prefix + e["id"][2:]
                                    e["sources"] = [prefix + x[2:] for x in e["sources"]]
                                    e["targets"] = [prefix + x[2:] for x in e["targets"]]
                                if decorated:
                                    es[0]["labels"] = [dict(id=prefix + "center", text="center", width=27.25, height=9.5)]
                                    es.append(dict(id=prefix + "loop", sources=[nodes[0]["id"]], targets=[nodes[0]["id"]],
                                                   labels=[dict(id=prefix + "looplabel", text="loop", width=11.5,
                                                                height=6.25)]))
                                groups.append(nodes)
                                edges.extend(es)
                            children = ([n for i in range(max(map(len, groups))) for group in groups for n in group[i:i + 1]]
                                        if interleaved else groups[0] + groups[1])
                            children.insert(3, dict(id="isolate-small", width=13.25, height=8.5))
                            children.append(dict(id="isolate-large", width=81.75, height=53.25))
                            inp = dict(id="root", layoutOptions=dict(bases[0]["layoutOptions"]) | {
                                "elk.direction": direction, "elk.layered.layering.strategy": layering,
                                "elk.layered.cycleBreaking.strategy": cycle, "elk.randomSeed": seed},
                                       children=children, edges=edges)
                            rows.append(dict(name=f"mixed-{direction.lower()}-{layering}-{cycle}-seed{seed}-"
                                                  f"interleave{interleaved}-decor{decorated}", input=inp))
    return rows
