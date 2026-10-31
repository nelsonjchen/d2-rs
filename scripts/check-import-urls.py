#!/usr/bin/env python3
"""Capture exactly the Go HTML/URL predicate reachable from D2 extendLinks.

Fixture generator for recipe `lang-import-url` (tools/fixtures/langlibelk-language.toml):
builds the stdlib-only tools/import-url-oracle with the manifest Go toolchain
(no D2 checkout involved), feeds it the requests below and writes import-url.json
to the staged output. `cargo test -p d2-rs-language --features oracle-fixtures`
does the native comparison (docs/FIXTURES.md).
"""
import re
import subprocess
from pathlib import Path
from fixture_recipe import go_env, workdir
from language_oracle import call, fixture, write
from references import ROOT

go = workdir() / 'bin/import-url-oracle'
subprocess.run(['go', 'build', '-o', str(go), str(ROOT / 'tools/import-url-oracle/main.go')], cwd=workdir(), check=True, env=go_env())
stdlib = Path(subprocess.check_output(['go', 'env', 'GOROOT'], env=go_env(), cwd=workdir(), text=True).strip()) / 'src'
names=re.findall(r'^\s*"([^"]+)":', (stdlib/'html/entity.go').read_text(),re.M)
inputs=[b'',b'&',b'&#',b'&#0',b'&#x',b'&#x;',b'&#;',b'&unknown;',b'&nLt;',b'&nGt;']
for name in names:
    for text in ['&'+name,'&'+name+'tail','&'+name.rstrip(';')+'=','/'+name+'&'+name]:inputs.append(text.encode())
for value in [0,1,9,10,13,31,47,58,127,128,159,160,0xd7ff,0xd800,0xdfff,0xe000,0x10ffff,0x110000,0x7fffffff,0x80000000,0xffffffff,0x100000000,0x10000002f]:
    for radix in [str(value),'x'+format(value,'x'),'X'+format(value,'X')]:
        for suffix in ['',';','x',';/',':foo']:inputs.append(('&#'+radix+suffix).encode())
urls=['http://host/a','HTTP://a:1,b:2/a','postgres://a:1,b:2/a','//host','//host/a','///host','%2Frelative','relative','a:b','a:%ZZ','a:/%ZZ','1:a','cache_object:foo','http://[::1]/','http://[127.0.0.1]/','http://[v1.x]/','http://[fe80::1%25en0]/','http://[fe80::1%25en%20x]/','http://user:p@ss@host/a','http://a%25b/a','http://a%2Fb/a','http://x:9999999999/a','http://x:/a','a:opaque?%ZZ','/a?%ZZ','/a#%ZZ','/a#\n','&#47;relative','http&#58;//host/a','&sol;&sol;host','&sol;&sol;host/a']
inputs += [u.encode() for u in urls]
for template in [b'http://host%s/a',b'http://u%sp@host/a',b'http://[fe80::1%%25en%s]/',b'/a%s',b'/a#%s',b'/a?%s',b'%s:opaque',b'http://host:80%s/a']:
    for byte in range(256):
        inputs.append(template % bytes([byte]));inputs.append(template % f'%{byte:02X}'.encode())
for address in ['::','::1','1:2:3:4:5:6:7:8','1::2::3','::ffff:192.0.2.1','::ffff:192.00.2.1','1:2:3:4:5:6:7','12345::','::1%25','::1%25%FF','::1%25x%2Fy','::1%25x%00y','::1%25x%2525y','[::1','::1]']:
    for suffix in ['',':',':80',':-1',':x',':%38%30','extra']:
        inputs.append(f'http://[{address}]{suffix}/a'.encode())
requests=list(dict.fromkeys(b.hex() for b in inputs))
reference=call(go,requests)
write(fixture('import-url.json'),{'requests':requests,'reference':reference})
print(f'{len(requests)} HTML/URL predicate cases captured')
