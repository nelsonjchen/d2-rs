#!/usr/bin/env python3
"""Bit-exact scalar oracle fixtures for crates/d2-rs-elk (tools/fixtures/langlibelk-elk-scalars.toml).

usage: check-elk-scalars.py trig|math|log|random|registry

  trig    javacompat.JSMathSin/JSMathCos (elk-go internal)  -> original_radial_trig.json
  math    Go math Acos/Atan2/Hypot                          -> original_radial_math.json
  log     Go math.Log (the stress/force logarithm)          -> original_force_logarithm.json
  random  Go math/rand/v2 ChaCha8 words and float bits      -> original_spore_random.json
  registry  elk-go OptionRegistry.BySuffix over every option-ID dot suffix
            -> original_option_resolution.json; also checks that the
            committed runtime table src/fixtures/original_option_ids.txt
            equals the registry's ID list (it is read by utility.rs, so it
            stays committed, like generated library source)

Inputs come from scripts/elk_cases/radial.py and stress.py. Oracles are arm64 builds
(scripts/elk_oracle.py). `cargo test -p d2-rs-elk --features oracle-fixtures`
compares.
"""
import json
import sys

from elk_cases import radial, stress
from elk_oracle import MODULE, build, run, write_json
from fixture_recipe import output
from references import ROOT

FIXTURES = "crates/d2-rs-elk/src/fixtures/"
# Keys beside the registry's own dot suffixes: wrong case, absent, and a full-ID near miss.
REGISTRY_PROBES = ["Interactive", "PADDING", "doesNotExist", "org.eclipse.elk.spacing.nodeNodeX"]


def call(oracle, value=None, env=None):
    q = run(oracle, b"" if value is None else json.dumps(value).encode(), env=env)
    if q.returncode:
        sys.exit(q.stderr.decode())
    return json.loads(q.stdout)


kind = sys.argv[1]
if kind == "registry":
    oracle = build(ROOT / "tools/elk-registry-oracle/main.go", "elk-registry-oracle",
                   module_path=MODULE + "/d2rs-registry-oracle")
    ids = call(oracle)
    committed = (ROOT / FIXTURES / "original_option_ids.txt").read_text()
    if committed != "\n".join(ids) + "\n":
        sys.exit("crates/d2-rs-elk/src/fixtures/original_option_ids.txt differs from the pinned elk-go registry")
    keys = {""} | set(REGISTRY_PROBES)
    for option in ids:
        parts = option.split(".")
        keys |= {".".join(parts[k:]) for k in range(len(parts))}
    keys = sorted(keys)
    q = run([*oracle, "resolve"], json.dumps(keys).encode())
    if q.returncode:
        sys.exit(q.stderr.decode())
    resolved = json.loads(q.stdout)
    assert list(resolved) == keys
    write_json(output(FIXTURES + "original_option_resolution.json"), resolved)
    sys.exit(0)
if kind == "random":
    rows = call(build(ROOT / "tools/elk-random-oracle/main.go", "elk-random-oracle"))
    write_json(output(FIXTURES + "original_spore_random.json"), rows, indent=None)
    sys.exit(0)
oracle = build(ROOT / "tools/elk-math-oracle/main.go", "elk-math-oracle", module_path=MODULE + "/d2rs-math-oracle")
if kind == "trig":
    bits = radial.trig_bits()
    results = call(oracle, bits)
    assert len(results) == len(bits)
    rows = [dict(input_bits=a, sin_bits=b[0], cos_bits=b[1]) for a, b in zip(bits, results)]
    write_json(output(FIXTURES + "original_radial_trig.json"), rows)
elif kind == "math":
    pairs = radial.scalar_pairs()
    results = call(oracle, pairs, env={"D2RS_RADIAL_SCALARS": "1"})
    assert len(results) == len(pairs)
    rows = [dict(x_bits=a[0], y_bits=a[1], acos_bits=b[0], atan2_bits=b[1], hypot_bits=b[2])
            for a, b in zip(pairs, results)]
    write_json(output(FIXTURES + "original_radial_math.json"), rows)
elif kind == "log":
    bits = stress.log_bits()
    results = call(oracle, bits, env={"D2RS_FORCE_LOG": "1"})
    assert len(results) == len(bits)
    rows = [dict(input_bits=a, output_bits=b) for a, b in zip(bits, results)]
    write_json(output(FIXTURES + "original_force_logarithm.json"), rows)
else:
    sys.exit(f"unknown scalar fixture {kind}")
