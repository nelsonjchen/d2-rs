#!/usr/bin/env python3
"""Observe pinned D2 Markdown segments and actual replacement traversal.

This is source qualification, not a Rust parity check. The overlay inserts
read-only event capture into the original substitution implementation.
"""
import json
import os
import shutil
import subprocess
from references import ROOT, REFERENCE, MANIFEST, digest, verify_source

source = verify_source()
module = REFERENCE / 'markdown-substitution-oracle-module'
module.mkdir(parents=True, exist_ok=True)
harness = ROOT / 'tools/markdown-substitution-oracle/main.go'
shutil.copy2(harness, module / 'main.go')
shutil.copy2(ROOT / 'tools/oracle/go.sum', module / 'go.sum')
(module / 'go.mod').write_text((ROOT / 'tools/oracle/go.mod').read_text().replace('../../../d2', str(source)))
original = source / 'lib/textmeasure/substitutions.go'
instrumented = original.read_text()
for before, after in [
    ('originalText := string(segment.Value(source))', 'originalText := string(segment.Value(source))\n if oracleMarkdownEvents != nil { *oracleMarkdownEvents = append(*oracleMarkdownEvents, map[string]any{"kind":"segment", "start":segment.Start,"stop":segment.Stop,"text":originalText}) }'),
    ('s = strings.ReplaceAll(s, "${"+k+"}", v)', 'if oracleMarkdownEvents != nil { *oracleMarkdownEvents = append(*oracleMarkdownEvents, map[string]any{"kind":"replace", "key":k,"value":v,"before":s}) }\n s = strings.ReplaceAll(s, "${"+k+"}", v)'),
]:
    assert instrumented.count(before) == 1
    instrumented = instrumented.replace(before, after)
instrumented += '''
var oracleMarkdownEvents *[]map[string]any
func OracleMarkdownSubstitutions(s string, vars map[string]string) (string, []map[string]any) {
 events := make([]map[string]any, 0)
 oracleMarkdownEvents = &events
 defer func(){ oracleMarkdownEvents = nil }()
 return ReplaceSubstitutionsMarkdown(s, vars), events
}
'''
overlay_source = module / 'substitutions.go.txt'
overlay_source.write_text(instrumented)
overlay = module / 'overlay.json'
overlay.write_text(json.dumps({'Replace': {str(original): str(overlay_source)}}))
binary = REFERENCE / 'bin/markdown-substitution-oracle'
subprocess.run(['go', 'build', '-mod=readonly', '-overlay', str(overlay), '-o', str(binary), '.'], cwd=module, check=True,
    env={**os.environ, 'GOTOOLCHAIN': MANIFEST['d2']['go_toolchain'], 'GOMODCACHE': str(REFERENCE / 'go-mod')})
plain_bridge = module / 'plain-bridge.go.txt'
plain_bridge.write_text('''package textmeasure
func OracleMarkdownSubstitutions(s string, vars map[string]string) (string, []map[string]any) {
 return ReplaceSubstitutionsMarkdown(s, vars), nil
}
''')
plain_overlay = module / 'plain-overlay.json'
plain_overlay.write_text(json.dumps({'Replace': {str(source / 'lib/textmeasure/oracle_markdown.go'): str(plain_bridge)}}))
plain_binary = REFERENCE / 'bin/markdown-substitution-oracle-plain'
subprocess.run(['go', 'build', '-mod=readonly', '-overlay', str(plain_overlay), '-o', str(plain_binary), '.'], cwd=module, check=True,
    env={**os.environ, 'GOTOOLCHAIN': MANIFEST['d2']['go_toolchain'], 'GOMODCACHE': str(REFERENCE / 'go-mod')})
texts = [
    '${x}', 'a ${x} b', '` ${x} `', '`` ${x} ` z ``',
    '    ${x}\n', '```\n${x}\n```\n${x}', '~~~lang\n${x}\n~~~',
    '*${x}* **${x}** ~~${x}~~', '[${x}](${x} "${x}")',
    '![${x}](${x})', '<span>${x}</span>', '<div>\n${x}\n</div>',
    '<!-- ${x} -->', '\\${x}', '${x}\n${x}', '${x}  \n${x}',
    '# ${x}\n', '> ${x}\n', '- ${x}\n- `${x}`',
    '| a | b |\n|---|---|\n| ${x} | `${x}` |', '${a.b}',
    '${missing}', '${x_${x}}', '&amp; ${x}', '😀 ${x}',
]
requests = [{'Text': t, 'Variables': {'x': 'VALUE', 'a.b': 'NESTED'}, 'Repeats': 2} for t in texts]
requests += [{'Text': '${a} ${b}', 'Variables': {'a': '${b}', 'b': 'DONE'}, 'Repeats': 128}]
runs = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
plain_runs = json.loads(subprocess.run([str(plain_binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(runs) == len(requests)
assert len(plain_runs) == len(requests)
cases = []
for index, (request, observations, plain) in enumerate(zip(requests, runs, plain_runs)):
    outputs = sorted({r['output'] for r in observations})
    plain_outputs = sorted({r['output'] for r in plain})
    if index < len(texts):
        assert outputs == plain_outputs and len(outputs) == 1
    # The chained case cannot be compared run-for-run across Go map seeds.
    # Check its permitted source-derived outcomes, retaining every raw run.
    else:
        assert set(outputs + plain_outputs) <= {'${b} DONE', 'DONE DONE'}
    cases.append({'request': request, 'outputs': outputs, 'plain_outputs': plain_outputs, 'runs': observations, 'plain_runs': plain})
report = {
    'source_commit': MANIFEST['d2']['commit'], 'go_toolchain': MANIFEST['d2']['go_toolchain'],
    'goldmark': 'v1.8.5', 'source_sha256': {p: digest(source / p) for p in ['lib/textmeasure/substitutions.go', 'lib/textmeasure/markdown.go']},
    'harness_sha256': digest(harness), 'overlay_sha256': digest(overlay_source), 'binary_sha256': digest(binary),
    'plain_binary_sha256': digest(plain_binary), 'plain_bridge_sha256': digest(plain_bridge),
    'scope': 'Source observations only; no native parity claim. Original function and instrumented build agree on all 25 independent-replacement cases. Chained replacements are checked against source-derived possible outcomes, not run-for-run or distribution parity. Actual map traversal and raw original runs retained; alternatives are upstream nondeterminism, not normalized matches.',
    'cases': cases,
}
report_path = ROOT / 'oracle-cache/reports/markdown-substitution-reference.json'
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, indent=2) + '\n')
verify_source()
print(f'{len(cases)} source cases; {sum(len(c["outputs"]) > 1 for c in cases)} with observed output nondeterminism')
