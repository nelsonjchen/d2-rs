"""Request corpus of the public d2parser.Parse fixture (recipe `lang-public-parse`).

Shared with the format and AST-clone generators, which reuse its
`public_parse` inputs, so none of them reads another recipe's oracle output.
"""


def public_parse_requests() -> list[dict]:
    inputs=[b"",b"a: b",b"a: [x",b"\xff",b"\xff\xfe",b"\xfe\xff",b"\xef\xbb\xbfa: b",b"a\xff: b"]
    for text in ["a: b","é😀: 中文","a: [x","a: |md\n  x\n|","\ufeffa: b"]:
     inputs.extend([text.encode(),b"\xff\xfe"+text.encode("utf-16le"),b"\xfe\xff"+text.encode("utf-16be")])
    for words in [[0xd800],[0xdc00],[0xd800,0xdc00],[0xdc00,0xdc01],[0xd800,0xd801,0xdc00],[0xdc00,0xd800,0xdc00],[0x61,0x3a,0x20,0xdc00,0xdc01]]:
     data=b"\xff\xfe"+b"".join(w.to_bytes(2,"little") for w in words)
     inputs.extend([data,data+b"\x61"])
    requests=[{"hex":data.hex(),"mode":"public_parse","utf16":utf16,"options":options,"path":"folder,a-é.d2"} for data in inputs for utf16 in [False,True] for options in ["none","default","empty","prior"]]
    helper_inputs = inputs + [s.encode() for s in ["a.b", "a -> b", "(a -> b)[0].style", "[1; true] trailing", "{a: b}", "null", "true${x}", "a.", "a: [", "\"", "a; tail", "\n", "# comment", "${x}", "@a.d2", "|md hi|", "é.*", "(a -> )"]]
    helper_inputs += [bytes([b]) for b in range(256)]
    for data in helper_inputs:
     for mode in ["public_key", "public_mapkey", "public_value"]:
      requests.append({"hex":data.hex(),"mode":mode,"utf16":False,"options":"none","path":""})
    return requests


def public_parse_inputs() -> list[bytes]:
    return [bytes.fromhex(r["hex"]) for r in public_parse_requests() if r["mode"] == "public_parse"]
