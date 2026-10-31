"""Inputs for the D2 ELK adapter controls (crates/d2-rs-elk/src/{bends,margins,shape_trace}.rs).

Synthetic d2graph geometry for d2layouts/d2elklayout deleteBends (plus its
bend collision index), the adapter's margin-shrinking block and
d2graph.Edge.TraceToShape. Recovered from the 2026-10-16 adapter probes.
"""
import copy
import random


def delete_bends():
    pt = lambda x, y: dict(X=x, Y=y)
    node = lambda x, y, w=100, h=100, **kw: dict(X=x, Y=y, W=w, H=h, Shape="rectangle", **kw)
    edge = lambda route, source=0, target=1, **kw: dict(Source=source, Target=target, Route=[pt(*x) for x in route], **kw)
    base = dict(Nodes=[node(0, 0), node(300, 0)], Edges=[edge([(100, 20), (160, 20), (160, 60), (300, 60)])])
    cases = []

    def add(name, c):
        c = copy.deepcopy(c)
        c["Name"] = name
        cases.append(c)

    add("source-S", base)
    c = copy.deepcopy(base)
    c["Edges"][0]["Route"] = list(reversed(c["Edges"][0]["Route"]))
    c["Edges"][0].update(Source=1, Target=0)
    add("target-S", c)
    for x, y in [(160, 60), (230, 60), (160, -30), (160, 101), (160, 10), (160, 90)]:
        c = copy.deepcopy(base)
        c["Edges"][0]["Route"][2] = pt(x, y)
        c["Edges"][0]["Route"][3]["Y"] = y
        add(f"attachment-{x}-{y}", c)
    for shape in ["oval", "diamond", "hexagon", "cloud", "person", "parallelogram", "cylinder", "step", "package",
                  "document", "queue", "page", "stored_data", "callout"]:
        c = copy.deepcopy(base)
        c["Nodes"][0]["Shape"] = shape
        add("source-shape-" + shape, c)
    for shape in ["rectangle", "hexagon"]:
        for flag in ["ThreeD", "Multiple"]:
            for y in [0, 3, 4, 8, 9, 10, 11, 89, 90]:
                c = copy.deepcopy(base)
                c["Nodes"][0]["Shape"] = shape
                c["Nodes"][0][flag] = True
                c["Edges"][0]["Route"][2]["Y"] = y
                c["Edges"][0]["Route"][3]["Y"] = y
                add(f"{shape}-{flag}-{y}", c)
    for y in [40, 45, 49, 50, 51, 55, 60, 70, 80, 90]:
        c = copy.deepcopy(base)
        c["Nodes"][0].update(Shape="sql_table", Columns=2)
        c["Edges"][0]["SourceColumn"] = 0
        c["Edges"][0]["Route"][2]["Y"] = y
        c["Edges"][0]["Route"][3]["Y"] = y
        add("column-" + str(y), c)
    for x, y in [(150, 40), (150, 110), (250, 60), (-200, -200), (120, 10)]:
        c = copy.deepcopy(base)
        c["Nodes"].append(node(x, y, 5, 5))
        add(f"object-{x}-{y}", c)
    for x in [100, 101, 109, 110, 119, 120, 121, 180, 300]:
        for y in [20, 40, 60, 61]:
            c = copy.deepcopy(base)
            c["Nodes"] += [node(500, 500), node(700, 500)]
            c["Edges"].append(edge([(x, y), (x, 200)], 2, 3))
            add(f"edge-{x}-{y}", c)
    ladder = dict(Nodes=[node(-100, -100, 10, 10), node(500, 500, 10, 10)],
                  Edges=[edge([(0, 0), (50, 0), (50, 50), (100, 50), (100, 100), (150, 100)])])
    add("ladder", ladder)
    for trans in ["mirror-x", "mirror-y", "swap", "signed-zero"]:
        c = copy.deepcopy(ladder)
        for e in c["Edges"]:
            for q in e["Route"]:
                if trans == "mirror-x":
                    q["X"] = -q["X"]
                elif trans == "mirror-y":
                    q["Y"] = -q["Y"]
                elif trans == "swap":
                    q["X"], q["Y"] = q["Y"], q["X"]
                elif q["X"] == 0:
                    q["X"] = -0.0
        add(trans, c)
    c = copy.deepcopy(ladder)
    c["Edges"].append(edge([(50, 50), (40, 50), (40, 200)], 1, 0))
    add("shared-corner", c)
    c = copy.deepcopy(ladder)
    c["Edges"][0]["Target"] = 0
    add("self-loop-unchanged", c)
    rng = random.Random(1030)
    for i in range(160):
        c = copy.deepcopy(base if i % 2 else ladder)
        for j in range(rng.randrange(1, 5)):
            c["Nodes"].append(node(rng.randrange(-50, 450), rng.randrange(-50, 150), rng.randrange(1, 100),
                                   rng.randrange(1, 100)))
        for j in range(rng.randrange(1, 6)):
            x, y = rng.randrange(-50, 450), rng.randrange(-50, 200)
            c["Edges"].append(edge([(x, y), (x + rng.randrange(1, 100), y),
                                    (x + rng.randrange(1, 100), y + rng.randrange(1, 100))], 2, len(c["Nodes"]) - 1))
        c["Queries"] = [dict(Edge=0, Segment=[pt(rng.randrange(-100, 500), rng.randrange(-100, 250)),
                                              pt(rng.randrange(-100, 500), rng.randrange(-100, 250))]) for j in range(8)]
        add("collision-seed-1030-" + str(i), c)
    return cases


