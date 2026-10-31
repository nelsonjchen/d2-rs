#!/usr/bin/env python3
"""Controlled remote-asset partial-output comparison against official D2."""
import argparse
import hashlib
import http.server
import json
from pathlib import Path
import subprocess
import shutil
import threading
import time
import re
import gzip
import zlib
import base64
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--rust', type=Path, default=ROOT / 'target/release/d2-rs')
parser.add_argument('--stdout', action='store_true')
parser.add_argument('--stdin', action='store_true')
parser.add_argument('--successful', action='store_true')
parser.add_argument('--local', action='store_true')
parser.add_argument('--dot-path', action='store_true')
parser.add_argument('--directory', action='store_true')
parser.add_argument('--delayed', action='store_true')
parser.add_argument('--slow', action='store_true')
parser.add_argument('--deadline', action='store_true')
parser.add_argument('--encoding', choices=['gzip', 'x-gzip', 'gzip, deflate', 'deflate', 'br', 'gzip, br', 'zstd'])
parser.add_argument('--truncate', action='store_true')
parser.add_argument('--diagnostics', action='store_true')
parser.add_argument('--animate', action='store_true')
parser.add_argument('--boards', action='store_true')
parser.add_argument('--nested', action='store_true')
args = parser.parse_args()
if args.nested:
    args.boards = True
WORK = ROOT / '.reference/asset-partial-output'
WORK.mkdir(parents=True, exist_ok=True)

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        good = self.path == '/good.svg'
        if args.deadline and good:
            time.sleep(62)
        if args.slow and good:
            time.sleep(5.2)
        if args.delayed and self.path == '/missing.svg':
            time.sleep(0.2)
        body = b'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>' if good else b'missing'
        if good and args.encoding in ['gzip', 'x-gzip', 'gzip, deflate']:
            body = gzip.compress(body)
            if args.encoding == 'gzip, deflate':
                body = zlib.compress(body)
        if good and args.encoding == 'deflate':
            body = zlib.compress(body)
        if good and args.encoding in ['br', 'gzip, br']:
            import brotli
            if args.encoding == 'gzip, br':
                body = gzip.compress(body)
            body = brotli.compress(body)
        if good and args.truncate:
            body = body[:-4]
        self.send_response(200 if good else 404)
        if good and args.encoding:
            self.send_header('Content-Encoding', args.encoding)
        self.send_header('Content-Type', 'image/svg+xml' if good else 'text/plain')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass
    def log_message(self, *_):
        pass

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
port = server.server_port
fixture = WORK / 'input.d2'
fixture.write_text(f'a: {{icon: http://127.0.0.1:{port}/good.svg}}\nb: {{icon: http://127.0.0.1:{port}/missing.svg}}\nc: {{icon: http://127.0.0.1:{port}/missing2.svg}}\n')
if args.boards:
    fixture.write_text(f'z\nlayers: {{first: {{a: {{icon: http://127.0.0.1:{port}/good.svg}}}}; second: {{b: {{icon: http://127.0.0.1:{port}/missing.svg}}}}; third: {{c: {{icon: http://127.0.0.1:{port}/good.svg}}}}}}\n')
if args.nested:
    fixture.write_text(f'z\nlayers: {{outer: {{x; layers: {{child: {{b: {{icon: http://127.0.0.1:{port}/missing.svg}}}}}}}}; later: {{c: {{icon: http://127.0.0.1:{port}/good.svg}}}}}}\n')
if args.successful:
    fixture.write_text(f'a: {{icon: http://127.0.0.1:{port}/good.svg}}\n')
if args.local:
    (WORK / 'missing-local.svg').unlink(missing_ok=True)
    fixture.write_text('a: {icon: unused/../missing-local.svg}\n' if args.dot_path else 'a: {icon: missing-local.svg}\n')
if args.directory:
    (WORK / 'directory.svg').mkdir(exist_ok=True)
    fixture.write_text('a: {icon: directory.svg}\n')
