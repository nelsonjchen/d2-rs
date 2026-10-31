#!/usr/bin/env python3
"""Capture the inline upstream d2parser TestParse corpus through public Parse.

Fixture generator for recipe `lang-upstream-parser` (tools/fixtures/langlibelk-language.toml):
mines every inline TestParse case from the pinned d2parser/parse_test.go with
tools/parser-oracle, replays each with UTF16Pos=false and true, checks the
UTF16Pos=false results against upstream's own testdata/d2parser snapshots and
writes upstream-parser.json to the staged output. `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, upstream, write
from references import MANIFEST, digest

go = parser_oracle()
source = upstream()
go_test = source / "d2parser/parse_test.go"
upstream_cases = call(go, None, "parse-fixtures", str(go_test))
expanded_cases = [(case, utf16) for case in upstream_cases for utf16 in (False, True)]
requests = [{"hex": case["hex"], "mode": "public_parse", "utf16": utf16, "options": "default", "path": case["path"]}
            for case, utf16 in expanded_cases]
reference = call(go, requests)
assert len(requests) == len(reference)

snapshot_hashes = {}
for index, case in enumerate(upstream_cases):
    snapshot_path = source / "testdata/d2parser" / (case["name"] + ".exp.json")
    default_result = reference[index * 2]
    assert json.loads(snapshot_path.read_text()) == {"ast": default_result["map"], "err": default_result["structured"]}, case["name"]
    snapshot_hashes[snapshot_path.relative_to(source).as_posix()] = digest(snapshot_path)

write(fixture("upstream-parser.json"), {
    "upstream_commit": MANIFEST["d2"]["commit"],
    "upstream_parse_test_sha256": digest(go_test),
    "upstream_snapshot_sha256": snapshot_hashes,
    "cases": [{"name": case["name"], "utf16": utf16, "path": case["path"], "hex": case["hex"], "reference": result}
              for (case, utf16), result in zip(expanded_cases, reference)],
})
print(f"{len(upstream_cases)} upstream TestParse cases captured with UTF16Pos=false/true")
