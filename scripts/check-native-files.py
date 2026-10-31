#!/usr/bin/env python3
"""Compare real native filesystem open/read outcomes to pinned Go."""
import hashlib, json, os, platform, subprocess
from pathlib import Path
from references import ROOT, REFERENCE, MANIFEST
output = REFERENCE / 'bin/native-file-oracle'
source = ROOT / 'tools/native-file-oracle/main.go'
subprocess.run(['go','build','-o',str(output),str(source)],check=True,env={**os.environ,'GOTOOLCHAIN':MANIFEST['d2']['go_toolchain'],'GOMODCACHE':str(REFERENCE/'go-mod')})
subprocess.run(['cargo','build','-p','d2-rs-language','--example','native-file-oracle'],cwd=ROOT,check=True)
folder = ROOT / 'target/native-file-witnesses'
folder.mkdir(parents=True,exist_ok=True)
(folder/'regular.d2').write_bytes(b'a -> b\n')
(folder/'empty').write_bytes(b'')
(folder/'large').write_bytes(bytes(range(256))*32)
for name,target in [('broken','missing'),('loop','loop')]:
    link=folder/name
    if link.is_symlink(): link.unlink()
    link.symlink_to(target)
paths=[str(folder/p) for p in ['regular.d2','empty','large','missing','broken','loop','regular.d2/child']]+[str(folder),'','nul\0path',str(folder/('x'*300))]
payload=json.dumps(paths).encode()
def run(exe):
    return json.loads(subprocess.run([str(exe)],input=payload,stdout=subprocess.PIPE,check=True).stdout)
go=run(output)
rust=run(ROOT/'target/debug/examples/native-file-oracle')
assert go==rust, [(p,g,r) for p,g,r in zip(paths,go,rust) if g!=r]
report={'go_toolchain':MANIFEST['d2']['go_toolchain'],'host':platform.platform(),'cases':len(paths),'matching':len(paths),'harness_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'binary_sha256':hashlib.sha256(output.read_bytes()).hexdigest(),'coverage':['regular and empty files','4096-byte read boundary','missing and dangling symlink','symlink loop','non-directory parent','directory read','empty/NUL/overlong names'],'unqualified':['Windows','other host kernels','permission/resource exhaustion','interrupt injection','non-UTF8 filenames','compiler import attachment']}
report_path = ROOT / 'oracle-cache/reports/native-file-parity.json'
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report,indent=2)+'\n')
print(f'{len(paths)} native file cases match pinned Go')
