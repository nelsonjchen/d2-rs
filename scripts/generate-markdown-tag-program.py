#!/usr/bin/env python3
"""Generate the two pinned tag programs from the original Go regexps."""
import json, subprocess
from references import ROOT, REFERENCE, digest, verify_source
verify_source()
binary=REFERENCE/'bin/markdown-link-oracle'
# markdown-link-oracle binary the committed tag_program.rs was generated from (formerly
# recorded in the retired docs/markdown-link-parity.json; see docs/RETIRED-EVIDENCE.md).
assert digest(binary)=='184c27065d71aeb06489ffaa014717d478b539fe446974b4c88d1a9f26b9a77f'
programs=json.loads(subprocess.check_output([str(binary)],input=json.dumps([{'Kind':'tagprogram','Source':''},{'Kind':'tagprogram','Source':'01'}]).encode()))
out=['// SPDX-License-Identifier: MPL-2.0 AND MIT','// Copyright (c) 2019 Yusuke Inuzuka','// Generated Goldmark v1.8.5 tag regexps using Go 1.27.0 regexp/syntax.','use super::tag_matcher::Inst;']
for name,entry in zip(['OPEN','CLOSE'],programs):
    assert entry['onepass'] is False
    program=entry['program']
    assert program['Start']==1 and program['NumCap']==4
    out.append('pub(super) static '+name+': &[Inst] = &[')
    for v in program['Inst']:
        assert v['Op'] in [0,2,3,4,5,7,8]
        assert v['Op']!=3 or v['Arg']==4
        runes=v['Rune'] or []
        out.append('    Inst { op: '+str(v['Op'])+', out: '+str(v['Out'])+', arg: '+str(v['Arg'])+', runes: &['+', '.join(map(str,runes))+'] },')
    out.append('];')
(ROOT/'crates/d2-rs-text/src/markdown/tag_program.rs').write_text('\n'.join(out)+'\n')
report_path = ROOT / 'oracle-cache/reports/markdown-tag-programs.json'
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps({'programs':programs,'oracle_sha256':digest(binary),'source_sha256':{name:digest(REFERENCE/'go-mod/golang.org/toolchain@v0.0.1-go1.27.0.darwin-arm64/src'/name) for name in ['regexp/exec.go','regexp/regexp.go','regexp/syntax/compile.go','regexp/syntax/prog.go']}},indent=2)+'\n')
