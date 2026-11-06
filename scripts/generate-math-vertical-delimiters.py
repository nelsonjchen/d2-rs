#!/usr/bin/env python3
"""Translate all pinned vertical delimiter recipes without executing an oracle."""
from pathlib import Path
import argparse
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('source', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
rows = []
for code, fields in re.findall(r'(0x[0-9A-Fa-f]+):\s+Delimiter\{Direction: DirectionVertical, (.*?)\},\n', args.source.read_text()):
    def array(name, kind):
        found = re.search(rf'\b{name}: \[\]{kind}\{{([^}}]*)\}}', fields)
        values = found[1] if found else ''
        if kind == 'float64': values = ', '.join(repr(float(v)) for v in values.split(',') if v.strip())
        return '[' + values + ']'
    def scalar(name, default):
        found = re.search(rf'\b{name}: ([^,}}]+)', fields)
        return found[1] if found else default
    hdw = re.search(r'HDW: \[3\]float64\{([^}]*)\}', fields)
    full = re.search(r'FullExt: \[2\]float64\{([^}]*)\}', fields)
    assert all(int(v.strip())==0 for v in re.search(r'StretchVariants: \[\]int\{([^}]*)\}', fields)[1].split(',')) if 'StretchVariants:' in fields else True
    rows.append((int(code,16), f'Recipe {{code: {code}, sizes: &{array("Sizes", "float64")}, variants: &{array("SizeVariants", "int")}, chars: &{array("SizeChars", "rune")}, parts: &{array("Stretch", "rune")}, width: {(repr(float(hdw[1].split(",")[2])) if hdw else "0.0")}, alias: {scalar("Alias", "0")}, minimum: {float(scalar("Min", "0.0"))}, full: '+('Some(['+', '.join(repr(float(v)) for v in full[1].split(','))+'])' if full else 'None')+' },'))
args.output.write_text('// SPDX-License-Identifier: MPL-2.0\n// Source-derived from mathjax-go v0.1.0 font/data_delimiters_gen.go;\n// MathJax 3.2.2 Apache-2.0. See THIRD_PARTY_NOTICES.md.\n'
    'pub(super) struct Recipe {pub code:u32,pub sizes:&\'static [f64],pub variants:&\'static [usize],pub chars:&\'static [u32],pub parts:&\'static [u32],pub width:f64,pub alias:u32,pub minimum:f64,pub full:Option<[f64;2]>}\n'
    'pub(super) const RECIPES:&[Recipe]=&[\n'+'\n'.join(text for _, text in sorted(rows))+'\n];\n')
subprocess.run(['rustfmt', str(args.output)], check=True)
print(f'generated {len(rows)} vertical recipes')
