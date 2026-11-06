"""elk-go SPoRE overlap-removal/compaction controls (crates/d2-rs-elk/src/spore/tests.rs).

Recovered from the 2026-10-19 SPoRE qualification generators.
"""
import copy
import random


def base():
    rows = []

    def add(name, n, opts={}, pattern="scatter", labels=False, ports=False, sections=False):
        nodes = []
        for i in range(n):
            x, y = (i * 11.7, (i % 3) * 8.9) if pattern == "overlap" else (i * 90.3 + (i % 2) * 8.7, (i % 3) * 100.9)
            node = {"id": f"n{i}", "x": x, "y": y, "width": 23.1 + i * 3.7, "height": 17.3 + i * 2.9}
            if labels:
                node["labels"] = [dict(id=f"l{i}", text="abc", x=-13.3, y=-12.7, width=70.1, height=10.9)]
            if ports:
                node["ports"] = [dict(id=f"p{i}", x=-3.7, y=5.9, width=3.7, height=7.1,
                                      layoutOptions={"elk.port.side": "WEST"})]
            nodes.append(node)
        edges = [dict(id=f"e{i}", sources=[f"p{i}" if ports else f"n{i}"], targets=[f"n{i + 1}"]) for i in range(n - 1)]
        if sections:
            for e in edges:
                e.update(sections=[dict(id=e["id"] + "_kept", startPoint={"x": 1.3, "y": 2.7},
                                        endPoint={"x": 3.9, "y": 4.1}, bendPoints=[{"x": 7.3, "y": 9.1}])],
                         labels=[dict(text="edge", x=-1.7, y=3.3, width=12.3, height=5.9)])
        rows.append(dict(name=name, input=dict(id="root", children=nodes, edges=edges, layoutOptions=opts)))

    for alg in ["sporeOverlap", "sporeCompaction"]:
        for n in [0, 1, 2, 3, 5, 9]:
            for pattern in ["scatter", "overlap"]:
                add(f"{alg}-{n}-{pattern}", n, {"elk.algorithm": alg}, pattern)
        for underlying in ["fixed", "box", "mrtree", "radial", "rectpacking", "noSuchProvider"]:
            add(alg + "-under-" + underlying, 6, {"elk.algorithm": alg, "elk.underlyingLayoutAlgorithm": underlying})
        for it in [0, 1, 2, 6]:
            for scan in [False, True]:
                add(f"{alg}-iterations-{it}-{scan}", 7,
                    {"elk.algorithm": alg, "elk.overlapRemoval.maxIterations": it, "elk.overlapRemoval.runScanline": scan},
                    "overlap")
        for feature in ["labels", "ports", "sections", "minimum", "padding"]:
            o = {"elk.algorithm": alg, "elk.spacing.nodeNode": 11.3}
            if feature == "minimum":
                o.update({"elk.nodeSize.constraints": "[MINIMUM_SIZE]", "elk.nodeSize.minimum": "(1000.3,800.7)"})
            if feature == "padding":
                o["elk.padding"] = "[top=3.7,left=6.3,bottom=9.1,right=12.9]"
            add(alg + "-" + feature, 5, o, "overlap", feature == "labels", feature == "ports", feature == "sections")
    for cost in ["CENTER_DISTANCE", "CIRCLE_UNDERLAP", "RECTANGLE_UNDERLAP", "INVERTED_OVERLAP", "MINIMUM_ROOT_DISTANCE"]:
        for tree in ["MINIMUM_SPANNING_TREE", "MAXIMUM_SPANNING_TREE"]:
            for ortho in [False, True]:
                add(f"cost-{cost}-{tree}-{ortho}", 8,
                    {"elk.algorithm": "sporeCompaction", "elk.processingOrder.spanningTreeCostFunction": cost,
                     "elk.processingOrder.treeConstruction": tree, "elk.compaction.orthogonal": ortho,
                     "elk.processingOrder.rootSelection": "FIXED", "elk.processingOrder.preferredRoot": "n3"})
    return rows


