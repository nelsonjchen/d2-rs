#!/usr/bin/env python3
"""Import path stack and complete-byte parsed-AST caching, not IR attachment.

Fixture generator for recipe `lang-import-loading` (tools/fixtures/langlibelk-language.toml):
captures tools/field-oracle `import-loading` at the manifest pin and writes
import-loading.json to the staged output. `cargo test -p d2-rs-language
--features oracle-fixtures` does the native comparison (docs/FIXTURES.md).
"""
from language_oracle import call, field_oracle, fixture, write

go = field_oracle()
requests=[]
for root in ['', 'main.d2', 'dir/main.d2', '/root/main.d2', '../root/./main.d2']:
 for file in ['', 'main','main.d2','main.D2','lib.txt','.d2','.', '..','../lib','a//b/../c','/absolute/lib','a\\b','你好','lib.d2/']:
  for pre in ['', '../', './sub']:
   requests.append(dict(ops=[dict(op='push',file=root),dict(op='push',file=file,pre=pre,located=True),dict(op='pop')]))
requests += [dict(ops=[dict(op='push',file='root.d2'),dict(op='push',file='a'),dict(op='push',file='b'),dict(op='push',file='a',located=True)]),dict(ops=[dict(op='push',file='',located=True)])]
for utf16 in [False,True]:
 for data in [b'a: first',b'a: {b: value}',b'a: "hi"',b'a: [1;2]',b'a: {',b'\xffbad', '😀: text\nb: bad {'.encode(),b'\xff\xfe'+ '😀: value'.encode('utf-16-le'),b'\xff\xfea\x00:\x00\x01']:
  requests.append(dict(utf16=utf16,files={'lib.d2':data.hex()},ops=[dict(op='load',file='lib.d2'),dict(op='mutate'),dict(op='write',file='lib.d2',hex=b'changed: replacement'.hex()),dict(op='load',file='lib.d2'),dict(op='clear-errors'),dict(op='load',file='lib.d2')]))
 for ops in [
  ['error','load','clear-errors','load','error','load'],
  ['load','error','load','mutate','load'],
  ['load','load','write','clear-errors','load'],
 ]:
  requests.append(dict(utf16=utf16,files={'lib.d2':b'a: first'.hex()},ops=[dict(op=op,file='lib.d2',hex=b'a: second'.hex()) for op in ops]))
requests.append(dict(files={},ops=[dict(op='load',file='missing.d2'),dict(op='write',file='missing.d2',hex=b'a: now'.hex()),dict(op='load',file='missing.d2')]))
reference = call(go, requests, 'import-loading', timeout=600)
assert len(reference) == len(requests)
write(fixture('import-loading.json'), dict(requests=requests, reference=reference), compact=True)
print(f'{len(requests)} import-loading requests captured')