runs = {}
def run_binary(name, binary):
    output = WORK / f'{name}.svg'
    output.unlink(missing_ok=True)
    shutil.rmtree(output.with_suffix(""), ignore_errors=True)
    process = subprocess.run([str(binary), '--layout=dagre', *(['--animate-interval=1000'] if args.animate else []), '-' if args.stdin else str(fixture), '-' if args.stdout else str(output)], input=fixture.read_bytes() if args.stdin else None, capture_output=True, timeout=90 if args.deadline else 60)
    svg = process.stdout if args.stdout else (output.read_bytes() if output.exists() else b'')
    stderr = process.stderr.decode(errors='replace').replace(str(port), '<PORT>')
    files = sorted(output.with_suffix('').rglob('*.svg')) if args.boards else []
    if files:
        svg = b'\n'.join(file.read_bytes() for file in files)
    return {'files': [str(file.relative_to(output.with_suffix(''))) for file in files], 'progress_messages': stderr.count('fetching images...'), 'failure_order': re.findall(r'failed to bundle (http://\S+): ', stderr), 'success_messages': stderr.count('success: successfully compiled '), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'exit_code': process.returncode, 'output_written': bool(svg) if args.stdout else (output.exists() or bool(files)), 'bundled_svg_hashes': sorted({hashlib.sha256(base64.b64decode(value)).hexdigest() for value in re.findall(rb'data:image/svg\+xml;base64,([^\"]+)', svg)}), 'successful_asset_bundled': b'data:image/svg+xml;base64,' in svg, 'failed_asset_reference_preserved': b'/missing.svg' in svg, 'stderr': stderr}
try:
    binaries = [('official', ROOT / '.reference/bin/d2-v0.9.0-official'), ('rust', args.rust.resolve())]
    if args.deadline:
        with ThreadPoolExecutor(max_workers=2) as executor:
            jobs = [(name, executor.submit(run_binary, name, binary)) for name, binary in binaries]
            runs = {name: job.result() for name, job in jobs}
    else:
        runs = {name: run_binary(name, binary) for name, binary in binaries}
finally:
    server.shutdown()
    thread.join()
report = {'kind': 'controlled-remote-asset-partial-output', 'deadline': args.deadline, 'directory': args.directory, 'stdin_mode': args.stdin, 'dot_path': args.dot_path, 'local': args.local, 'diagnostics': args.diagnostics, 'truncate': args.truncate, 'slow': args.slow, 'encoding': args.encoding, 'delayed': args.delayed, 'successful': args.successful, 'stdout_mode': args.stdout, 'animated': args.animate, 'multiboard': args.boards, 'nested': args.nested, 'scope': 'controlled output boundaries, embedded asset hashes and message counts; complete stderr compared with only ephemeral port normalized' if args.diagnostics else 'controlled output boundaries, embedded asset hashes and message counts; diagnostic text recorded but not qualified', 'runs': runs, 'passed': all(runs['rust'][key] == runs['official'][key] for key in ['files', 'success_messages', 'exit_code', 'output_written', 'successful_asset_bundled', 'bundled_svg_hashes', 'failed_asset_reference_preserved'] + (['failure_order'] if args.delayed else []) + (['progress_messages'] if args.slow else []) + (['stderr'] if args.diagnostics else []))}
path = ROOT / f"oracle-cache/reports/asset-partial-{'deadline-' if args.deadline else ''}{'directory-' if args.directory else ''}{'stdin-' if args.stdin else ''}{'dot-path-' if args.dot_path else ''}{'local-' if args.local else ''}{'truncated-' if args.truncate else ''}{args.encoding.replace(', ', '-') + '-' if args.encoding else ''}{'slow-' if args.slow else ''}{'delayed-' if args.delayed else ''}{'successful-' if args.successful else ''}{'nested-' if args.nested else ''}{'boards-' if args.boards else ''}{'animated-' if args.animate else ''}{'stdout' if args.stdout else 'output'}-controlled-20261001.json"
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
raise SystemExit(0 if report['passed'] else 1)
