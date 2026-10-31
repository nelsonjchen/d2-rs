#!/usr/bin/env python3
"""Compare file-map, container, value and edge-key parser productions.

Fixture generator for recipe `lang-parser-containers` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes containers.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
from references import ROOT, digest
from upstream_inputs import E2E_LABEL, e2e_inputs
go = parser_oracle()
cases={
"file":["","a","a: b\nc -> d: edge","a: 1 {b: [true; false; null]}\na.b -> c", "# one\n# two\na: 0x1.fp2",'""" comment """ a: b',"...@file.d2\n...${vars}","}: a\n: value\n[]", "a: 'x' trailing\nb: next", "a: |md text | trailing\nb: ok", "a: {b: {c: d}}", "a: [suspend; unsuspend; 3/4; [x; y]]", "a -> ", "(a -> b)[*].style: {stroke: red}", "a: |md\n  é😀\n|", "!&*.shape: circle\n&**: {x: y}", "a: true${x}","a: faLſe\nb: ſuſpend", "a: {b: x\n", "a: [x\n", "a --\\\n -> b"],
"map":["{}","{a: b} tail","{a: [x; y]; b: 2}","{a: b","{;#c\nx}","{:x}","{a: \"\"\"c\"\"\"}"],
"array":["[]","[a; b] tail","[true; suspend; unsuspend; false]","[...${a}; ...@file]",'["""c""" a]',"[a","['a' b]","[x\n#comment\ny]"],
"value":["","null","NULL","ſuſpend","unſuſpend","faLſe","true${x}","0x1.fp-2","1/-2","1e1000001","'true'","${x}","@../file.d2.x","[x; {a: b}]","a: b"],
"mapkey":["", "a", "a: b", ": v", "a {b: c}","a: label {x: y}","&a","!&a","!x", "a -> b -> c", "a <-> b", "a -- b", "a --* b", "-> b", "a ->", "a-", "a>b", "(a -> b)[2].label: x", "a.(b -> c)[*].style", "(a -> b", "(a -> b x", "a --\\x b", "a --\\\n-- b"],
"index":["[0]","[*]","[12 3]","[1x2]","[١]","[9223372036854775808]","[1","[","[\n]","[-1]","[x]","[1\n]","[999999999999999999999999999x]"],
}
requests=[{"hex":text.encode().hex(),"mode":mode,"utf16":utf16,"fail":fail} for mode,texts in cases.items() for text in texts for utf16 in [False,True] for fail in [-1,1,4,12]]
# Committed D2 seeds, then the upstream e2e inputs read from the pinned tree
# (formerly copied to tests/upstream/d2).
fixtures=[(path, str(path.relative_to(ROOT))) for path in sorted((ROOT / "tests/d2").glob("*.d2"))]
fixtures+=[(path, f"{E2E_LABEL}/{path.name}") for path in e2e_inputs()]
for path, label in fixtures:
 for utf16 in [False,True]:
  requests.append({"hex":path.read_bytes().hex(),"mode":"file","utf16":utf16,"fail":-1,"fixture":label,"sha256":digest(path)})
reference = call(go, requests)
write(fixture('containers.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} container requests captured')
