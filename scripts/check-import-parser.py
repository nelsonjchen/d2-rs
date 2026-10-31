#!/usr/bin/env python3
"""Import production prefixes, paths and partial errors.

Fixture generator for recipe `lang-parser-imports` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes imports.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
requests=[]
for path in ["", "file", "file.d2", "file.d2.object", "file.D2", "'file'.d2", "file.'d2'", "../file.d2.obj", "../../file", "/file.d2", ".../file", "./", "file\nrest", "file.\"obj.x\"", "file.é😀", "file d2"]:
 for spread in [False,True]:
  for utf16 in [False,True]:
   for fail in [-1,4,5,8]:
    text=("...@" if spread else "@")+path
    requests.append({"hex":text.encode().hex(),"mode":"import","utf16":utf16,"fail":fail,"in_key":False,"edge":False,"spread":spread})
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('imports.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} imports.json requests captured")