def expanded():
    rng = random.Random(927003)
    rows = []
    for k in range(100):
        n = 2 + k % 8
        mode = ["sporeOverlap", "sporeCompaction"][k % 2]
        nodes = []
        for i in range(n):
            c = dict(id=f"n{i}", x=rng.uniform(-120, 120) * (1 if k % 3 else 4),
                     y=rng.uniform(-100, 100) * (1 if k % 3 else 4), width=rng.uniform(5, 80), height=rng.uniform(5, 60))
            if k % 5 == 0:
                c["labels"] = [dict(text="lbl", x=-15.7, y=-13.3, width=77.9, height=11.7)]
            if k % 7 == 0:
                c["ports"] = [dict(id=f"p{i}", x=3.7, y=-5.1, width=7.3, height=5.1)]
            nodes.append(c)
        edges = [dict(id=f"e{i}", sources=[f"p{i}" if k % 7 == 0 else f"n{i}"], targets=[f"n{i + 1}"])
                 for i in range(n - 1)]
        if k % 4 == 0:
            edges[0]["sections"] = [dict(id="existing", startPoint=dict(x=-1.7, y=3.3), endPoint=dict(x=8.9, y=11.3),
                                         bendPoints=[dict(x=13.9, y=-5.7)])]
        opts = {"elk.algorithm": mode, "elk.overlapRemoval.runScanline": k % 4 < 2,
                "elk.overlapRemoval.maxIterations": k % 8 + 1, "elk.compaction.orthogonal": k % 3 == 0,
                "elk.spacing.nodeNode": rng.uniform(0, 25),
                "elk.padding": f"[top={rng.uniform(0, 10)},left={rng.uniform(0, 10)},bottom={rng.uniform(0, 10)},"
                               f"right={rng.uniform(0, 10)}]",
                "elk.processingOrder.spanningTreeCostFunction": ["CENTER_DISTANCE", "CIRCLE_UNDERLAP",
                                                                 "RECTANGLE_UNDERLAP", "INVERTED_OVERLAP",
                                                                 "MINIMUM_ROOT_DISTANCE"][k % 5],
                "elk.processingOrder.treeConstruction": ["MINIMUM_SPANNING_TREE", "MAXIMUM_SPANNING_TREE"][k % 3 == 0],
                "elk.processingOrder.rootSelection": "FIXED", "elk.processingOrder.preferredRoot": f"n{k % n}"}
        rows.append(dict(name=f"fractional-{k:03}", input=dict(id="root", children=nodes, edges=edges,
                                                                layoutOptions=opts)))
    controls = base()
    find = lambda name: copy.deepcopy(next(x for x in controls if x["name"] == name))
    for alg in ["sporeOverlap", "sporeCompaction"]:
        for under in ["fixed", "box", "mrtree", "radial", "rectpacking", "layered", "force", "stress", "random"]:
            for minimum in [False, True]:
                row = find(alg + "-5-scatter")
                row["name"] = f"delegate-{alg}-{under}-min-{minimum}"
                opts = row["input"]["layoutOptions"]
                opts.update({"elk.underlyingLayoutAlgorithm": under, "elk.randomSeed": 17, "elk.force.iterations": 3,
                             "elk.stress.iterationLimit": 2})
                if minimum:
                    opts.update({"elk.nodeSize.constraints": "[MINIMUM_SIZE]", "elk.nodeSize.minimum": "(800.3,600.7)"})
                rows.append(row)
        for under in [None, "fixed", "box", "mrtree"]:
            row = find(alg + "-5-overlap")
            row["name"] = f"nested-{alg}-{under}"
            node = row["input"]["children"][0]
            node.update(children=[dict(id="aa", width=21.3, height=13.7), dict(id="bb", width=37.9, height=19.3)],
                        edges=[dict(id="inside", sources=["aa"], targets=["bb"])],
                        layoutOptions={"elk.algorithm": "mrtree"})
            if under:
                row["input"]["layoutOptions"]["elk.underlyingLayoutAlgorithm"] = under
            rows.append(row)
    # Error delegation: Radial has no incoming-free root in this directed cycle.
    for alg in ["sporeOverlap", "sporeCompaction"]:
        row = find(alg + "-3-scatter")
        row["name"] = "delegated-error-" + alg
        row["input"]["layoutOptions"]["elk.underlyingLayoutAlgorithm"] = "radial"
        row["input"]["edges"].append(dict(id="back", sources=["n2"], targets=["n0"]))
        rows.append(row)
    return rows


def root_transfer():
    """Root size transfer of a two-node Layered graph: minimum size, fixed graph size, padding, direction.

    Recovered from the 2026-10-19 SPoRE root-transfer controls (spore/tests.rs).
    """
    profiles = [
        {"elk.nodeSize.constraints": "[MINIMUM_SIZE]", "elk.nodeSize.minimum": "(800.3,600.7)"},
        {"elk.nodeSize.fixedGraphSize": True},
        {"elk.nodeSize.fixedGraphSize": True, "elk.nodeSize.constraints": "[MINIMUM_SIZE]",
         "elk.nodeSize.minimum": "(800.3,600.7)"},
        {"elk.nodeSize.constraints": "[MINIMUM_SIZE]", "elk.nodeSize.minimum": "(-1,-2)",
         "elk.padding": "[top=0,left=0,bottom=0,right=0]"},
        {"elk.nodeSize.constraints": "[MINIMUM_SIZE]", "elk.nodeSize.minimum": "(800.3,600.7)", "elk.direction": "DOWN"},
        {"elk.nodeSize.fixedGraphSize": True, "elk.direction": "LEFT"},
    ]
    return [dict(name=f"root-transfer-{i}", input=dict(
        id="root", children=[dict(id="a", width=23.1, height=17.3), dict(id="b", width=26.8, height=20.2)],
        edges=[dict(id="ab", sources=["a"], targets=["b"])],
        layoutOptions={"elk.algorithm": "layered", **opts}, width=250.1, height=180.3))
        for i, opts in enumerate(profiles)]
