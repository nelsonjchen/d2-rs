"""elk-go Layered hierarchy matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-16..17 hierarchy and root-wire probes. Base graphs
are elk-go's curated testdata or the small D2-adapter wire graph below.
"""
import copy
import itertools
import json

from elk_cases import curated


def _owned_grandchild(labelled):
    """`{labelled,plain}-right-grandchild-owned`: an INCLUDE_CHILDREN cluster two levels deep."""
    inside = {"id": "inside", "sources": ["cluster.a"], "targets": ["cluster.b"]}
    if labelled:
        inside["labels"] = [{"text": "inside", "width": 36, "height": 12}]
    return {"id": "root",
            "layoutOptions": {"elk.algorithm": "layered", "elk.direction": "RIGHT",
                              "elk.hierarchyHandling": "INCLUDE_CHILDREN"},
            "children": [{"id": "outer", "width": 0, "height": 0, "children": [
                {"id": "cluster", "width": 180, "height": 120,
                 "layoutOptions": {"elk.direction": "RIGHT", "elk.padding": "[top=30,left=30,bottom=30,right=30]"},
                 "children": [{"id": "cluster.a", "width": 55, "height": 35},
                              {"id": "cluster.b", "width": 65, "height": 40}],
                 "edges": [inside]}]},
                         {"id": "outside", "width": 90, "height": 50}],
            "edges": [{"id": "cluster-outside", "sources": ["cluster"], "targets": ["outside"]}]}


def hierarchy_resize_depths():
    rows = []
    for direction in ["RIGHT", "DOWN", "LEFT", "UP"]:
        for wrappers in range(4):
            for size, (width, height) in [("zero", (0, 0)), ("shrink", (180, 120)), ("large", (420, 250))]:
                for labelled in [False, True]:
                    for incoming in [False, True]:
                        tag = "labelled" if labelled else "plain"
                        g = _owned_grandchild(labelled)
                        g["layoutOptions"]["elk.direction"] = direction
                        cluster = g["children"][0]["children"][0]
                        cluster["width"] = width
                        cluster["height"] = height
                        child = cluster
                        for index in range(wrappers):
                            child = dict(id=f"wrapper{index}", width=0, height=0, children=[child])
                        g["children"][0] = child
                        if incoming:
                            edge = g["edges"][0]
                            edge["sources"], edge["targets"] = edge["targets"], edge["sources"]
                        rows.append(dict(name=f"resize-{direction.lower()}-depth{wrappers}-{size}-{tag}-" +
                                              ("incoming" if incoming else "outgoing"), input=g))
    return rows


def _cross_hierarchy():
    """elk-go curated `11_cross_hierarchy`: two DOWN groups joined by crossed cross-hierarchy edges."""
    return curated.load("11_cross_hierarchy")


def hierarchy_boundary_orders():
    rows = []
    base = _cross_hierarchy()
    for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
        for pair_count in [2, 3]:
            for reverse_root in [False, True]:
                for crossed in [False, True]:
                    for reversed_children in range(4):
                        for seed in [1, 42]:
                            g = copy.deepcopy(base)
                            g["layoutOptions"].update({"elk.direction": direction, "elk.randomSeed": seed})
                            names = ["one", "two", "three"][:pair_count]
                            for index, group in enumerate(g["children"]):
                                group["layoutOptions"]["elk.direction"] = direction
                                group["children"] = [dict(id=group["id"] + "." + name, width=70, height=40)
                                                     for name in names]
                                if reversed_children & (1 << index):
                                    group["children"].reverse()
                            g["edges"] = [dict(id="cross-" + name, sources=["source_group." + name],
                                               targets=["target_group." +
                                                        (names[pair_count - 1 - index] if crossed else name)])
                                          for index, name in enumerate(names)]
                            if reverse_root:
                                g["children"].reverse()
                            rows.append(dict(name=f"boundary-{direction.lower()}-n{pair_count}-root{int(reverse_root)}-"
                                                  + ("cross" if crossed else "straight")
                                                  + f"-children{reversed_children}-seed{seed}", input=g))
    return rows


_D2_NODE_OPTIONS = {"elk.direction": "", "elk.layered.edgeRouting.selfLoopDistribution": "EQUALLY",
                    "elk.spacing.nodeSelfLoop": 0}


