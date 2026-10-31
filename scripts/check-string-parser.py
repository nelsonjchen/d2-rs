#!/usr/bin/env python3
"""Compare the recursive key/string/substitution production group.

Fixture generator for recipe `lang-parser-strings` (tools/fixtures/langlibelk-language.toml):
captures tools/parser-oracle at the manifest pin and writes strings.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import json
from language_oracle import call, fixture, parser_oracle, write
go = parser_oracle()
requests=[]
cases={
 "key":["","a","a.b", ".a", "a..b", "a . 'b.c'", "a\n.b", "a-", "a-\n", "a->b", "a-*b", "*.**.***", "@bad", "a"*519, "é"*260, "|md\n  x\n|.b"],
 "string":[""," a ","\nkey","'a'",'"a"',"|md\n a\n|", "${x}","a: b","a\xff", "(a)"],
 "unquoted":["*${a}é*","","hello world  ","a\\tb\\n","a\\","a\\\n  b","a\\\n\nb","a-","a-;","a->b","a--b","a-*b","a-b","...@x","...","$","${a}","before${a.b}after", "${'a.b'}tail", "a$bad", "a${x", "a${\nx}", "*a**b", "a\\*b", "a) .b", "a)z", "é😀", "a\\a\\b\\f\\r\\t\\v"],
 "double":['""','"hello" tail','"a\\tb"','"${a}tail"','"before${a}after${b}end"','"$bad"','"a\\','"a\nb"','"a\\\nb"','"${a"'],
 "sub":["$","${}","${a}","${a.b}","$bad","$\n{a}","${a","${a\n}","${'a.b'}","${|md x |}"]
}
for mode,inputs in cases.items():
 for data in inputs:
  for utf16 in [False,True]:
   for in_key in ([False,True] if mode in ("string","unquoted","double") else [False]):
    for fail in [-1,1,3,6]:
     requests.append({"hex":data.encode().hex(),"mode":mode,"utf16":utf16,"fail":fail,"in_key":in_key,"edge":True})
requests=[{**r,"edge":edge} for r in requests for edge in [False,True]]
for mode,data in [("key",b"a\xff.b"),("unquoted",b"a\xff\xe2\x82"),("double",b'"a\xff"')]:
 for utf16 in [False,True]:
  requests.append({"hex":data.hex(),"mode":mode,"utf16":utf16,"fail":-1,"in_key":False,"edge":False})
for text in ["...${a}","...${a.b}","...${a", "...$bad"]:
 for utf16 in [False,True]:
  requests.append({"hex":text.encode().hex(),"mode":"sub","utf16":utf16,"fail":-1,"in_key":False,"edge":False,"spread":True})
reference = call(go, requests)
write(fixture('strings.json'), {'requests': requests, 'reference': reference})
print(f'{len(requests)} key/string requests captured')
