#!/usr/bin/env python3
"""Pin Go Unicode SimpleFold equivalence representatives for IR lookups."""
import json,subprocess
from references import ROOT,REFERENCE,verify_source,digest,MANIFEST
verify_source()
go=REFERENCE/'bin/parser-oracle';record=json.loads(go.with_suffix('.provenance.json').read_text())
assert record['binary_sha256']==digest(go)
assert record['harness_sha256']==digest(ROOT/'tools/parser-oracle/main.go')
assert record['go_toolchain']==MANIFEST['d2']['go_toolchain']
data=json.loads(subprocess.check_output([str(go),'unicode']))
output='// SPDX-License-Identifier: MPL-2.0\n// Generated from pinned Go 1.27.0 Unicode '+data['version']+'.\n'
output+='pub(super) fn simple_fold(r:char)->char{let r=r as u32;let i=FOLD.partition_point(|p|p.0<r);char::from_u32(FOLD.get(i).filter(|p|p.0==r).map_or(r,|p|p.1)).unwrap()}\n'
output+='const FOLD:&[(u32,u32)]=&[\n'+''.join(f'(0x{a:x},0x{b:x}),\n' for a,b in data['fold'])+'];\n'
output+='pub(super) fn to_lower(r:char)->char{let r=r as u32;let i=LOWER.partition_point(|p|p.0<r);char::from_u32(LOWER.get(i).filter(|p|p.0==r).map_or(r,|p|p.1)).unwrap()}\n'
output+='const LOWER:&[(u32,u32)]=&[\n'+''.join(f'(0x{a:x},0x{b:x}),\n' for a,b in data['lower'])+'];\n'
path=ROOT/'crates/d2-rs-core/src/ir/fold.rs';path.write_text(output)
subprocess.run(['rustfmt','--edition','2024',str(path)],check=True)
report_path = ROOT / 'oracle-cache/reports/ir-unicode-reference.json'
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps({'toolchain':record['go_toolchain'],'unicode':data['version'],'fold_mappings':len(data['fold']),'lower_mappings':len(data['lower']),'oracle_sha256':digest(go),'table_sha256':digest(path)},indent=2)+'\n')