def margin_shrink():
    node = lambda x, y, w=100, h=100, parent=None, margin=None: dict(
        X=x, Y=y, W=w, H=h, Parent=parent, Margin=margin or dict(Top=0, Right=0, Bottom=0, Left=0))
    point = lambda x, y: dict(x=x, y=y)
    edge = lambda s, t, route: dict(Source=s, Target=t, Route=[point(*x) for x in route])
    rows = []

    def add(name, nodes, edges, order=None):
        rows.append(dict(Name=name, Nodes=nodes, Edges=edges, Order=order or list(range(len(nodes)))))

    add("codec-all-four-bands", [node(20, 30, 114, 88, margin=dict(Top=6, Right=4, Bottom=2, Left=10)),
                                 node(25, 37, 20, 12, parent=0)], [])
    for side in ["Left", "Right", "Top", "Bottom"]:
        for amount in [-4, 0, 2, 6, 10, 20, 120]:
            ns = [node(0, 0, margin={k: amount if k == side else 0 for k in ["Top", "Right", "Bottom", "Left"]}),
                  node(20, 20, 40, 40, parent=0), node(30, 30, 10, 10, parent=1), node(400, 200)]
            es = [edge(0, 3, [(0, 0), (120, 0), (120, 250), (400, 250)]),
                  edge(3, 0, [(400, 250), (120, 250), (120, 100), (100, 100)]),
                  edge(1, 3, [(20, 40), (21, 40), (22, 40), (23, 40), (400, 40), (400, 200)]),
                  edge(3, 2, [(400, 200), (34, 200), (34, 32), (34, 31), (34, 30)]),
                  edge(0, 2, [(50, 100), (50, 110), (35, 110), (35, 30)]), edge(2, 1, [(30, 35), (35, 35), (35, 20)]),
                  edge(0, 0, [(0, 50), (-20, 50), (-20, -20), (50, -20), (50, 0)])]
            add(f"band-{side}-{amount}", ns, es)
    rng = random.Random(1030)
    for i in range(220):
        ns = []
        for j in range(rng.randrange(3, 10)):
            parent = None if j == 0 or rng.randrange(4) == 0 else rng.randrange(j)
            ns.append(node(rng.randrange(-100, 800) / 4, rng.randrange(-100, 800) / 4, rng.randrange(1, 500) / 4,
                           rng.randrange(1, 500) / 4, parent=parent,
                           margin={k: rng.choice([0, 0, 1, 2, 5, 10, 15, 16, 26, 32, 64]) / 2
                                   for k in ["Top", "Right", "Bottom", "Left"]}))
        es = []
        for j in range(rng.randrange(3, 18)):
            s, t = rng.randrange(len(ns)), rng.randrange(len(ns))
            a, b = ns[s], ns[t]
            sx, sy = a["X"], a["Y"]
            tx, ty = b["X"], b["Y"]
            if rng.randrange(2):
                sx += a["W"]
            if rng.randrange(2):
                sy += a["H"]
            if rng.randrange(2):
                tx += b["W"]
            if rng.randrange(2):
                ty += b["H"]
            route = [(sx, sy), (sx + 1, sy), (sx + 2, sy), (sx + 4, sy), (tx, sy), (tx, ty)]
            if i % 3 == 0:
                route = [(sx, sy), (sx, sy + 1), (sx, sy + 2), (sx, sy + 4), (sx, ty), (tx, ty)]
            es.append(edge(s, t, route))
        order = list(range(len(ns)))
        rng.shuffle(order)
        add("nested-seed1030-" + str(i), ns, es, order)
    return rows


