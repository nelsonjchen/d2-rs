#!/usr/bin/env python3
"""Expose original compiler primitives using an additive Go build overlay."""
import json, os, shutil, subprocess, sys
from references import ROOT, REFERENCE, MANIFEST, verify_source, digest
source = verify_source()
module = REFERENCE / 'field-oracle-module'
module.mkdir(parents=True, exist_ok=True)
shutil.copy2(ROOT / 'tools/field-oracle/main.go', module / 'main.go')
shutil.copy2(ROOT / 'tools/oracle/go.sum', module / 'go.sum')
(module / 'go.mod').write_text((ROOT / 'tools/oracle/go.mod').read_text().replace('../../../d2', str(source)))
bridge = ROOT / 'tools/field-oracle/bridge.go'
overlay = module / 'overlay.json'
replacements={str(source / 'd2ir/oracle_fields.go'): str(bridge)}
tracing='--trace' in sys.argv
if tracing:
    original=(source/'d2ir/compile.go').read_text()
    marker='func (c *compiler) compileKey(refctx *RefContext) {'
    assert original.count(marker)==1
    instrumented=original.replace(marker,marker+'\n oracleTraceEnabled=true\n oracleCompilerEvent(c,"enter",refctx)\n defer oracleCompilerEvent(c,"exit",refctx)')
    # The initializer also identifies an instrumented empty compilation.
    instrumented+='\nfunc init(){oracleTraceEnabled=true}\n'
    (module/'compile-trace.txt').write_text(instrumented)
    replacements[str(source/'d2ir/compile.go')]=str(module/'compile-trace.txt')
overlay.write_text(json.dumps({'Replace':replacements}))
output = REFERENCE / ('bin/field-oracle-trace' if tracing else 'bin/field-oracle')
subprocess.run(['go', 'build', '-mod=readonly', '-overlay', str(overlay), '-o', str(output), '.'], cwd=module, check=True,
    env={**os.environ, 'GOTOOLCHAIN': MANIFEST['d2']['go_toolchain'], 'GOMODCACHE': str(REFERENCE / 'go-mod/pkg/mod')})
verify_source()
output.with_suffix('.provenance.json').write_text(json.dumps(dict(source_commit=MANIFEST['d2']['commit'],
    source_sha256={p: digest(source / p) for p in ['d2ir/d2ir.go','d2ir/compile.go','d2ir/pattern.go']},
    harness_sha256=digest(module / 'main.go'), bridge_sha256=digest(bridge), binary_sha256=digest(output),
    go_toolchain=MANIFEST['d2']['go_toolchain'], trace=tracing, instrumented_sha256=digest(module/'compile-trace.txt') if tracing else None, source_change='Additive overlay; trace build also instruments key entry/exit through a compile.go overlay; checkout unchanged'), indent=2)+'\n')
print(output)
