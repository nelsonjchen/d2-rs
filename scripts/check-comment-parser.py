#!/usr/bin/env python3
"""Line comment production, diagnostics and remaining input.

Fixture generator for recipe `lang-parser-comment` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes comment.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
inputs=[b"#",b"# ",b"#  two",b"#\ttab",b"# one\n# two\nnext",b"# one\n\n# separate",b"# one\r\n  # two",b"# one\n  next",b"# \xff\xe2\x82", "# é😀\n\u2000# next".encode(),b"#"+b"a"*2048+b"\n# tail"]
requests=[]
for data in inputs:
    for utf16 in [False,True]:
        for fail in [-1,1,2,3,5,8]:
            requests.append({"hex":data.hex(),"utf16":utf16,"fail":fail,"mode":"comment"})
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('comment.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} comment.json requests captured")
