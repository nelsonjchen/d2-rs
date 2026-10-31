#!/usr/bin/env python3
"""Compare source representation choice and full boxed AST strings."""
import json
import random
import subprocess

from references import ROOT, REFERENCE, verify_source, digest

source = verify_source()
values = ["", "plain", "null", "suspend", "unsuspend", "true", "false", "42",
          "a-b", "a-", "-", "--", "-é", "a--b", "$x", 'a"b', 'a"\nb', "a.b"]
for char in "#;\n\\{}[]'\"|:.-<>*&()@$":
    values.extend([char, "a" + char, char + "a", "a" + char + "b"])
for code in [*range(9, 14), 32, 0x85, 0xa0, 0x1680, *range(0x2000, 0x200c), 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff]:
    values.extend([chr(code) + "x", "x" + chr(code), "x" + chr(code) + "y"])
rng = random.Random(900)
alphabet = "ab0é🦀-#;\n\\{}[]'\"|:<>*&()@$ \t"
values.extend("".join(rng.choices(alphabet, k=rng.randrange(1, 24))) for _ in range(200))
requests = [{"text": value, "in_key": in_key} for value in values for in_key in (False, True)]
payload = json.dumps(requests).encode()
go = REFERENCE / "bin/d2-oracle"
rust = ROOT / "target/debug/examples/raw-strings-oracle"
left = json.loads(subprocess.check_output([str(go), "raw-strings"], input=payload))
right = json.loads(subprocess.check_output([str(rust)], input=payload))
differences = [{"request": requests[index], "go": a, "rust": b}
               for index, (a, b) in enumerate(zip(left, right)) if a != b]
matched = len(left) == len(right) == len(requests) and not differences
report = {"source_sha256": digest(source / "d2ast/d2ast.go"),
          "oracle_sha256": digest(go), "rust_sha256": digest(rust),
          "cases": len(requests), "matched": matched, "differences": differences,
          "scope": "valid Unicode input; full boxed AST JSON, exact string values", "parser_parity": False}
report_path = ROOT / "oracle-cache/reports/raw-string-parity.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(not matched)
