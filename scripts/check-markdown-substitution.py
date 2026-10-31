#!/usr/bin/env python3
"""Compare native Markdown substitutions with the pinned D2 v0.9.0 oracle."""
from __future__ import annotations

import json
import random
import subprocess
from pathlib import Path

from references import ROOT, REFERENCE, MANIFEST, digest, verify_source


def requests() -> list[dict]:
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
    result = [{'Text': text, 'Variables': {'x': 'VALUE', 'a.b': 'NESTED'}, 'Repeats': 2} for text in texts]
    # Keep a reproducible malformed/block-boundary fuzz slice in the checked
    # corpus. These inputs are deliberately small, so parser hangs and lazy
    # block ownership errors fail quickly without depending on a live fuzzer.
    randomizer = random.Random(0xD20F)
    parts = [
        '${x}', '${a.b}', '${missing}', '*', '_', '~', '`', '[', ']', '!',
        '<', '>', '<!--', '-->', '<?', '?>', '<!DOCTYPE', '<![CDATA[', ']]>',
        '#', '##', '#######', '-', '+', '1.', '2)', '|', '---', '___', '***',
        '```', '~~~', '    ', '\t', '\\', '  ', '\r\n', '😀', 'é', '&amp;',
        '<div>', '</div>', 'plain',
    ]
    for _ in range(1000):
        lines = [
            ''.join(randomizer.choice(parts) for _ in range(randomizer.randint(0, 8)))
            for _ in range(randomizer.randint(0, 10))
        ]
        result.append({'Text': '\n'.join(lines), 'Variables': {'x': 'VALUE', 'a.b': 'NESTED'}, 'Repeats': 1})
    result.append({'Text': '${a} ${b}', 'Variables': {'a': '${b}', 'b': 'DONE'}, 'Repeats': 128})
    return result


def main() -> None:
    verify_source()
    payload = json.dumps(requests()).encode()
    oracle = REFERENCE / 'bin/markdown-substitution-oracle-plain'
    if not oracle.is_file():
        raise SystemExit('missing pinned oracle; run scripts/check-markdown-substitution-reference.py')
    native = ROOT / 'target/release/examples/markdown-substitution-oracle'
    subprocess.run([
        'cargo', 'build', '-p', 'd2-rs-text', '--example', 'markdown-substitution-oracle',
        '--release', '--offline',
    ], cwd=ROOT, check=True)
    go_runs = json.loads(subprocess.run([str(oracle)], input=payload, stdout=subprocess.PIPE, check=True).stdout)
    rust_runs = json.loads(subprocess.run([str(native)], input=payload, stdout=subprocess.PIPE, check=True).stdout)
    if len(go_runs) != len(rust_runs):
        raise SystemExit(f'case count mismatch: Go={len(go_runs)} Rust={len(rust_runs)}')
    cases = []
    mismatches = []
    for index, (go_case, rust_case) in enumerate(zip(go_runs, rust_runs)):
        go_outputs = sorted({run['output'] for run in go_case})
        rust_outputs = sorted({run['output'] for run in rust_case})
        chained = index == len(go_runs) - 1
        if chained:
            # Go map traversal is intentionally nondeterministic for this case.
            allowed = {'${b} DONE', 'DONE DONE'}
            matched = bool(set(rust_outputs) <= allowed and set(rust_outputs) & set(go_outputs))
        else:
            matched = len(go_outputs) == 1 and rust_outputs == go_outputs
        case = {
            'index': index, 'text': requests()[index]['Text'],
            'go_outputs': go_outputs, 'rust_outputs': rust_outputs,
            'matched': matched, 'chained_upstream_nondeterminism': chained,
        }
        cases.append(case)
        if not matched:
            mismatches.append(case)
    report = {
        'source_commit': MANIFEST['d2']['commit'],
        'go_toolchain': MANIFEST['d2']['go_toolchain'],
        'oracle_sha256': digest(oracle), 'native_sha256': digest(native),
        'cases': len(cases), 'matched': len(mismatches) == 0,
        'scope': 'Native Markdown replacement outputs compared with the pinned v0.9.0 implementation; chained map-order behavior remains upstream-nondeterministic.',
        'mismatches': mismatches, 'results': cases,
    }
    report_path = ROOT / 'oracle-cache/reports/markdown-substitution-parity-20260921.json'
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + '\n')
    print(f"{len(cases)} cases; {len(mismatches)} mismatches")
    if mismatches:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
