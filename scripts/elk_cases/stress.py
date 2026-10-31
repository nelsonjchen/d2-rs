"""elk-go Force/Stress controls (crates/d2-rs-elk/src/force_layout/tests.rs).

`base()` -> original_force_stress.json cases (2026-10-19 Force/Stress generator);
`followup()` -> original_force_stress_expanded.json cases (follow-up matrix,
reconstructed: its inline generator was not retained, and this one reproduces
the 132 recorded inputs exactly);
`log_bits()` -> original_force_logarithm.json inputs (Go math.Log controls).
"""
import copy
import random


def _add(rows, name, g, options=None):
    for algorithm in ["force", "stress"]:
        graph = copy.deepcopy(g)
        graph.setdefault("id", "g")
        o = graph.setdefault("layoutOptions", {})
        o["org.eclipse.elk.algorithm"] = algorithm
        o.update({"org.eclipse.elk." + k: v for k, v in (options or {}).items()})
        rows.append(dict(name=algorithm + "-" + name, input=graph))


def base():
    rows = []
    add = lambda name, g, options=None: _add(rows, name, g, options)

    def graph(n, edges):
        return dict(children=[dict(id=f"n{i}", width=20 + i * 7, height=15 + i * 4, x=i * 43, y=i * i * 7)
                              for i in range(n)],
                    edges=[dict(id=f"e{i}", sources=[f"n{a}"], targets=[f"n{b}"]) for i, (a, b) in enumerate(edges)])

    for n in [1, 2, 3, 5]:
        for kind in ["chain", "star", "cycle", "disconnected"]:
            edges = ([(i, i + 1) for i in range(n - 1)] if kind == "chain" else
                     [(0, i) for i in range(1, n)] if kind == "star" else
                     [(i, (i + 1) % n) for i in range(n)] if kind == "cycle" else [])
            add(f"{kind}-{n}", graph(n, edges))
    g = graph(4, [(0, 1), (1, 2), (2, 0)])
    g["edges"][0]["labels"] = [dict(id="l0", text="label", width=30, height=10)]
    for name, o in [("eades", {"force.model": "EADES", "force.iterations": 7}),
                    ("short", {"force.iterations": 5, "stress.iterationLimit": 2}),
                    ("interactive", {"interactive": True, "force.iterations": 3}),
                    ("dimension-x", {"stress.dimension": "X"}), ("dimension-y", {"stress.dimension": "Y"}),
                    ("negative-seed", {"randomSeed": -81}),
                    ("asymmetric", {"padding": "[top=7,left=11,bottom=13,right=17]"}),
                    ("one-component", {"separateConnectedComponents": False}),
                    ("minimum", {"nodeSize.constraints": "MINIMUM_SIZE", "nodeSize.minimum": "(500,400)"}),
                    ("no-shift", {"nodeSize.fixedGraphSize": True}), ("bend-particles", {"force.iterations": 4})]:
        h = copy.deepcopy(g)
        if name == "bend-particles":
            h["edges"][0]["layoutOptions"] = {"org.eclipse.elk.force.repulsivePower": 2}
        if name == "no-shift":
            h.update(width=987, height=654)
        add(name, h, o)
    h = copy.deepcopy(g)
    h["children"][0]["layoutOptions"] = {"org.eclipse.elk.stress.fixed": True}
    add("fixed", h)
    h = graph(3, [(0, 1), (0, 1), (1, 0), (1, 2)])
    h["edges"][1]["labels"] = [dict(text="in", width=13, height=9, layoutOptions={"org.eclipse.elk.edgeLabels.inline": True})]
    add("parallel", h, {"force.iterations": 10})
    h = graph(2, [(0, 1)])
    h["children"][0]["ports"] = [dict(id="p", x=0, y=0, width=5, height=7, layoutOptions={"org.eclipse.elk.port.side": "EAST"})]
    h["edges"][0]["sources"] = ["p"]
    add("port", h)
    h = graph(2, [(0, 1)])
    h["children"][0]["children"] = [dict(id="inside", width=15, height=10)]
    add("nested", h)
    for k in range(16):
        rng = random.Random(512 + k)
        n = rng.randrange(2, 6)
        h = graph(n, [(i, i + 1) for i in range(n - 1)])
        for node in h["children"]:
            node["width"] = rng.randrange(10, 60) + 0.125
            node["height"] = rng.randrange(10, 40) + 0.375
        o = {"randomSeed": rng.randint(1, 100), "force.iterations": rng.randint(2, 12),
             "stress.iterationLimit": rng.randrange(0, 12), "stress.epsilon": 0.0001, "force.repulsion": 3.5,
             "spacing.nodeNode": 55.125}
        if k % 3 == 0:
            o["force.model"] = "EADES"
        if k % 4 == 0:
            o["interactive"] = True
        add(f"varied-{k}", h, o)
    return rows


