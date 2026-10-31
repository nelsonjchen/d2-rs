#!/usr/bin/env python3
"""Compare block comment production and raw value bytes.

Fixture generator for recipe `lang-parser-block-comment` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes block-comment.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
go = parser_oracle()
inputs=[b'"""',b'""""""',b'"""hi""" next',b'"""\n  a\n  b\n"""',b'"""x"',b'"""x""',b'"""x"a""b"""',b'"""\t a\n  b\n"""',b'"""\r\n  x\r\n"""',b'""" \xff\xe2\x82"""', '"""\n\u2000a\n\u2000b\n"""'.encode(), '"""é😀中文"""'.encode(),b'"""'+b'a'*2048+b'"""']
requests=[]
for data in inputs:
    for utf16 in [False,True]:
        for depth in [0,2]:
            for fail in [-1,3,4,5,6,8]:
                requests.append({"hex":data.hex(),"utf16":utf16,"fail":fail,"mode":"block_comment","depth":depth})
reference = call(go, requests)
write(fixture('block-comment.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} block-comment requests captured')
