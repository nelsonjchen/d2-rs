#!/usr/bin/env python3
"""Compare partial stream parsing, chunk boundaries, BOM handling and I/O errors.

Fixture generator for recipe `lang-stream-parser` (tools/fixtures/langlibelk-language.toml):
captures tools/field-oracle at the manifest pin and writes stream-parser.json to the staged
output. `cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
from language_oracle import call, field_oracle, fixture, write
go = field_oracle()
requests=[]
inputs=[b'',b'a: value',b'a: {b: 1}',b'a: "quoted"',b'a -> b',b'# comment\na: [1;2]',b'a: |md\nhello\n|',b'\xffa: b',b'a: \xe2\x82',b'\xef\xbb\xbfa: b',b'\xfe\xffa\x00', '😀: value'.encode(),b'\xff\xfe'+ '😀: value'.encode('utf-16-le'),b'\xff\xfea\x00:\x00\x01',b'\xff\xfe\x00\xd8',b'\xff\xfe\x00\xdc\x00\xdc',b'\xff\xfe\x00\xd8a\x00']
for data in inputs:
 for chunk in [1,2,3,4,0]:
  for cutoff in [None,*range(len(data)+1)]:
   for combined in [False,True]:
    requests.append(dict(hex=data.hex(),chunk=chunk,error_at=cutoff,combined=combined,utf16=False,seed=False))
for length in [4093,4094,4095,4096,4097]:
 for data in [('a: '+ 'x'*length+'😀').encode(),b'\xff\xfe'+('a: '+'x'*length+'😀').encode('utf-16-le')]:
  for cutoff in [None,4095,4096,len(data)-1,len(data)]:
   for combined in [False,True]:
    requests.append(dict(hex=data.hex(),error_at=cutoff,combined=combined))
for zero_reads in [1,2,99,100,101,199,200,201,1000]:
 requests.append(dict(hex=b'a: b'.hex(),zero_reads=zero_reads))
for utf16 in [False,True]:
 for seed in [False,True]:
  for cutoff in [None,1,4]:
   requests.append(dict(hex='😀: hi'.encode().hex(),error_at=cutoff,utf16=utf16,seed=seed))
requests += [dict(r,load=True) for r in requests]
reference=call(go,requests,'stream-parse',timeout=600)
assert len(reference)==len(requests)
write(fixture('stream-parser.json'),dict(requests=requests,reference=reference),compact=True)
print(f'{len(requests)} stream-parse requests captured')
