"""elk-go Radial provider controls (crates/d2-rs-elk/src/radial_layout/tests.rs) and
the scalar inputs of the Go math / elk-go javacompat oracles
(crates/d2-rs-elk/src/{go_math,js_trig}.rs).

Recovered from the 2026-10-19 Radial qualification generators.
"""
import copy
import math
import random
import struct


def _bits(value: float) -> int:
    return struct.unpack(">Q", struct.pack(">d", value))[0]


def _graph(n, edges):
    return dict(id="g", children=[dict(id=f"n{i}", width=23 + i * 7, height=17 + i * 3, x=13 + i * 31, y=7 + i * 19)
                                  for i in range(n)],
                edges=[dict(id=f"e{i}", sources=[f"n{a}"], targets=[f"n{b}"]) for i, (a, b) in enumerate(edges)])


def base():
    rows = []

    def add(name, g, options={}):
        h = copy.deepcopy(g)
        h.setdefault("layoutOptions", {}).update(
            {"org.eclipse.elk.algorithm": "radial", **{"org.eclipse.elk." + k: v for k, v in options.items()}})
        rows.append(dict(name=name, input=h))

    for n in [1, 2, 3, 5, 7]:
        for kind in ["chain", "star", "binary", "disconnected"]:
            edges = ([(i, i + 1) for i in range(n - 1)] if kind == "chain" else
                     [(0, i) for i in range(1, n)] if kind == "star" else
                     [((i - 1) // 2, i) for i in range(1, n)] if kind == "binary" else [])
            add(f"{kind}-{n}", _graph(n, edges))
    g = _graph(5, [(0, 1), (0, 2), (1, 3), (1, 4)])
    profiles = [("radius", {"radial.radius": 73.125}), ("leaf", {"radial.wedgeCriteria": "LEAF_NUMBER"}),
                ("id", {"radial.sorter": "ID"}), ("polar", {"radial.sorter": "POLAR_COORDINATE"}),
                ("radial-compact", {"radial.radius": 120, "radial.compactor": "RADIAL_COMPACTION",
                                    "radial.compactionStepSize": 3}),
                ("wedge-compact", {"radial.radius": 120, "radial.compactor": "WEDGE_COMPACTION",
                                   "radial.compactionStepSize": 3}),
                ("edge-opt", {"radial.optimizationCriteria": "EDGE_LENGTH"}),
                ("position-opt", {"radial.optimizationCriteria": "EDGE_LENGTH_BY_POSITION"}),
                ("crossing-opt", {"radial.optimizationCriteria": "CROSSING_MINIMIZATION_BY_POSITION"}),
                ("center", {"radial.centerOnRoot": True}),
                ("rotate", {"radial.rotate": True, "radial.rotation.targetAngle": 1.25}),
                ("wedge-rotation", {"radial.rotate": True, "radial.rotation.targetAngle": -0.75,
                                    "radial.rotation.computeAdditionalWedgeSpace": True}),
                ("angles", {"radial.rotation.outgoingEdgeAngles": True}),
                ("padding", {"padding": "[top=7,left=11,bottom=13,right=17]"}),
                ("omit-micro", {"omitNodeMicroLayout": True}), ("fixed", {"nodeSize.fixedGraphSize": True}),
                ("minimum", {"nodeSize.constraints": "MINIMUM_SIZE", "nodeSize.minimum": "(500,400)"})]
    for i, node in enumerate(g["children"]):
        node["layoutOptions"] = {"org.eclipse.elk.position": f"({i * 17 - 15},{i * 11 - 7})",
                                 "org.eclipse.elk.radial.orderId": 7 - i}
    g.update(width=777, height=333)
    for name, o in profiles:
        add(name, g, o)
    for k in range(20):
        rng = random.Random(717 + k)
        n = rng.randrange(2, 8)
        h = _graph(n, [(rng.randrange(i), i) for i in range(1, n)])
        for node in h["children"]:
            node.update(width=rng.randrange(10, 80) + .125, height=rng.randrange(8, 40) + .375)
            node["layoutOptions"] = {"org.eclipse.elk.position": f"({rng.randrange(-99, 99)},{rng.randrange(-99, 99)})",
                                     "org.eclipse.elk.radial.orderId": rng.randrange(20)}
        o = {"radial.radius": 50 + k * 3, "radial.sorter": ["NONE", "ID", "POLAR_COORDINATE"][k % 3],
             "radial.wedgeCriteria": ["LEAF_NUMBER", "NODE_SIZE"][k % 2]}
        if k % 5 == 0:
            o.update({"radial.rotate": True, "radial.rotation.targetAngle": k * .125})
        if k % 7 == 0:
            o["radial.centerOnRoot"] = True
        add(f"varied-{k}", h, o)
    for which in ["node", "source-port", "target-port", "edge-label", "nested", "child-radial", "owned-edge",
                  "hyperedge", "negative-zero"]:
        h = _graph(3, [(0, 1), (0, 2)])
        if which == "node":
            h["children"][1]["labels"] = [dict(text="wide", x=-5, y=-9, width=77, height=31)]
        if which in ["source-port", "target-port"]:
            index = 0 if which == "source-port" else 1
            h["children"][index]["ports"] = [dict(id="p", x=-7, y=3, width=5, height=9,
                                                  layoutOptions={"org.eclipse.elk.port.side": "WEST"})]
            h["edges"][0]["sources" if index == 0 else "targets"] = ["p"]
        if which == "edge-label":
            h["edges"][0]["labels"] = [dict(text="head", width=66, height=14,
                                            layoutOptions={"org.eclipse.elk.edgeLabels.placement": "HEAD"})]
        if which in ["nested", "child-radial", "owned-edge"]:
            h["children"][0]["children"] = [dict(id="a", width=15, height=10), dict(id="b", width=22, height=17)]
            if which == "child-radial":
                h["children"][0]["layoutOptions"] = {"org.eclipse.elk.algorithm": "radial"}
                h["children"][0]["edges"] = [dict(id="inner", sources=["a"], targets=["b"])]
            if which == "owned-edge":
                h["children"][0]["edges"] = [dict(id="inner", sources=["a"], targets=["b"])]
        if which == "hyperedge":
            h["edges"] = [dict(id="h", sources=["n0"], targets=["n1", "n2"])]
        if which == "negative-zero":
            h.update(x=-0.0, y=-0.0)
        add(which, h)
    add("cycle-error", _graph(3, [(0, 1), (1, 2), (2, 0)]))
    add("self-error", _graph(1, [(0, 0)]))
    add("empty", _graph(0, []))
    return rows


def expanded():
    rows = []
    g = dict(id="g", children=[dict(id=f"n{i}", width=23.125 + 7 * i, height=11.375 + 3 * i, x=17 * i, y=29 * i,
                                    layoutOptions={"org.eclipse.elk.position": f"({i * 13 - 17},{i * 19 - 41})",
                                                   "org.eclipse.elk.radial.orderId": 9 - i}) for i in range(7)],
             edges=[dict(id=f"e{i}", sources=[f"n{a}"], targets=[f"n{b}"])
                    for i, (a, b) in enumerate([(0, 1), (0, 2), (0, 3), (1, 4), (2, 5), (3, 6)])])

    def add(name, o, graph=g):
        h = copy.deepcopy(graph)
        h["layoutOptions"] = {"org.eclipse.elk.algorithm": "radial", **{"org.eclipse.elk." + k: v for k, v in o.items()}}
        rows.append(dict(name=name, input=h))

    for optimizer in ["NONE", "EDGE_LENGTH", "EDGE_LENGTH_BY_POSITION", "CROSSING_MINIMIZATION_BY_POSITION"]:
        for sorter in ["NONE", "ID", "POLAR_COORDINATE"]:
            for wedge in ["LEAF_NUMBER", "NODE_SIZE"]:
                add(f"optimization-{optimizer}-{sorter}-{wedge}",
                    {"radial.optimizationCriteria": optimizer, "radial.sorter": sorter, "radial.wedgeCriteria": wedge,
                     "radial.radius": 77.25})
    for compactor in ["RADIAL_COMPACTION", "WEDGE_COMPACTION"]:
        for step in [1, 2, 5]:
            for sorter in ["NONE", "ID", "POLAR_COORDINATE"]:
                for wedge in ["LEAF_NUMBER", "NODE_SIZE"]:
                    add(f"compaction-{compactor}-{step}-{sorter}-{wedge}",
                        {"radial.compactor": compactor, "radial.compactionStepSize": step, "radial.sorter": sorter,
                         "radial.wedgeCriteria": wedge, "radial.radius": 100})
    for i, angle in enumerate([-math.pi, -math.pi / 2, -0.0, 0, .123, .75, math.pi / 2, 1e8, 1e16]):
        for extra in [False, True]:
            add(f"rotation-{i}-{extra}", {"radial.radius": 77.25, "radial.rotate": True,
                                          "radial.rotation.targetAngle": angle,
                                          "radial.rotation.computeAdditionalWedgeSpace": extra,
                                          "radial.centerOnRoot": i % 2 == 0})
    for omitted in [False, True]:
        h = copy.deepcopy(g)
        h["children"][0]["layoutOptions"]["org.eclipse.elk.margins"] = "[top=3,left=5,bottom=7,right=11]"
        add(f"margins-{omitted}", {"omitNodeMicroLayout": omitted, "padding": "[top=3,left=5,bottom=7,right=11]"}, h)
    h = copy.deepcopy(g)
    h["edges"].append(dict(id="shared", sources=["n2"], targets=["n4"]))
    add("shared-descendant", {}, h)
    return rows


def fractional():
    rows = []
    for k in range(100):
        rng = random.Random(97171 + k)
        n = rng.randrange(2, 13)
        nodes = []
        for i in range(n):
            node = dict(id=f"n{i}", width=rng.uniform(10, 130), height=rng.uniform(9, 70), x=rng.uniform(-9, 39),
                        y=rng.uniform(-9, 39),
                        layoutOptions={"org.eclipse.elk.position": f"({rng.uniform(-70, 70)},{rng.uniform(-70, 70)})",
                                       "org.eclipse.elk.radial.orderId": rng.randrange(20)})
            if k % 4 == 0 and i % 2:
                node["labels"] = [dict(text="fractional", x=rng.uniform(-10, 10), y=rng.uniform(-8, 8),
                                       width=rng.uniform(10, 100), height=rng.uniform(8, 30))]
            nodes.append(node)
        opts = {"org.eclipse.elk.algorithm": "radial", "org.eclipse.elk.radial.radius": rng.uniform(40, 150),
                "org.eclipse.elk.radial.sorter": ["NONE", "ID", "POLAR_COORDINATE"][k % 3],
                "org.eclipse.elk.radial.wedgeCriteria": ["LEAF_NUMBER", "NODE_SIZE"][k % 2]}
        if k % 5 == 0:
            opts["org.eclipse.elk.radial.optimizationCriteria"] = "EDGE_LENGTH"
        if k % 7 == 0:
            opts.update({"org.eclipse.elk.radial.rotate": True, "org.eclipse.elk.radial.rotation.targetAngle":
                         rng.uniform(-4, 4)})
        if k % 9 == 0:
            opts["org.eclipse.elk.radial.compactor"] = "RADIAL_COMPACTION"
        g = dict(id="g", layoutOptions=opts, children=nodes,
                 edges=[dict(id=f"e{i}", sources=[f"n{rng.randrange(i)}"], targets=[f"n{i}"]) for i in range(1, n)])
        rows.append(dict(name=f"fractional-{k}", input=g))
    return rows


def trig_bits():
    """2,393 float64 bit patterns for javacompat JSMathSin/JSMathCos."""
    bits = [_bits(x) for x in [0.0, -0.0, 1e8, -1e8, 1e16, -1e16]]
    rng = random.Random(747171)
    bits += [_bits(rng.uniform(-1000, 1000)) for _ in range(1000)]
    for k in range(-64, 65):
        x = k * math.pi / 2
        bits += [_bits(x), _bits(math.nextafter(x, -math.inf)), _bits(math.nextafter(x, math.inf))]
    rng = random.Random(72743)
    bits += [(rng.randrange(2) << 63) | (rng.randrange(1044, 2047) << 52) | rng.getrandbits(52) for _ in range(1000)]
    assert len(bits) == 2393
    return bits


def scalar_pairs():
    """2,009 (x, y) float64 bit pairs for Go math Acos(x), Atan2(y, x), Hypot(x, y)."""
    rng = random.Random(728281)
    rows = [[_bits(rng.uniform(-1, 1)), _bits(rng.uniform(-300, 300))] for _ in range(1000)]
    for _ in range(1000):
        rows.append([(rng.randrange(2) << 63) | (rng.randrange(2047) << 52) | rng.getrandbits(52),
                     (rng.randrange(2) << 63) | (rng.randrange(2047) << 52) | rng.getrandbits(52)])
    rows.extend([[0, 0], [1 << 63, 0], [0, 1 << 63], [1 << 63, 1 << 63], [0x7ff0000000000000, 0x7ff0000000000000],
                 [0xfff0000000000000, 0xfff0000000000000], [0x7ff0000000000000, 0xfff0000000000000],
                 [0x7ff8000000000001, _bits(1.)], [_bits(.5), 0x7ff8000000000001]])
    assert len(rows) == 2009
    return rows
