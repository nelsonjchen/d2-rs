#!/usr/bin/env python3
"""Derive exact Go IsLetter from the certified parser letter/digit union."""
import argparse,hashlib,json,re,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('output',type=Path);a=p.parse_args()
root=Path(__file__).resolve().parent.parent
source=root/'crates/d2-rs-language/src/parser/unicode.rs';record=json.loads((root/'docs/parser-unicode-reference.json').read_text())
assert record['go_toolchain']=='go1.27.0' and record['unicode_version']=='17.0.0'
assert hashlib.sha256(source.read_bytes()).hexdigest()==record['rust_table_sha256']
s=source.read_text()
def ranges(name):
 body=s.split('const '+name+': &[(u32, u32)] = &[',1)[1].split('];',1)[0]
 return [(int(x,16),int(y,16)) for x,y in re.findall(r'\(0x([0-9a-f]+), 0x([0-9a-f]+)\)',body)]
letters=set()
for lo,hi in ranges('LETTER_OR_DIGIT'):letters.update(range(lo,hi+1))
for lo,hi in ranges('DIGIT'):letters.difference_update(range(lo,hi+1))
out=[]
for cp in sorted(letters):
 if out and cp==out[-1][1]+1:out[-1][1]=cp
 else:out.append([cp,cp])
text='// SPDX-License-Identifier: MPL-2.0\n// Go 1.27.0 unicode.IsLetter, derived from the certified parser table.\n// Go Authors BSD-3-Clause; Unicode 17.0.0.\n'
text+='pub fn is_letter(r: char) -> bool { let r=r as u32;let i=LETTER.partition_point(|range|range.1<r);LETTER.get(i).is_some_and(|range|range.0<=r) }\n'
text+='const LETTER: &[(u32,u32)] = &[\n'+''.join(f'    (0x{lo:x},0x{hi:x}),\n' for lo,hi in out)+'];\n'
a.output.write_text(text);subprocess.run(['rustfmt','--edition','2024',str(a.output)],check=True);print('letter ranges',len(out),'codepoints',len(letters))
