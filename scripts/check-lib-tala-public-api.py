#!/usr/bin/env python3
"""Public TALA API oracle fixture generator (tools/fixtures/langlibelk-lib.toml).

usage: check-lib-tala-public-api.py <family>

Builds tools/tala-public-api-oracle against the runner's patched upstream
checkout ($D2RS_UPSTREAM), replays the family's requests from
scripts/tala_public_api_cases.py and writes
crates/d2-rs-lib/tests/fixtures/<family>-source-v090.json (the historical
logical path; the content is a capture at the manifest pin) as
{"source_commit", "cases"}. Run through
`uv run --no-project python scripts/regen-fixtures.py --only 'lib-tala-*'`;
`cargo test -p d2-rs-lib --features oracle-fixtures` performs the native
comparison (docs/FIXTURES.md).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from fixture_recipe import go_env, output, workdir
from references import MANIFEST, ROOT
import tala_public_api_cases as cases

# Families whose rows record effective_font_sizes, and whose requests carry
# raw JSON text instead of a decoded graph (as in the original captures).
FONTS = {'input-tables', 'missing-geometry', 'geometry-conversion', 'public-records', 'near-keys', 'raw-graph'}
RAW = {'raw-graph', 'json-syntax', 'graph-identity'}


def loads(text: str):
    # Go writes negative zero as `-0`; keep it a float so its sign survives.
    return json.loads(text, parse_int=lambda t: -0.0 if t == '-0' else int(t))


def build() -> Path:
    upstream = os.environ.get('D2RS_UPSTREAM')
    if not upstream:
        sys.exit('D2RS_UPSTREAM is unset: the recipe must set upstream = true')
    module = workdir() / 'tala-public-api-oracle'
    module.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / 'tools/tala-public-api-oracle/main.go', module / 'main.go')
    # Require exactly the pinned module graph: upstream's own go.mod/go.sum,
    # with upstream itself replaced by the patched checkout.
    go_mod, count = re.subn(r'(?m)^module github\.com/d2lang/d2$', 'module d2rs.local/tala-public-api-oracle',
                            (Path(upstream) / 'go.mod').read_text())
    assert count == 1, 'upstream go.mod has no module github.com/d2lang/d2 line'
    go_mod += ('\nrequire github.com/d2lang/d2 v0.0.0-00010101000000-000000000000\n'
               f'\nreplace github.com/d2lang/d2 => {upstream}\n')
    (module / 'go.mod').write_text(go_mod)
    shutil.copy2(Path(upstream) / 'go.sum', module / 'go.sum')
    binary = workdir() / 'bin' / 'tala-public-api-oracle'
    subprocess.run(['go', 'build', '-mod=readonly', '-o', str(binary), '.'], cwd=module, env=go_env(), check=True)
    return binary


def run(binary: Path, requests, *flags: str) -> str:
    return subprocess.run([str(binary), *flags], input=json.dumps(requests).encode(), stdout=subprocess.PIPE,
                          check=True, timeout=1500).stdout.decode()


def main() -> None:
    family = sys.argv[1]
    generate = cases.FAMILIES[family]
    binary = build()
    if family == 'near-keys':
        requests = generate(run(binary, cases.near_key_values(), '-key-values'))
    else:
        requests = generate()
    flags = (['-fonts'] if family in FONTS else []) + (['-raw'] if family in RAW else [])
    rows = loads(run(binary, requests, *flags))
    assert len(rows) == len(requests), (len(rows), len(requests))
    for request, row in zip(requests, rows):
        assert row['input']['name'] == request['name'], (row['input']['name'], request['name'])
    path = output(f'crates/d2-rs-lib/tests/fixtures/{family}-source-v090.json')
    path.write_text(json.dumps({'source_commit': MANIFEST['d2']['commit'], 'cases': rows}, indent=2) + '\n')
    print(f"{family}: {len(rows)} cases, {sum(row.get('error') == '' for row in rows)} successes, "
          f"{sum('panic' in row for row in rows)} panics")


if __name__ == '__main__':
    main()
