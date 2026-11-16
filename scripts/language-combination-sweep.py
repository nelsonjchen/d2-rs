#!/usr/bin/env python3
"""Language interaction hunt, using the differential scoreboard's CLI/budgets.

All normal differential-scoreboard arguments apply. Example:
  uv run --no-project python scripts/language-combination-sweep.py \
    --go /path/to/clock-oracle --epoch 946782245 \
    --seed 2026111612 --cap 5000 --jobs 5 --output oracle-cache/reports/H12.json
"""
import importlib.util
from pathlib import Path
import random
import re
import sys


spec = importlib.util.spec_from_file_location(
    "differential", Path(__file__).with_name("differential-scoreboard.py"))
diff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diff)


def upstream_cases():
    # assertCompile inputs and dsl fields include vars/glob contracts absent
    # from the stock script:-only extraction. Retain their original text.
    cases = list(diff.inputs())
    path = diff.UPSTREAM / "d2compiler/compile_test.go"
    text = path.read_text()
    pattern = re.compile(r'(?:assertCompile\(t,\s*|\bdsl:\s*)`([^`]*)`', re.S)
    cases.extend((f"d2compiler/compile_test.go:contract:{i}", m[1])
                 for i, m in enumerate(pattern.finditer(text)))
    unique = {}
    for name, source in cases:
        if 0 < len(source) <= 1800 and len(source.splitlines()) <= 65:
            # Network assets are outside the H hunt. Local imports remain and
            # are deliberately exercised both missing and through fixtures.
            if re.search(r'\bicon\s*:', source):
                continue
            unique.setdefault(source, name)
    return [(name, source) for source, name in unique.items()]


def combination_jobs(seed, cap):
    rng = random.Random(seed)
    corpus = upstream_cases()
    rng.shuffle(corpus)
    families = ("import-spread", "vars-spread", "class-override",
                "filtered-glob", "board-null")
    seen = set()
    for index in range(cap * 8):
        family = families[index % len(families)]
        name, original = corpus[(index // len(families)) % len(corpus)]
        color = rng.choice(("red", "blue", "orange", "#123456"))
        shape = rng.choice(("rectangle", "circle", "oval", "person"))
        fixtures = {}
        if family == "import-spread":
            fixtures = {"lib/model.d2": original,
                        "lib/relay.d2": "...@model\n"}
            source = rng.choice(("...@lib/model\n", "...@lib/relay\n",
                                 "model: @lib/model\n"))
            source += rng.choice((f"**.style.stroke: {color}\n",
                                  "scenarios: {review: {model: null}}\n",
                                  "model: {extra -> node}\n"))
        elif family == "vars-spread":
            source = (f"vars: {{hunt_style: {{style.fill: {color}; shape: {shape}}}; "
                      "hunt_text: hello}\n" + original + "\n")
            source += rng.choice(("probe: {...${hunt_style}; label: ${hunt_text}}\n",
                                  "vars.hunt_copy: ${hunt_style}\nprobe: ${hunt_copy}\n",
                                  "box: {vars.hunt_text: local; probe: ${hunt_text}}\n",
                                  "vars.hunt_text: null\nprobe: ${hunt_text}\n"))
        elif family == "class-override":
            source = (f"classes: {{hunt_base: {{style.fill: {color}}}; "
                      f"hunt_alt: {{shape: {shape}; style.bold: true}}}}\n")
            source += original + "\n" + rng.choice((
                "probe: {class: [hunt_base; hunt_alt]; style.fill: null}\n",
                "**: {class: hunt_base}\nscenarios: {review: {probe.class: hunt_alt}}\n",
                "classes.hunt_base.style.fill: null\nprobe.class: hunt_base\n",
                "probe: {class: hunt_alt; shape: null; child.class: hunt_base}\n"))
        elif family == "filtered-glob":
            selector = rng.choice(("&leaf: true", "!&leaf: true", "&level: 1",
                                   "&connected: true", "&shape: circle", "&link: *"))
            glob = f"**: {{{selector}; style.fill: {color}}}\n"
            source = (glob + original if rng.randrange(2) else original + "\n" + glob)
            source += rng.choice(("\nprobe: {a -> b; c}\n",
                                  "\nscenarios: {review: {probe: {a -> b}}}\n",
                                  "\nprobe: {shape: circle}\nprobe: null\nprobe\n"))
        else:
            collection = rng.choice(("layers", "scenarios", "steps"))
            deletion = rng.choice(("a: null", "a -> b: null", "classes: null",
                                   "vars: null", "a.style.fill: null"))
            source = original + (f"\n{collection}: {{review: {{{deletion}; "
                                 "probe -> next}; final: {probe: null; next}}\n")
        options = ["--layout=" + rng.choice(("dagre", "elk", "tala"))]
        identity = (source, tuple(options), tuple(sorted(fixtures.items())))
        if identity in seen:
            continue
        seen.add(identity)
        yield (name, source, "combined-" + family, options, fixtures)
        if len(seen) == cap:
            return
    raise RuntimeError("could not generate requested number of distinct jobs")


if __name__ == "__main__":
    diff.language_jobs = combination_jobs
    if "--language-matrix" not in sys.argv:
        sys.argv.append("--language-matrix")
    sys.exit(diff.main())