def _d2_root_options(direction):
    """Root layoutOptions of D2's ELK adapter (d2layouts/d2elklayout) for a diagram direction."""
    return {"elk.spacing.edgeNode": 40, "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
            "elk.layered.thoroughness": 8, "elk.layered.spacing.edgeEdgeBetweenLayers": 50,
            "elk.direction": direction, "elk.hierarchyHandling": "INCLUDE_CHILDREN",
            "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
            "elk.layered.cycleBreaking.strategy": "GREEDY_MODEL_ORDER",
            "elk.nodeSize.constraints": "MINIMUM_SIZE", "elk.contentAlignment": "H_CENTER V_CENTER",
            "elk.algorithm": "layered", "spacing.nodeNodeBetweenLayers": 70,
            "spacing.edgeNodeBetweenLayers": 40, "elk.spacing.nodeSelfLoop": 50}


def _d2_container_options(minimum):
    return {"elk.spacing.edgeNode": 40, "elk.layered.nodePlacement.bk.fixedAlignment": "BALANCED",
            "elk.layered.thoroughness": 8, "elk.layered.spacing.edgeEdgeBetweenLayers": 50, "elk.direction": "",
            "elk.hierarchyHandling": "INCLUDE_CHILDREN", "elk.layered.considerModelOrder.strategy": "NONE",
            "elk.layered.cycleBreaking.strategy": "GREEDY", "elk.nodeSize.constraints": "MINIMUM_SIZE",
            "elk.contentAlignment": "H_CENTER V_CENTER", "elk.nodeSize.minimum": minimum,
            "spacing.nodeNodeBetweenLayers": 70, "elk.padding": "[top=50,left=50,bottom=50,right=50]",
            "spacing.edgeNodeBetweenLayers": 40, "elk.spacing.nodeSelfLoop": 50}


def _d2_loop_graph(nested):
    """ELK wire graph the D2 adapter emits for `a -> a: loop; a -> b -> c; a -> c; b -> a` (DOWN).

    `nested` puts a, b and c in a zero-size `group`; otherwise a second
    labelled self-loop `extra-loop` is added at the root.
    """
    nodes = [{"id": n, "x": 0, "y": 0, "width": w, "height": 66,
              "labels": [{"text": n, "x": 0, "y": 0, "width": 8, "height": 21}], "layoutOptions": dict(_D2_NODE_OPTIONS)}
             for n, w in [("a", 120), ("b", 80), ("c", 80)]]
    label = lambda text, w, h: [{"text": text, "x": 0, "y": 0, "width": w, "height": h,
                                 "layoutOptions": {"elk.direction": "", "elk.edgeLabels.inline": True,
                                                   "elk.spacing.nodeSelfLoop": 0}}]
    edges = [{"id": "(a -> a)[0]", "sources": ["a"], "targets": ["a"], "labels": label("loop", 29, 21), "container": ""}]
    edges += [{"id": f"({s} -> {t})[0]", "sources": [s], "targets": [t], "container": ""}
              for s, t in [("a", "b"), ("b", "c"), ("a", "c"), ("b", "a")]]
    if not nested:
        edges.append({"id": "extra-loop", "sources": ["a"], "targets": ["a"], "labels": label("extra", 37, 25),
                      "container": ""})
    return {"id": "", "layoutOptions": _d2_root_options("DOWN"),
            "children": [{"id": "group", "width": 0, "height": 0, "children": nodes}] if nested else nodes,
            "edges": edges}


def included_hierarchy_direction():
    """Root/child direction pairs, then with an edge leaving or entering the INCLUDE_CHILDREN group."""
    base = _d2_loop_graph(nested=True)
    plain, crossing = [], []
    for root, child in itertools.product(["DOWN", "RIGHT", "UP", "LEFT"], repeat=2):
        g = json.loads(json.dumps(base))
        g["layoutOptions"]["elk.direction"] = root
        g["children"][0]["layoutOptions"] = {"elk.direction": child}
        plain.append(dict(name=f"{root.lower()}-child-{child.lower()}", input=g))
        for topology in ["outgoing", "incoming"]:
            h = json.loads(json.dumps(g))
            h["children"].append(dict(id="external", width=80, height=66))
            h["edges"].append(dict(id="cross-boundary", sources=["a" if topology == "outgoing" else "external"],
                                   targets=["external" if topology == "outgoing" else "a"]))
            crossing.append(dict(name=f"{topology}-{root.lower()}-child-{child.lower()}", input=h))
    # The captures list each probe directory in file-name order.
    by_name = lambda rows: sorted(rows, key=lambda row: row["name"])
    return by_name(plain) + by_name(crossing)


