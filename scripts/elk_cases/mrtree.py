"""elk-go Mr.Tree provider controls (crates/d2-rs-elk/src/mrtree/tests.rs).

`base()` -> original_mrtree.json cases; `expanded()` -> original_mrtree_expanded.json.
Recovered from the 2026-10-19 Mr.Tree qualification generators.
"""
import json
import random

SHAPES = {
    "chain": (5, [(0, 1), (1, 2), (2, 3), (3, 4)]),
    "star": (6, [(0, 1), (0, 2), (0, 3), (0, 4), (0, 5)]),
    "mixed": (9, [(0, 1), (0, 2), (0, 3), (1, 4), (1, 5), (3, 6), (6, 7), (6, 8)]),
    "cycle": (4, [(0, 1), (1, 2), (2, 3), (3, 0)]),
    "dag": (5, [(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)]),
    "forest": (7, [(0, 1), (0, 2), (3, 4), (4, 5)]),
    "parallel": (3, [(0, 1), (0, 1), (1, 2)]),
}


def _graph(n, edges, options, fractional=False, constraints=False):
    children = []
    for i in range(n):
        node = {"id": f"n{i}", "width": 13.1 + i * 2.31, "height": 7.3 + i * 1.17} if fractional \
            else {"id": f"n{i}", "width": 20 + i * 7, "height": 17 + i * 3}
        if constraints:
            node["layoutOptions"] = {"elk.mrtree.positionConstraint": [-1, 2, 0, 3, 1][i % 5]}
        children.append(node)
    return {"id": "root", "children": children,
            "edges": [{"id": f"e{i}", "sources": [f"n{a}"], "targets": [f"n{b}"]} for i, (a, b) in enumerate(edges)],
            "layoutOptions": {"elk.algorithm": "mrtree", **options}}


def base():
    rows = [dict(name="empty", input=_graph(0, [], {})),
            dict(name="single", input=_graph(1, [], {})),
            dict(name="self-loop", input=_graph(1, [(0, 0)], {}))]
    for shape, (n, edges) in SHAPES.items():
        for direction in ["DOWN", "UP", "RIGHT", "LEFT"]:
            rows.append(dict(name=f"{shape}-{direction}", input=_graph(n, edges, {"elk.direction": direction})))
    n, edges = SHAPES["mixed"]
    for weighting in ["MODEL_ORDER", "DESCENDANTS", "FAN", "CONSTRAINT"]:
        for routing in ["AVOID_OVERLAP", "MIDDLE_TO_MIDDLE", "NONE"]:
            for compaction in [False, True]:
                rows.append(dict(name=f"options-{weighting}-{routing}-{compaction}", input=_graph(n, edges, {
                    "elk.mrtree.weighting": weighting, "elk.mrtree.edgeRoutingMode": routing,
                    "elk.mrtree.compaction": compaction, "elk.mrtree.searchOrder": "BFS",
                    "elk.spacing.nodeNode": 13.7, "elk.spacing.edgeNode": 2.3,
                    "elk.mrtree.edgeEndTextureLength": 4.9}, fractional=True, constraints=True)))
    n, edges = SHAPES["cycle"]
    for direction in ["LEFT", "RIGHT", "UP", "DOWN"]:
        for compaction in [False, True]:
            rows.append(dict(name=f"cycle-{direction}-compact-{compaction}", input=_graph(n, edges, {
                "elk.direction": direction, "elk.mrtree.compaction": compaction, "elk.mrtree.searchOrder": "BFS"},
                fractional=True)))
    n, edges = SHAPES["forest"]
    for options in [{"elk.separateConnectedComponents": False}, {"elk.padding": "[top=3,left=7,bottom=11,right=13]"},
                    {"elk.nodeSize.fixedGraphSize": True}, {"elk.aspectRatio": 0.6}]:
        rows.append(dict(name=f"forest-{len(rows)}", input=_graph(n, edges, options, fractional=True)))
    return rows


