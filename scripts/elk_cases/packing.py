"""elk-go Box and Rectpacking matrices (crates/d2-rs-elk/src/{box_layout,rectpacking/mod}.rs).

`box_grouping()` and `rectpacking_options()` return the [{name, input}]
cases of crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture
order, recovered from the 2026-10-19 Box grouping and Rectpacking option
generators.
"""
import copy
import random


def box_grouping():
    rng = random.Random(9019)
    cases = []
    sets = [[(30, 20)], [(30, 20)] * 7, [(5, 8), (8, 9), (15, 10), (120, 70)],
            [(200, 180), (65, 70), (25, 20), (12, 9), (4, 8), (2, 2)],
            [(50, 40), (40, 50), (80, 25), (25, 80), (2, 100), (100, 2)]]
    for _ in range(15):
        sets.append([(rng.randrange(8, 900) / 8, rng.randrange(8, 800) / 8) for i in range(rng.randrange(3, 15))])
    for mode in ["SIMPLE", "GROUP_INC", "GROUP_DEC", "GROUP_MIXED"]:
        for expand in [False, True]:
            for seed, sizes in enumerate(sets):
                opts = {"elk.algorithm": "box", "elk.box.packingMode": mode, "elk.expandNodes": expand}
                if seed % 3 == 0:
                    opts.update({"elk.padding": "[top=3.25,left=7.5,bottom=12.25,right=5.75]",
                                 "elk.spacing.nodeNode": 7.123456789, "elk.aspectRatio": 0.731})
                if seed % 4 == 0:
                    opts.update({"elk.nodeSize.minimum": "(230,190)", "elk.nodeSize.constraints": "[MINIMUM_SIZE]"})
                cases.append(dict(name=f"{mode}-{expand}-{seed:02}", input={
                    "id": "root", "layoutOptions": opts,
                    "children": [dict(id=f"n{i}", width=w, height=h, x=i * 7, y=i * 3) for i, (w, h) in enumerate(sizes)]}))
    # Explicit child constraints: leaves resize before packing, but area sorting occurs first.
    for mode in ["SIMPLE", "GROUP_INC", "GROUP_DEC", "GROUP_MIXED"]:
        for fixed in [False, True]:
            g = {"id": "root", "layoutOptions": {"elk.algorithm": "box", "elk.box.packingMode": mode,
                                                 "elk.expandNodes": True, "elk.direction": "RIGHT"}, "children": []}
            for i, (w, h) in enumerate([(120, 60), (30, 20), (10, 10), (5, 5)]):
                g["children"].append(dict(
                    id=f"n{i}", width=w, height=h,
                    layoutOptions={"elk.nodeSize.constraints": "[MINIMUM_SIZE, PORTS]",
                                   "elk.nodeSize.minimum": f"({w / 2},{h / 2})",
                                   "elk.portConstraints": "FIXED_POS" if fixed else "FREE"},
                    ports=[dict(id=f"p{i}", x=w, y=h / 3, width=4, height=6, layoutOptions={"elk.port.side": "EAST"})],
                    labels=[dict(id=f"l{i}", text="label", x=w, y=h / 4, width=12, height=8)]))
            cases.append(dict(name=f"{mode}-constraints-{fixed}", input=g))
    # Nested Fixed keeps exact local sections/labels; expansion translates content, not owner ports.
    for mode in ["SIMPLE", "GROUP_INC", "GROUP_DEC", "GROUP_MIXED"]:
        for alignment in ["[H_LEFT,V_TOP]", "[H_CENTER,V_CENTER]", "[H_RIGHT,V_BOTTOM]"]:
            g = {"id": "root", "layoutOptions": {"elk.algorithm": "box", "elk.box.packingMode": mode,
                                                 "elk.expandNodes": True, "elk.nodeSize.minimum": "(400,300)",
                                                 "elk.nodeSize.constraints": "[MINIMUM_SIZE]"},
                 "children": [], "edges": []}
            for i, (w, h) in enumerate([(160, 70), (24, 16), (20, 12), (8, 5)]):
                g["children"].append(dict(
                    id=f"n{i}", width=w, height=h,
                    layoutOptions={"elk.algorithm": "fixed", "elk.nodeSize.fixedGraphSize": True,
                                   "elk.contentAlignment": alignment},
                    children=[dict(id=f"a{i}", x=1, y=2, width=3, height=4), dict(id=f"b{i}", x=7, y=8, width=2, height=3)],
                    edges=[dict(id=f"e{i}", sources=[f"a{i}"], targets=[f"b{i}"],
                                sections=[dict(id=f"s{i}", startPoint={"x": 4, "y": 4}, endPoint={"x": 7, "y": 9},
                                               bendPoints=[{"x": 5, "y": 9}])],
                                labels=[dict(text="e", x=5, y=6, width=2, height=3)], junctionPoints=[{"x": 5, "y": 9}])]))
            cases.append(dict(name=f"{mode}-nested-{alignment}", input=g))
    return cases