def followup():
    rows = []
    add = lambda name, g, options=None: _add(rows, name, g, options)

    def graph(n, edges):
        return dict(children=[dict(id=f"n{i}", width=13 + i * 7, height=9 + i * 3, x=i * 41, y=i * 17) for i in range(n)],
                    edges=[dict(id=f"e{i}", sources=[f"n{a}"], targets=[f"n{b}"]) for i, (a, b) in enumerate(edges)])

    # A child whose single node is narrower than, equal to or wider than its parent.
    for width in [0, 1, 5, 15, 19.5, 20, 30]:
        for direction in ["RIGHT", "DOWN"]:
            h = graph(2, [(0, 1)])
            h["children"][0]["children"] = [dict(id="inside", width=width, height=7.25)]
            h["children"][0]["layoutOptions"] = {"org.eclipse.elk.direction": direction}
            add(f"tiny-nested-{width}-{direction}", h)
    # Receiver-local child algorithm selection (Random explicitly seeded: deterministic geometry).
    for algorithm in ["layered", "fixed", "box", "rectpacking", "random", "force", "stress"]:
        h = graph(2, [(0, 1)])
        h["children"][0]["children"] = [dict(id="inner0", width=15, height=10, x=2, y=4),
                                        dict(id="inner1", width=22, height=17, x=19, y=28)]
        h["children"][0]["layoutOptions"] = {"org.eclipse.elk.algorithm": algorithm}
        if algorithm == "random":
            h["children"][0]["layoutOptions"]["org.eclipse.elk.randomSeed"] = 73
        add(f"child-{algorithm}", h)
    for seed in ["-2147483648", "2147483647", "-1", 1, 73]:
        add(f"seed-{seed}", graph(4, [(0, 1), (1, 2), (2, 0)]),
            {"randomSeed": seed, "force.iterations": 11, "stress.iterationLimit": 7})
    for k in range(40):
        rng = random.Random(7264 + k)
        n = rng.randrange(2, 7)
        edges = [(i, i + 1) for i in range(n - 1)]
        if k % 2:
            edges.append((n - 1, 0))
        if k % 5 == 0:
            edges += [(0, 1), (1, 0)]
        h = graph(n, edges)
        for node in h["children"]:
            node["width"] = rng.randrange(60) + .125
            node["height"] = rng.randrange(40) + .375
            node["layoutOptions"] = {"org.eclipse.elk.priority": rng.randrange(-4, 8)}
        for edge in h["edges"]:
            edge["layoutOptions"] = {"org.eclipse.elk.force.repulsivePower": k % 4,
                                     "org.eclipse.elk.stress.desiredEdgeLength": rng.randrange(30, 110),
                                     "org.eclipse.elk.priority": rng.randrange(-3, 9)}
        if k % 3 == 0:
            h["edges"][0]["labels"] = [dict(id="l0", text="first", width=23, height=9),
                                       dict(id="l1", text="second", width=11, height=7,
                                            layoutOptions={"org.eclipse.elk.edgeLabels.inline": True})]
        add(f"weighted-{k}", h, {"randomSeed": k + 1, "force.iterations": 2 + k % 19,
                                 "force.model": ["FRUCHTERMAN_REINGOLD", "EADES"][k % 2],
                                 "force.temperature": (k + 1) * .0001, "force.repulsion": 1.5 + k % 7,
                                 "stress.iterationLimit": 1 + k % 13, "stress.dimension": ["XY", "X", "Y"][k % 3],
                                 "interactive": k % 4 == 0, "separateConnectedComponents": k % 7 != 0})
    return rows


LOG_SPECIALS = [0x0, 0x1, 0x8000000000000000, 0x10000000000000, 0x3ff0000000000000, 0x3fefffffffffffff,
                0x3ff0000000000001, 0x7fefffffffffffff, 0x7ff0000000000000, 0x7ff8000000000001, 0xbff0000000000000]


def log_bits():
    """1,011 float64 bit patterns for Go math.Log (force_layout/logarithm.rs).

    The 2026-10-19 capture's eleven specials (zeros, subnormal and normal
    minimum, 1 and its neighbours, maximum, +Inf, NaN, -1) followed by 1,000
    positive finite values with uniform exponent fields. That capture's
    random tail came from an unretained inline generator, so this tail is
    drawn afresh from the same distribution with a fixed seed.
    """
    rng = random.Random(1019)
    bits = LOG_SPECIALS + [(rng.randrange(2047) << 52) | rng.getrandbits(52) for _ in range(1000)]
    assert len(bits) == 1011
    return bits