def expanded():
    randomizer = random.Random(90127)
    rows = []
    for k in range(120):
        n = 3 + k % 10
        direction = ["DOWN", "RIGHT", "UP", "LEFT"][k % 4]
        opts = {"elk.algorithm": "mrtree", "elk.direction": direction, "elk.mrtree.searchOrder": ["DFS", "BFS"][k % 2],
                "elk.mrtree.weighting": ["MODEL_ORDER", "FAN", "DESCENDANTS", "CONSTRAINT"][(k // 4) % 4],
                "elk.mrtree.compaction": k % 3 == 0, "elk.spacing.nodeNode": randomizer.uniform(5, 40),
                "elk.spacing.edgeNode": randomizer.uniform(.5, 8),
                "elk.mrtree.edgeEndTextureLength": randomizer.uniform(.1, 13),
                "elk.aspectRatio": randomizer.uniform(.4, 2.5), "elk.padding": "[top=3.7,left=6.1,bottom=9.3,right=11.7]",
                "elk.separateConnectedComponents": k % 5 != 0}
        children = []
        for i in range(n):
            c = {"id": f"n{i}", "width": randomizer.uniform(1, 120), "height": randomizer.uniform(1, 100),
                 "x": randomizer.uniform(-90, 90), "y": randomizer.uniform(-70, 70),
                 "layoutOptions": {"elk.mrtree.positionConstraint": randomizer.randrange(-2, 15),
                                   "elk.priority": randomizer.randrange(-1, 5)}}
            if k % 6 == 0:
                c["labels"] = [{"id": f"l{i}", "text": "abc", "width": randomizer.uniform(5, 50),
                                "height": randomizer.uniform(3, 20)}]
                c["layoutOptions"]["elk.nodeLabels.placement"] = "[OUTSIDE, H_LEFT, V_TOP]"
            if k % 8 == 0:
                c["ports"] = [{"id": f"p{i}", "width": 2.3, "height": 4.7, "layoutOptions": {"elk.port.side": "EAST"}}]
            children.append(c)
        edges = []
        for i in range(1, n):
            if k % 9 == 0 and i % 3 == 0:
                continue
            parent = randomizer.randrange(i)
            edges.append({"id": f"e{i}", "sources": [f"p{parent}" if k % 8 == 0 else f"n{parent}"], "targets": [f"n{i}"]})
        if k % 5 == 0:
            edges.append({"id": "cycle", "sources": [f"n{n - 1}"], "targets": ["n0"]})
        if k % 11 == 0:
            edges.append({"id": "long", "sources": ["n0"], "targets": [f"n{n - 1}"]})
        rows.append({"name": f"fractional-{k:03}", "input": {"id": "root", "children": children, "edges": edges,
                                                             "layoutOptions": opts}})
    # Recursive registered dispatch, existing sections, clamped internal sizes, fixed graph dimensions.
    for k in range(12):
        row = json.loads(json.dumps(rows[k]))
        row["name"] = f"wire-{k:02}"
        if k < 4:
            c = row["input"]["children"][0]
            c["children"] = [{"id": "aa", "width": 31.7, "height": 23.1}, {"id": "bb", "width": 21.3, "height": 17.9}]
            c["edges"] = [{"id": "inside", "sources": ["aa"], "targets": ["bb"]}]
            c["layoutOptions"]["elk.algorithm"] = ["mrtree", "radial", "box", "fixed"][k]
        elif k < 8:
            row["input"]["layoutOptions"]["elk.omitNodeMicroLayout"] = True
            row["input"]["children"][0]["width"] = 0.
            row["input"]["children"][1]["height"] = -3.3
        else:
            row["input"]["width"] = 700.7
            row["input"]["height"] = 400.3
            row["input"]["layoutOptions"]["elk.nodeSize.fixedGraphSize"] = True
        rows.append(row)
    return rows
