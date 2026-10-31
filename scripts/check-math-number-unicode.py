#!/usr/bin/env python3
"""Verify native numeric classification against the pinned Go Unicode oracle."""
import json, re, subprocess
from references import ROOT, REFERENCE, verify_source, digest, MANIFEST
verify_source()
oracle = REFERENCE / "bin/parser-oracle"
record = json.loads(oracle.with_suffix(".provenance.json").read_text())
assert record["go_toolchain"] == MANIFEST["d2"]["go_toolchain"]
assert record["binary_sha256"] == digest(oracle)
assert record["harness_sha256"] == digest(ROOT / "tools/parser-oracle/main.go")
data = json.loads(subprocess.check_output([str(oracle), "unicode"]))
path = ROOT / "crates/d2-rs-math/src/number_unicode.rs"
tables = {}
source = path.read_text()
for name in ["digit", "letter_or_digit"]:
    body = source.split("const " + name.upper() + ":", 1)[1].split("];", 1)[0]
    ranges = [[int(a, 16), int(b, 16)] for a, b in re.findall(r"\(0x([0-9a-f]+), 0x([0-9a-f]+)\)", body)]
    assert ranges == data["tables"][name]
    tables[name] = len(ranges)
print(json.dumps({"verified": True, "go_toolchain": record["go_toolchain"], "unicode_version": data["version"], "ranges": tables["digit"], "classification_ranges": tables, "oracle_sha256": digest(oracle), "rust_table_sha256": digest(path)}, indent=2))