RECTPACKING_PROFILES = [
    ("default", {}), ("area", {"widthApproximation.optimizationGoal": "AREA_DRIVEN"}),
    ("aspect", {"widthApproximation.optimizationGoal": "ASPECT_RATIO_DRIVEN"}),
    ("no-shift", {"widthApproximation.lastPlaceShift": False}), ("simple", {"packing.strategy": "SIMPLE"}),
    ("none", {"packing.strategy": "NONE"}),
    ("width-small", {"widthApproximation.strategy": "TARGET_WIDTH", "widthApproximation.targetWidth": 99}),
    ("width-large", {"widthApproximation.strategy": "TARGET_WIDTH", "widthApproximation.targetWidth": "500"}),
    ("reevaluate", {"packing.compaction.rowHeightReevaluation": True}),
    ("iterations-4", {"packing.compaction.iterations": 4}),
    ("whitespace-equal", {"whiteSpaceElimination.strategy": "EQUAL_BETWEEN_STRUCTURES"}),
    ("whitespace-aspect", {"whiteSpaceElimination.strategy": "TO_ASPECT_RATIO"}),
    ("simple-whitespace", {"packing.strategy": "SIMPLE", "whiteSpaceElimination.strategy": "TO_ASPECT_RATIO"}),
    ("trybox", {"trybox": True}), ("ordered", {"orderBySize": True}), ("interactive", {"interactive": True}),
    ("forced-rows", {}),
    ("all", {"packing.compaction.rowHeightReevaluation": True, "packing.compaction.iterations": 3,
             "widthApproximation.optimizationGoal": "ASPECT_RATIO_DRIVEN", "orderBySize": True,
             "whiteSpaceElimination.strategy": "TO_ASPECT_RATIO"}),
    ("invalid-enum", {"packing.strategy": "simple", "widthApproximation.strategy": "bad",
                      "whiteSpaceElimination.strategy": "bad"}),
    ("ordinal-enum", {"packing.strategy": 1, "widthApproximation.optimizationGoal": "2",
                      "whiteSpaceElimination.strategy": "0"})]

# Single-node micro-layout bases (the last six original_rectpacking_default.json controls).
_ONE = {"id": "n0", "width": 40, "height": 30}
MICRO_BASES = [
    ("child-minimum", dict(_ONE, layoutOptions={"elk.nodeSize.constraints": "MINIMUM_SIZE",
                                                 "elk.nodeSize.minimum": "(80,90)"})),
    ("child-default-minimum", dict(_ONE, layoutOptions={"elk.nodeSize.constraints": "MINIMUM_SIZE"})),
    ("label-minimum", dict(_ONE, layoutOptions={"elk.nodeSize.constraints": "NODE_LABELS",
                                                 "elk.nodeLabels.placement": "INSIDE H_CENTER V_CENTER"},
                           labels=[{"text": "L", "width": 100, "height": 50}])),
    ("outside-label", dict(_ONE, layoutOptions={"elk.nodeSize.constraints": "NODE_LABELS",
                                                 "elk.nodeLabels.placement": "OUTSIDE H_CENTER V_TOP"},
                           labels=[{"text": "L", "width": 100, "height": 50}])),
    ("fixed-port", dict(_ONE, layoutOptions={"elk.portConstraints": "FIXED_POS"},
                        ports=[{"id": "p", "x": 0, "y": 10, "width": 2, "height": 3,
                                "layoutOptions": {"elk.port.side": "EAST", "elk.port.borderOffset": 4}}])),
    ("label-position", dict(_ONE, layoutOptions={"elk.nodeLabels.placement": "H_RIGHT V_BOTTOM"},
                            labels=[{"text": "L", "width": 10, "height": 5}])),
]
MICRO_PROFILES = [("simple", {"packing.strategy": "SIMPLE"}), ("none", {"packing.strategy": "NONE"}),
                  ("equal", {"whiteSpaceElimination.strategy": "EQUAL_BETWEEN_STRUCTURES"}),
                  ("aspect", {"whiteSpaceElimination.strategy": "TO_ASPECT_RATIO"}),
                  ("reeval", {"packing.compaction.rowHeightReevaluation": True}),
                  ("iterate", {"packing.compaction.iterations": 5}), ("trybox", {"trybox": True}),
                  ("no-shift", {"widthApproximation.lastPlaceShift": False}),
                  ("area", {"widthApproximation.optimizationGoal": "AREA_DRIVEN"}),
                  ("narrow", {"widthApproximation.strategy": "TARGET_WIDTH", "widthApproximation.targetWidth": 60})]