def root_coordinates():
    """Root x/y forms (absent, null, one, both, -0, fractional) on empty and populated graphs, then root sizes."""
    rows = []
    forms = [("omitted", {}), ("null", {"x": None, "y": None}), ("one", {"x": 42}), ("explicit", {"x": 42, "y": -17}),
             ("negative-zero", {"x": -0.0, "y": -0.0}), ("fraction", {"x": 0.1, "y": -134217728.25})]
    for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
        for kind in ["empty", "nested"]:
            for form, coordinates in forms:
                if kind == "empty":
                    g = {"id": "root", "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction}}
                else:
                    g = _d2_loop_graph(nested=False)
                    g["layoutOptions"]["elk.direction"] = direction
                g.update(coordinates)
                rows.append(dict(name=f"{direction.lower()}-{kind}-{form}", input=g))
    for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
        for form, (width, height) in [("zero", (0, 0)), ("sized", (123.5, 87.25))]:
            rows.append(dict(name=f"{direction.lower()}-empty-{form}-dimensions", input={
                "id": "root", "x": -0.0, "y": 13.75, "width": width, "height": height,
                "layoutOptions": {"elk.algorithm": "layered", "elk.direction": direction,
                                  "elk.padding": "[top=90,left=80,bottom=70,right=60]"}}))
    return rows


def nested_model_order():
    """D2 adapter wire graphs for `outside -> box.(inner.)a -> b -> outside` with the box label outside.

    The adapter sizes the labelled container (label above/below or beside it)
    and writes nodeSize.minimum as (height, width) for vertical and
    (width, height) for horizontal diagram directions.
    """
    rows = []
    for direction in ["DOWN", "RIGHT", "UP", "LEFT"]:
        vertical = direction in ("DOWN", "UP")
        minimum = lambda w, h: f"({h}, {w})" if vertical else f"({w}, {h})"
        for position in ["top", "bottom", "left", "right"]:
            width, height = (88, 122) if position in ("top", "bottom") else (136, 81)
            for depth in ["shallow", "deep"]:
                prefix = "box." if depth == "shallow" else "box.inner."
                leaves = [{"id": prefix + n, "x": 0, "y": 0, "width": 53, "height": 66,
                           "labels": [{"text": n, "x": 0, "y": 0, "width": 8, "height": 21}],
                           "layoutOptions": dict(_D2_NODE_OPTIONS)} for n in ["a", "b"]]
                if depth == "deep":
                    leaves = [{"id": "box.inner", "x": 0, "y": 0, "width": 98, "height": 76, "children": leaves,
                               "labels": [{"text": "inner", "x": 0, "y": 0, "width": 53, "height": 31}],
                               "layoutOptions": _d2_container_options(minimum(98, 76))}]
                box = {"id": "box", "x": 0, "y": 0, "width": width, "height": height, "children": leaves,
                       "labels": [{"text": "Box", "x": 0, "y": 0, "width": 43, "height": 36}],
                       "layoutOptions": _d2_container_options(minimum(width, height))}
                outside = {"id": "outside", "x": 0, "y": 0, "width": 100, "height": 66,
                           "labels": [{"text": "Outside", "x": 0, "y": 0, "width": 55, "height": 21}],
                           "layoutOptions": dict(_D2_NODE_OPTIONS)}
                edges = [{"id": prefix + "(a -> b)[0]", "sources": [prefix + "a"], "targets": [prefix + "b"],
                          "container": ""},
                         {"id": f"(outside -> {prefix}a)[0]", "sources": ["outside"], "targets": [prefix + "a"],
                          "container": ""},
                         {"id": f"({prefix}b -> outside)[0]", "sources": [prefix + "b"], "targets": ["outside"],
                          "container": ""}]
                rows.append(dict(name=f"nested__{direction.lower()}-outside-{position}-center-{depth}",
                                 input={"id": "", "layoutOptions": _d2_root_options(direction),
                                        "children": [box, outside], "edges": edges}))
    return rows
