#!/usr/bin/env python3
"""Compare block string production and raw scalar bytes.

Fixture generator for recipe `lang-parser-block` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes block.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
go = parser_oracle()
inputs=[b"|",b"|md",b"| ",b"| hi | tail",b"|md\n  hello\n| next",b"||md\n  a|b\n||",b"|!md\n x!|",b"|!?md\n x?!|",b"|md\n\t a\n  b\n|",b"|md\r\n  a\r\n|",b"|md\n \xff\xe2\x82|", "|md\n\u2000x\n\u2000y\n|".encode(), "|😀md\n hi😀| tail".encode(), "|!😀md\n x😀!|".encode(), "|é tag |".encode(), "|١ text |".encode(),b"|md\n"+b"a"*2048+b"|"]
requests=[]
for data in inputs:
    for utf16 in [False,True]:
        for depth in [0,2]:
            for fail in [-1,1,2,3,5,8]:
                requests.append({"hex":data.hex(),"utf16":utf16,"fail":fail,"mode":"block","depth":depth})
reference = call(go, requests)
write(fixture('block.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} block-string requests captured')