def _profiles(qualified):
    rows = []
    for k in range(9):
        rng = random.Random(727 + k)
        children = [dict(id=f"n{i}", width=rng.randrange(11, 130), height=rng.randrange(11, 110)) for i in range(k + 1)]
        for name, profile in RECTPACKING_PROFILES:
            nodes = copy.deepcopy(children)
            for i, n in enumerate(nodes):
                if name == "interactive" and i % 2 == 0:
                    n["layoutOptions"] = {"rectpacking.desiredPosition": str((i + 1) % 3)}
                if name == "forced-rows" and i % 2 == 1:
                    n["layoutOptions"] = {"rectpacking.inNewRow": True}
            options = {"algorithm": "rectpacking",
                       **{("rectpacking." + key if not key.startswith("interactive") else key): value
                          for key, value in profile.items()}}
            if k % 3 == 1:
                options.update({"nodeSize.constraints": "MINIMUM_SIZE", "nodeSize.minimum": "(500,400)",
                                "contentAlignment": "H_CENTER V_BOTTOM"})
            if k % 3 == 2:
                options["padding"] = "[top=2,left=7,bottom=11,right=19]"
            if qualified:
                options = {("org.eclipse.elk." + key): value for key, value in options.items()}
                for n in nodes:
                    if "layoutOptions" in n:
                        n["layoutOptions"] = {"org.eclipse.elk." + key: value for key, value in n["layoutOptions"].items()}
            rows.append(dict(name=("qualified-" if qualified else "") + f"{name}-{k}",
                             input=dict(id="g", layoutOptions=options, children=nodes)))
    return rows


def _micro():
    rows = []
    for base_name, node in MICRO_BASES:
        for name, options in MICRO_PROFILES:
            graph = {"id": "root", "layoutOptions": {"elk.algorithm": "rectpacking"}, "children": [copy.deepcopy(node)]}
            graph["layoutOptions"].update({"org.eclipse.elk.rectpacking." + k: v for k, v in options.items()})
            rows.append(dict(name="micro-" + base_name + "-" + name, input=graph))
    return rows


def _varied():
    rows = []
    for seed in range(100):
        rng = random.Random(9301 + seed)
        packing = rng.choice(["COMPACTION", "SIMPLE", "NONE"])
        whitespace = rng.choice(["NONE", "EQUAL_BETWEEN_STRUCTURES", "TO_ASPECT_RATIO"]) if packing != "NONE" else "NONE"
        opts = {"algorithm": "rectpacking", "padding": "[top=1.25,left=3.125,bottom=6.5,right=13.375]",
                "spacing.nodeNode": rng.choice([0, 0.1, 3.5, 17.625]), "aspectRatio": rng.choice([0.8, 1.25, 2.1]),
                "rectpacking.packing.strategy": packing, "rectpacking.whiteSpaceElimination.strategy": whitespace,
                "rectpacking.widthApproximation.optimizationGoal": rng.choice(["AREA_DRIVEN", "MAX_SCALE_DRIVEN",
                                                                                "ASPECT_RATIO_DRIVEN"]),
                "rectpacking.packing.compaction.iterations": rng.randint(0, 6),
                "rectpacking.packing.compaction.rowHeightReevaluation": rng.choice([True, False]),
                "rectpacking.widthApproximation.lastPlaceShift": rng.choice([True, False])}
        if seed % 2 == 0:
            opts.update({"rectpacking.widthApproximation.strategy": "TARGET_WIDTH",
                         "rectpacking.widthApproximation.targetWidth": rng.randint(60, 400) + 0.125})
        if seed % 3 == 0:
            opts.update({"nodeSize.constraints": "MINIMUM_SIZE", "nodeSize.minimum": "(355.625,233.125)",
                         "contentAlignment": "H_CENTER V_CENTER"})
        nodes = [dict(id=f"n{i}", width=rng.randint(11, 140) + rng.choice([0, 0.125, 0.3]),
                      height=rng.randint(5, 180) + rng.choice([0, 0.125, 0.2])) for i in range(rng.randint(2, 14))]
        if seed % 5 == 0:
            for i, n in enumerate(nodes):
                if i % 3 == 1:
                    n["layoutOptions"] = {"org.eclipse.elk.rectpacking.inNewRow": True}
        rows.append(dict(name=f"varied-{seed}", input=dict(id="g", layoutOptions={"org.eclipse.elk." + k: v
                                                                                  for k, v in opts.items()},
                                                            children=nodes)))
    return rows


