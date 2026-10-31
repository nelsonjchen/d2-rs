#!/usr/bin/env python3
"""Raw indentation bytes, including invalid UTF-8 intermediates.

Fixture generator for recipe `lang-parser-indent` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes indent.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
inputs=[b"",b"x",b"  x\n    y\n",b"\t x\n \ty",b"\n\t\n ",b"\xff  ",b" \xe2\x82\n  ",b"a\n\n",b" \r\nx \t"]
for space in [" ","\t","\r","\x85","\xa0","\u1680","\u2000","\u2028","\u2029","\u202f","\u205f","\u3000"]:
    for content in [space,space+"x",space+"x\n"+space*2+"y\n",space+"\t x\n "+space+"y"]:
        inputs.append(content.encode())
requests=[{"hex":data.hex(),"max":maximum,"mode":"indent"} for data in inputs for maximum in [-1,0,1,2,3,4,5,8]]
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('indent.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} indent.json requests captured")
