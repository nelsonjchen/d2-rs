"""elk-go Layered connected-component matrices (crates/d2-rs-elk/src/model_order_contract.rs).

Each public function returns the [{name, input}] cases of
crates/d2-rs-elk/src/fixtures/original_<function>.json in fixture order,
recovered from the 2026-10-17 component probes.
"""

STACK = [("first", 13.53, 5.29), ("second", 23.17, 7.71), ("third", 9.29, 3.13)]


def _packing(direction, count, dims, profile, seed):
    integer = dims == "integer"
    spacing = 23.57 if dims == "decimal" else 23.5
    g = dict(id="root", layoutOptions={"elk.algorithm": "layered", "elk.direction": direction, "elk.randomSeed": seed,
                                       "elk.spacing.componentComponent": spacing}, children=[], edges=[])
    for index in range(count):
        if integer:
            w, h = 70 + 7 * index, 35 + 3 * index
        elif dims == "fractional":
            w, h = 61.25 + 7.125 * index, 26.5 + 3.0625 * index
        else:
            w, h = 61.1 + 7.13 * index, 26.3 + 3.07 * index
        a, b = f"g{index}.a", f"g{index}.b"
        g["children"].append(dict(id=a, width=w, height=h))
        if profile == "isolated":
            continue
        if profile == "loop":
            loop = (13.53, 5.29) if dims == "decimal" else (13.5, 5.25)
            g["edges"].append(dict(id=f"e{index}", sources=[a], targets=[a],
                                   labels=[dict(text="loop", width=loop[0], height=loop[1])]))
            continue
        if profile == "loop-stack":
            g["edges"].append(dict(id=f"e{index}", sources=[a], targets=[a],
                                   labels=[dict(text=t, width=lw, height=lh) for t, lw, lh in STACK]))
            continue
        if integer:
            bw, bh = 49 + 5 * index, 39 + 2 * index
        elif dims == "fractional":
            bw, bh = 40.875 + 5.25 * index, 36.125 + 1.375 * index
        else:
            bw, bh = 40.87 + 5.29 * index, 36.12 + 1.37 * index
        g["children"].append(dict(id=b, width=bw, height=bh))
        edge = dict(id=f"e{index}", sources=[a], targets=[b])
        if profile in ("center-inline", "center-noninline"):
            size = (17.23, 7.71) if dims == "decimal" else (17.25, 7.75)
            edge["labels"] = [dict(text="center", width=size[0], height=size[1],
                                   layoutOptions={"elk.edgeLabels.inline": profile == "center-inline"})]
        elif profile.endswith("-stack"):
            edge["labels"] = [dict(text=t, width=lw, height=lh,
                                   layoutOptions={"elk.edgeLabels.inline": profile == "center-inline-stack"})
                              for t, lw, lh in STACK]
        g["edges"].append(edge)
    return dict(name=f"packing-{direction.lower()}-n{count}-{dims}-{profile}-seed{seed}", input=g)


def component_world_packing():
    rows = []
    for dims_list, profiles in [(["integer", "fractional"], ["isolated", "path", "loop", "center-inline", "center-noninline"]),
                                (["decimal"], ["isolated", "path", "loop", "center-inline", "center-noninline"]),
                                (["decimal"], ["loop-stack", "center-inline-stack", "center-noninline-stack"])]:
        for direction in ["RIGHT", "DOWN", "LEFT", "UP"]:
            for count in [2, 3, 5]:
                for dims in dims_list:
                    for profile in profiles:
                        for seed in [1, 42]:
                            rows.append(_packing(direction, count, dims, profile, seed))
    return rows