def rectpacking_options():
    return _profiles(False) + _profiles(True) + _micro() + _varied()


def rectpacking_default():
    """Default Rectpacking: five hand-picked node sets, 80 random ones (2..16 nodes), then MICRO_BASES.

    Options cycle per row through none, asymmetric padding, node spacing and a
    centered minimum size (2026-10-19 Rectpacking generator).
    """
    profiles = [{}, {"elk.padding": "[top=10,left=20,bottom=30,right=40]"}, {"elk.spacing.nodeNode": 17.1234567},
                {"elk.nodeSize.constraints": "MINIMUM_SIZE", "elk.nodeSize.minimum": "(300,200)",
                 "elk.contentAlignment": "H_CENTER V_CENTER"}]
    sets = [[(40, 30)], [(40, 30)] * 2, [(40, 30)] * 7, [(200, 100), (40, 30), (60, 70), (20, 80)],
            [(20, 200), (80, 30), (200, 20), (40, 150), (90, 70)]]
    rng = random.Random(90210)
    for k in range(5, 85):
        sets.append([(rng.randint(10, 200), rng.randint(10, 200)) for _ in range(2 + (k - 5) % 15)])
    rows = [dict(name=str(k), input={"id": "root", "layoutOptions": {"elk.algorithm": "rectpacking", **profiles[k % 4]},
                                     "children": [dict(id=f"n{i}", width=w, height=h) for i, (w, h) in enumerate(sizes)]})
            for k, sizes in enumerate(sets)]
    rows += [dict(name=name, input={"id": "root", "layoutOptions": {"elk.algorithm": "rectpacking"},
                                    "children": [copy.deepcopy(node)]}) for name, node in MICRO_BASES]
    return rows


def project_rectpacking(output):
    """The root size and every child's id, box, labels and ports of a raw Rectpacking output (rectpacking/mod.rs)."""
    def child(node):
        shape = {key: node[key] for key in ("id", "x", "y", "width", "height")}
        for key, fields in [("labels", ("text", "x", "y", "width", "height")), ("ports", ("id", "x", "y", "width", "height"))]:
            if key in node:
                shape[key] = [{field: item[field] for field in fields} for item in node[key]]
        return shape
    return dict(width=output["width"], height=output["height"], children=[child(n) for n in output["children"]])


def rectpacking_errors():
    """Rectpacking option combinations elk-go rejects; rows record the provider error."""
    one = [{"id": "a", "width": 30, "height": 40}]
    width = {"algorithm": "rectpacking", "rectpacking.widthApproximation.strategy": "TARGET_WIDTH"}
    return [
        dict(name="missing-width", input={"id": "g", "layoutOptions": dict(width), "children": copy.deepcopy(one)}),
        dict(name="invalid-width", input={"id": "g", "layoutOptions": {
            **width, "rectpacking.widthApproximation.targetWidth": "bad"}, "children": copy.deepcopy(one)}),
        dict(name="negative-position", input={"id": "g", "layoutOptions": {"algorithm": "rectpacking", "interactive": True},
                                              "children": [dict(one[0], layoutOptions={"rectpacking.desiredPosition": -2})]}),
        dict(name="none-whitespace", input={"id": "g", "layoutOptions": {
            "algorithm": "rectpacking", "rectpacking.packing.strategy": "NONE",
            "rectpacking.whiteSpaceElimination.strategy": "TO_ASPECT_RATIO"}, "children": copy.deepcopy(one)}),
    ]


