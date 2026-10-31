#!/usr/bin/env python3
"""Generate original normal-font Content substitutions and logical metrics."""
import argparse,json,re,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('source',type=Path);p.add_argument('output',type=Path);a=p.parse_args()
pattern=re.compile(r'\s*(0x[0-9A-Fa-f]+):\s+\{Metrics: Metrics\{Height: ([^,]+), Depth: ([^,]+), Width: ([^,}]+)(?:[^}]*)\}, Content: ("(?:\\.|[^"\\])*")\},')
rows=[(int(c,16),float(h),float(d),float(w),json.loads(content)) for c,h,d,w,content in pattern.findall(a.source.read_text())];rows.sort()
operators=(a.source.parent.parent/'tex/operator_dictionary_generated.go').read_text()
large_source=(a.source.parent/'data_largeop_gen.go').read_text()
large={int(c,16):(float(h),float(d),float(w)) for c,h,d,w,_ in pattern.findall(large_source)}
classes={}
for operator,cl in re.findall(r'Operator: "([^"\n]+)", Definition: mjOperatorDefinition\{[^}]*TexClass: (\d+)',operators):
 if len(operator)==1:classes.setdefault(ord(operator),int(cl))
text='// SPDX-License-Identifier: MPL-2.0\n// Source-derived from mathjax-go v0.1.0 font/data_normal_gen.go,\n// MathJax 3.2.2 Apache-2.0. See SOURCE.md and THIRD_PARTY_NOTICES.md.\n'
text+='pub(crate) struct Compound {pub codepoint:u32,pub height:f64,pub depth:f64,pub width:f64,pub class:usize,pub large:Option<(f64,f64,f64)>,pub content: &\'static str}\n'
text+='pub(crate) const COMPOUNDS: &[Compound] = &[\n'
for c,h,d,w,content in rows:
 large_tuple='Some('+str(large[c])+')' if c in large else 'None'
 text+=f'Compound {{codepoint:0x{c:X},height:{h!r},depth:{d!r},width:{w!r},class:{classes.get(c,0)},large:{large_tuple},content:{json.dumps(content,ensure_ascii=False)}}},\n'
text+='];\n';a.output.write_text(text);subprocess.run(['rustfmt','--edition','2024',str(a.output)],check=True);print('compound entries',len(rows))
