#!/usr/bin/env python3
"""Build private parser-production oracle without modifying the reference checkout."""
import json
import os
import shutil
import subprocess
from references import ROOT, REFERENCE, MANIFEST, verify_source, digest

source = verify_source()
module = REFERENCE / "parser-oracle-module"
module.mkdir(parents=True, exist_ok=True)
original = source / "d2parser/parse.go"
text = original.read_text()
assert text.startswith("package d2parser\n")
(module / "parse.go").write_text(text.replace("package d2parser\n", "package main\n", 1))
shutil.copy2(ROOT / "tools/parser-oracle/main.go", module / "main.go")
shutil.copy2(ROOT / "tools/oracle/go.sum", module / "go.sum")
(module / "go.mod").write_text((ROOT / "tools/oracle/go.mod").read_text().replace("../../../d2", str(source)))
output = REFERENCE / "bin/parser-oracle"
subprocess.run(["go", "build", "-mod=readonly", "-o", str(output), "."], cwd=module, check=True,
    env={**os.environ, "GOTOOLCHAIN": MANIFEST["d2"]["go_toolchain"], "GOMODCACHE": str(REFERENCE / "go-mod/pkg/mod")})
verify_source()
output.with_suffix(".provenance.json").write_text(json.dumps({
    "source_commit": MANIFEST["d2"]["commit"], "source_sha256": digest(original),
    "harness_sha256": digest(module / "main.go"), "binary_sha256": digest(output),
    "go_toolchain": MANIFEST["d2"]["go_toolchain"], "source_change": "package name only",
}, indent=2) + "\n")
print(output)