def box_simple():
    """SIMPLE packing of five node sets under default, asymmetric-padding and spacing options.

    The fixture keeps only the root size and the children's boxes of the raw
    output (`project_boxes`).
    """
    sets = [[(40, 30)], [(40, 30)] * 2, [(90, 10), (10, 90), (45, 35), (20, 20)],
            [(200, 100), (10, 10), (40, 30), (60, 70), (20, 80)], [(10, 10)] * 7]
    profiles = [("default", {}), ("asymmetric", {"elk.padding": "[top=10,left=20,bottom=30,right=40]"}),
                ("spacing", {"elk.spacing.nodeNode": 17.1234567})]
    return [dict(name=f"{k}-{name}", input={
        "id": "root", "layoutOptions": {"elk.algorithm": "box", **extra},
        "children": [dict(id=f"n{i}", width=w, height=h) for i, (w, h) in enumerate(sizes)]})
        for k, sizes in enumerate(sets) for name, extra in profiles]


def project_boxes(output):
    """The root size and every child's x/y/width/height of a raw Box output."""
    return dict(width=output["width"], height=output["height"],
                children=[{key: child[key] for key in ("x", "y", "width", "height")} for child in output["children"]])


def box_grouping_expanded():
    """Port inference, nested Box scopes and flat edge carriers per packing mode (2026-10-19 expansion)."""
    cases = []
    grouping = box_grouping()
    for mode in ["SIMPLE", "GROUP_INC", "GROUP_DEC", "GROUP_MIXED"]:
        for direction in ["RIGHT", "LEFT", "UP", "DOWN"]:
            for fixed in [False, True]:
                nodes = [dict(id=f"n{i}", width=w, height=h,
                              layoutOptions={"elk.nodeSize.constraints": "[PORTS, MINIMUM_SIZE]",
                                             "elk.nodeSize.minimum": "(3.5,4.75)",
                                             "elk.portConstraints": "FIXED_POS" if fixed else "FREE"},
                              ports=[dict(id=f"p{i}", x=x, y=y, width=5.5, height=7.25)])
                         for i, (w, h, x, y) in enumerate([(50, 40, -2, 6), (30, 20, 32, 7), (90, 50, 4, -4),
                                                           (15, 10, 4, 14)])]
                cases.append(dict(name=f"{mode}-port-inference-{direction}-{fixed}", input=dict(
                    id="root", layoutOptions={"elk.algorithm": "box", "elk.box.packingMode": mode,
                                              "elk.direction": direction, "elk.expandNodes": True},
                    children=nodes)))
        for constrained in [False, True]:
            for childmode in ["SIMPLE", "GROUP_INC", "GROUP_DEC", "GROUP_MIXED"]:
                nodes = [dict(id=f"n{i}", width=40, height=30,
                              layoutOptions={"elk.algorithm": "box", "elk.box.packingMode": childmode,
                                             "elk.expandNodes": i == 0, "elk.nodeSize.minimum": "(130,110)",
                                             "elk.nodeSize.constraints": "[MINIMUM_SIZE]" if constrained else "[]"},
                              children=[dict(id=f"a{i}", width=10.5, height=20.25),
                                        dict(id=f"b{i}", width=12.75, height=5.25),
                                        dict(id=f"c{i}", width=7.5, height=3.5)])
                         for i in range(3)]
                cases.append(dict(name=f"{mode}-nested-box-{childmode}-{constrained}", input=dict(
                    id="root", layoutOptions={"elk.algorithm": "box", "elk.box.packingMode": mode,
                                              "elk.expandNodes": True, "elk.nodeSize.minimum": "(300,250)",
                                              "elk.nodeSize.constraints": "[MINIMUM_SIZE]" if constrained else "[]"},
                    children=nodes)))
        # Imported flat edge carriers must follow their actual child container.
        for c in grouping:
            if c["name"] == f"{mode}-nested-[H_CENTER,V_CENTER]":
                c = copy.deepcopy(c)
                c["name"] = f"{mode}-flat-carriers"
                for child in c["input"]["children"]:
                    c["input"]["edges"].extend(child.pop("edges"))
                cases.append(c)
    return cases
