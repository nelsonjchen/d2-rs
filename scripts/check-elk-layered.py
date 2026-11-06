#!/usr/bin/env python3
"""elk-go Layered/Box/Rectpacking fixture generator (tools/fixtures/langlibelk-elk-layered.toml).

usage: check-elk-layered.py FIXTURE

FIXTURE names crates/d2-rs-elk/src/fixtures/original_<FIXTURE>.json. Builds
tools/elk-oracle against the pinned elk-go module (arm64, scripts/elk_oracle.py),
feeds each case from scripts/elk_cases (its `raw_input` text when recorded,
otherwise the JSON input) to elk.LayoutJSON and writes rows
{name, input, [raw_input], expected} with the raw output (or a projection of
it, PROJECT). Every control must complete, except in the rejection fixtures
(PROVIDER_ERRORS, REJECTED), where elk-go must refuse every case.
`cargo test -p d2-rs-elk --features oracle-fixtures` compares.
"""
import json
import sys

from elk_cases import (adapter_wire, coffman, components, curated, hierarchy, identity, junctions, labels, layered,
                       model_order, packing, placement, spacing)
from elk_oracle import negative_zero_int, run, wire_oracle, write_json
from fixture_recipe import output

MODULES = [adapter_wire, coffman, components, curated, hierarchy, identity, junctions, labels, layered, model_order,
           packing, placement, spacing]
# Python's default int parsing for these (their captures went through a batch oracle without a -0 hook);
# every other capture kept Go's `-0` as -0.0.
PLAIN_INTEGERS = {"box_grouping", "box_grouping_expanded", "box_simple", "rectpacking_options"}
COMPACT = {"box_grouping", "box_grouping_expanded"}
INDENT = {"ancestor_descendant_edges": 1}
# Fixtures that keep a projection of the raw output.
PROJECT = {"box_simple": packing.project_boxes, "fixed": placement.project_fixed,
           "random_seeded": placement.project_random, "rectpacking_default": packing.project_rectpacking}
# Row key order where the fixture's recorded layout differs from {case..., expected}.
ROW_KEYS = {"label_identity": ["name", "raw_input", "expected", "input"],
            "junction_admission": ["name", "raw_input", "input", "expected"]}
# Written with raw UTF-8 instead of \u escapes.
UTF8 = {"label_identity"}
# Provider rejections: rows keep exit/stdout/stderr and the provider message after this prefix.
PROVIDER_ERRORS = {"rectpacking_errors": "layout ELK graph: layout algorithm org.eclipse.elk.rectpacking: "}
# Inputs elk-go must refuse to decode: rows {name, raw_input, source_error = stderr}.
REJECTED = {"invalid_label_identity", "invalid_junction_admission"}
DEADLINE = 30

fixture = sys.argv[1]
generators = [getattr(m, fixture) for m in MODULES if hasattr(m, fixture)]
if len(generators) != 1:
    sys.exit(f"no unique case generator for {fixture}")
oracle = wire_oracle()
rows = []
for case in generators[0]():
    stdin = case["raw_input"] if "raw_input" in case else json.dumps(case["input"])
    q = run(oracle, stdin.encode(), timeout=DEADLINE)
    if fixture in PROVIDER_ERRORS or fixture in REJECTED:
        stderr = q.stderr.decode()
        if q.returncode != 1:
            sys.exit(f"{case['name']}: elk-go exited {q.returncode}, expected a rejection: {stderr[:2000]}")
        if fixture in REJECTED:
            rows.append(dict(**case, source_error=stderr))
            continue
        prefix = PROVIDER_ERRORS[fixture]
        assert stderr.startswith(prefix) and stderr.endswith("\n"), stderr
        rows.append(dict(**case, exit=q.returncode, stdout=q.stdout.decode(), stderr=stderr,
                         provider_error=stderr[len(prefix):-1]))
        continue
    if q.returncode:
        sys.exit(f"{case['name']}: elk-go exited {q.returncode}: {q.stderr.decode()[:2000]}")
    parse = {} if fixture in PLAIN_INTEGERS else {"parse_int": negative_zero_int}
    expected = json.loads(q.stdout, **parse)
    row = dict(**case, expected=PROJECT[fixture](expected) if fixture in PROJECT else expected)
    rows.append({key: row[key] for key in ROW_KEYS[fixture]} if fixture in ROW_KEYS else row)
write_json(output(f"crates/d2-rs-elk/src/fixtures/original_{fixture}.json"), rows,
           indent=None if fixture in COMPACT else INDENT.get(fixture, 2), ensure_ascii=fixture not in UTF8)
print(f"{fixture}: {len(rows)} controls")
