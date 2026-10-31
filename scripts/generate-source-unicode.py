from pathlib import Path
import json
import argparse
import os
root=Path(__file__).resolve().parent.parent
# The Go evidence is regenerated, not tracked: recipe mathtext-txt-go-unicode
# (uv run --no-project python scripts/regen-fixtures.py --only mathtext-txt-go-unicode).
fixtures=root/os.environ.get('D2RS_FIXTURES_DIR','oracle-cache/fixtures')
parser=argparse.ArgumentParser(description="Translate pinned original D2 Unicode lookup evidence into native Rust tables")
parser.add_argument('--source-json',type=Path,default=fixtures/'vendor/rustybuzz-go-v090/tests/fixtures/go-unicode-v090.json')
args=parser.parse_args()
r=json.loads(args.source_json.read_text());ccc=[];mirrors=[]
for x in r['ranges']:
 start,end,script,category,value,mirror=x[:6]
 if value:
  if ccc and ccc[-1][1]+1==start and ccc[-1][2]==value:ccc[-1][1]=end
  else:ccc.append([start,end,value])
 if mirror:
  mirrors.extend((cp,cp+mirror) for cp in range(start,end+1))
s='''// SPDX-License-Identifier: Unlicense OR BSD-3-Clause
// Copyright 2021 The go-text authors; Rust translation Copyright 2026 Nelson Chen.
// Generated from pinned original go-text/typesetting v0.3.4 Unicode 17 lookups.
// Regenerate with uv run python scripts/generate-source-unicode.py.
pub const UNICODE_VERSION: (u8,u8,u8) = (17,0,0);
const COMBINING: &[(u32,u32,u8)] = &[\n'''
s+=''.join(f'    (0x{a:x},0x{b:x},{c}),\n' for a,b,c in ccc)+'];\nconst MIRROR: &[(u32,u32)] = &[\n'+''.join(f'    (0x{a:x},0x{b:x}),\n' for a,b in mirrors)+'];\n'
compositions={ (x[0],x[1]):x[2] for x in r['composition'][:r['stored_composition_count']] if x[3] }
s+='const COMPOSITION: &[(u64,u32)] = &[\n'+''.join(f'    (0x{((a<<32)|b):x},0x{c:x}),\n' for (a,b),c in sorted(compositions.items()))+'];\n'
s+='pub fn compose(a:char,b:char)->Option<char>{ let k=((a as u64)<<32)|(b as u64); COMPOSITION.binary_search_by_key(&k,|r|r.0).ok().and_then(|i|char::from_u32(COMPOSITION[i].1)) }\n'
s+='''pub fn combining_class(c: char) -> u8 {
    let cp=c as u32; let i=COMBINING.partition_point(|r|r.0<=cp);
    if i>0 && cp<=COMBINING[i-1].1 { COMBINING[i-1].2 } else { 0 }
}
pub fn mirrored(c: char) -> Option<char> {
    MIRROR.binary_search_by_key(&(c as u32),|r|r.0).ok().and_then(|i|char::from_u32(MIRROR[i].1))
}
'''
(root/'vendor/rustybuzz-go-v090/src/hb/source_unicode_data.rs').write_text(s);print({'ccc_ranges':len(ccc),'mirror_entries':len(mirrors)})
