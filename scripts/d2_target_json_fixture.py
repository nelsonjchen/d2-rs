#!/usr/bin/env python3
"""Shape d2-target-json-probe hook results into public-target-json-v090.json.

Recipe `d2-target-json` (after `d2-target-json-probe`). The probe decodes each
committed raw input (tools/fixtures/hooks/d2-target-json-probe/cases.json) with the pinned
d2target model and Go encoding/json, re-encodes it, and hashes it. The
fixture keeps the expected outcome, the metadata projection the Rust test
compares (fields absent from Diagram.Bytes), the HashID and the SHA-256 of
the exact hash-input bytes.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = "crates/d2-rs-render/tests/fixtures/public-target-json-v090.json"
RESULTS = "tools/d2-target-json-probe/results.json"
KEEP = ("name", "isFolderOnly", "description", "fontFamily", "monoFontFamily", "legend", "layers", "scenarios", "steps")


def projection(value):
    if value is None:
        return None
    out = {k: v for k, v in value.items() if k in KEEP}
    for key in ("layers", "scenarios", "steps"):
        if isinstance(out.get(key), list):
            out[key] = [projection(v) for v in out[key]]
    return out


def main() -> None:
    commit = os.environ.get("D2RS_CAPTURE_COMMIT") or json.loads((ROOT / "reference-manifest.json").read_text())["d2"]["commit"]
    inputs = json.loads((ROOT / "tools/fixtures/hooks/d2-target-json-probe/cases.json").read_text())
    results = json.loads((Path(os.environ["D2RS_FIXTURES_DIR"]) / RESULTS).read_text())
    if [r["name"] for r in results] != [i["name"] for i in inputs]:
        sys.exit("probe results do not match the committed inputs")
    cases = []
    for given, got in zip(inputs, results):
        row = dict(given)
        outcome = got["outcome"]
        row["expected_outcome"] = outcome
        if outcome in ("decoded", "source-panic"):
            row["expected_projection"] = projection(json.loads(got["canonical"]))
        if outcome == "decoded":
            row["expected_hash"] = got["hash"]
            row["expected_hash_input_sha256"] = hashlib.sha256(base64.b64decode(got["hash_input"])).hexdigest()
        cases.append(row)
    fixture = {"provenance": f"Pinned Go D2 {commit}, Go1.27 encoding/json v1; tools/fixtures/hooks/d2-target-json-probe/d2rs_target_json_test.go. "
                             "Hash input SHA256 values are exact original bytes, with no normalization. Projection checks "
                             "fields absent from Diagram.Bytes, including descendant metadata. One source hash panic "
                             "remains unqualified.",
               "cases": cases}
    fixture_recipe.output(OUT).write_text(json.dumps(fixture, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