def trace_to_shape():
    node = lambda x, y, w=100, h=100, **kw: dict(X=x, Y=y, W=w, H=h, Shape="rectangle", Label="",
                                                 Position="INSIDE_MIDDLE_CENTER", IconPosition="INSIDE_MIDDLE_CENTER",
                                                 Icon=False, LabelW=30, LabelH=12) | kw
    point = lambda x, y: dict(x=x, y=y)
    rows = []

    def add(name, nodes, route):
        rows.append(dict(Name=name, Nodes=nodes, Route=[point(*p) for p in route]))

    for shape in ["rectangle", "oval", "diamond", "hexagon", "cloud", "person", "parallelogram", "cylinder", "step",
                  "package", "document", "queue", "page", "stored_data", "callout", "text", "code", "class", "sql_table",
                  "image"]:
        for x in [10.125, 33.333, 50, 90.875]:
            add(shape + "-" + str(x), [node(0, 0, Shape=shape), node(200, 200, Shape=shape)],
                [(x, 100), (x, 140), (200 + x, 140), (200 + x, 200)])
    positions = ["OUTSIDE_" + s for s in ["TOP_LEFT", "TOP_CENTER", "TOP_RIGHT", "LEFT_TOP", "LEFT_MIDDLE", "LEFT_BOTTOM",
                                          "RIGHT_TOP", "RIGHT_MIDDLE", "RIGHT_BOTTOM", "BOTTOM_LEFT", "BOTTOM_CENTER",
                                          "BOTTOM_RIGHT"]]
    # Rich crossing paths, and paths inside the target decoration across multiple segments.
    for pos in positions:
        for side in range(2):
            for kind in ["label", "icon", "both"]:
                ns = [node(0, 0), node(300, 0)]
                ns[side].update(Label="node" if kind != "icon" else "", Position=pos, Icon=kind != "label",
                                IconPosition=pos)
                for j, path in enumerate([[(-100, 50), (500, 50)],
                                          [(50, -150), (50, 200), (350, 200), (350, -150)],
                                          [(50, 250), (50, 101), (50, 100), (350, 100), (350, 101), (350, 250)],
                                          [(50, -80), (50, -2), (50, -12), (350, -12), (350, -2), (350, -80)],
                                          [(50, 50), (150, 50), (150, -10), (300, -10), (320, -10), (320, 50)]]):
                    add(f"{pos}-{side}-{kind}-{j}", ns, path)
    for neighbor in [100, 101, 109, 109.99999999999999, 110, 110.00000000000001, 111]:
        for side in range(2):
            path = [(95, 50), (neighbor, 50), (200, 50), (305, 50)]
            if side:
                path = list(reversed([(400 - x, y) for x, y in path]))
            add(f"short-{side}-{neighbor}", [node(0, 0), node(300, 0)], path)
    for size in [7, 12, 31, 32, 63, 100, 200]:
        for side in range(2):
            ns = [node(0, 0, size, size), node(300, 0, size, size)]
            ns[side].update(Icon=True, IconPosition="OUTSIDE_TOP_CENTER")
            add(f"icon-size-{side}-{size}", ns, [(size / 2, -100), (size / 2, 0), (300 + size / 2, 0), (300 + size / 2, -100)])
    rng = random.Random(1030)
    for i in range(150):
        ns = [node(.25, .75, 97.5, 83.25), node(300.125, -.875, 83.75, 97.25)]
        for n in ns:
            n.update(Label="label", Position=rng.choice(positions), Icon=rng.choice([False, True]),
                     IconPosition=rng.choice(positions))
        route = [(rng.randrange(-100, 500) / 2, rng.randrange(-200, 300) / 2) for _ in range(rng.randrange(2, 9))]
        add("fractional-" + str(i), ns, route)
    return rows
