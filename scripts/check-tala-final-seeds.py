#!/usr/bin/env -S uv run python
# SPDX-License-Identifier: MPL-2.0
"""Require exact TALA final geometry for every selected seed and repeat."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from references import ROOT


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixtures', nargs='+')
    parser.add_argument('--go', type=Path, required=True)
    parser.add_argument('--rust', type=Path, default=ROOT / 'target/diagnostic/d2-rs')
    parser.add_argument('--seeds', type=int, nargs='+', default=[1, 2, 3, 17, 42, 2147483647])
    parser.add_argument('--repeats', type=int, default=2)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    if args.repeats < 2:
        parser.error('at least two repeats are required for determinism qualification')
    environment = os.environ.copy()
    environment['D2_TRACE_LAYOUT_FINAL'] = '1'
    results = []
    first_runs = {}
    nondeterminism = []
    for seed in args.seeds:
        for repeat in range(args.repeats):
            child_report = args.report.with_name(f'{args.report.stem}-seed-{seed}-repeat-{repeat}.json')
            completed = subprocess.run([
                sys.executable, str(ROOT / 'scripts/check-grid-trace.py'),
                '--require-final-state', '--layout', 'tala', '--seed', str(seed),
                '--go', str(args.go.resolve()), '--rust', str(args.rust.resolve()),
                '--report', str(child_report.resolve()), *args.fixtures,
            ], cwd=ROOT, env=environment, capture_output=True, text=True, timeout=600)
            if not child_report.is_file():
                raise RuntimeError(completed.stderr or completed.stdout)
            report = json.loads(child_report.read_text())
            results.append({'seed': seed, 'repeat': repeat, 'report': str(child_report),
                            'passed': completed.returncode == 0 and report['passed']})
            for case in report['results']:
                for binary, run in case['runs'].items():
                    key = (seed, case['fixture'], binary)
                    # Final snapshots retain every recorded box/route float bit.
                    evidence = {'svg_sha256': run['svg_sha256'],
                                'final_records': [record for record in run['trace_records']
                                                  if 'stage=layout-final ' in record]}
                    if key in first_runs and evidence != first_runs[key]:
                        nondeterminism.append({'seed': seed, 'repeat': repeat,
                                               'fixture': case['fixture'], 'binary': binary,
                                               'first': first_runs[key], 'current': evidence})
                    first_runs.setdefault(key, evidence)
    result = {'scope': 'Explicit-seed final geometry and repeated-run SVG/snapshot determinism; detailed execution traces excluded.',
              'passed': all(run['passed'] for run in results) and not nondeterminism,
              'seeds': args.seeds, 'repeats': args.repeats, 'results': results,
              'nondeterminism': nondeterminism, 'full_goal_complete': False}
    args.report.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'passed': result['passed'], 'cases': len(results) * len(args.fixtures),
                      'nondeterminism': len(nondeterminism), 'report': str(args.report)}))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
