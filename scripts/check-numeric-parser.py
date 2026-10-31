#!/usr/bin/env python3
"""Exact numeric acceptance with the pinned Go math/big.Rat.SetString.

Fixture generator for recipe `lang-parser-numeric` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes numeric.json to the
staged output. `cargo test -p d2-rs-language --features oracle-fixtures` does
the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, fixture, parser_oracle, write

go = parser_oracle()
values=set(["","+","-",".","NaN","Inf"," 1","1 ","1e1000001","1p10000001","0e1000001","0p10000001","1e9223372036854775808","0e9223372036854775808","1e-9223372036854775808","0e-9223372036854775808"])
mantissas=["0","1","12","08","001","1.25",".5","1.","0x1.f","0X.F","0b1.1","0o7.4","0x1e2","0_1","1_2.3_4","0x_ff","0b_10","0o_7","0x","1__2","1_.2","1._2","_1","1_","0x_","0b2","0o8","1..2"]
for sign in ["","+","-"]:
 for mantissa in mantissas:
  for suffix in ["","e2","E-2","p3","P-3","e+1_0","e_1","e1_","p","e1__2"]:
   values.add(sign+mantissa+suffix)
for numerator in ["0","1","-1","+2","01","08","0x10","0b11","1_0","1.5",""]:
 for denominator in ["1","0","-1","+1","01","08","0x10","0b11","1_0","1.5",""]:
  values.add(numerator+"/"+denominator)
requests=[{"hex":v.encode().hex(),"mode":"number"} for v in sorted(values)]
reference = call(go, requests)
assert len(reference) == len(requests)
write(fixture('numeric.json'), {"requests": requests, "reference": reference})
print(f"{len(requests)} numeric.json requests captured")
