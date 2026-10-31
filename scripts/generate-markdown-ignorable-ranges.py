#!/usr/bin/env python3
"""Generate exhaustive native Source Markdown control classification ranges."""
import json, sys
from pathlib import Path
rows=json.loads(Path(sys.argv[1]).read_text())
assert rows and all(0 <= a <= b <= 0x10ffff for a,b in rows)
assert all(rows[i-1][1] < rows[i][0] for i in range(1,len(rows)))
p=Path(sys.argv[2]);existing=p.read_text();header=existing[:existing.index("pub(super) const RANGES:")]
p.write_text(header+"pub(super) const RANGES: &[(u32,u32)] = &[\n"+"".join(f"    (0x{a:x},0x{b:x}),\n" for a,b in rows)+"];\n")
