#!/usr/bin/env python3
"""Score d2-rs command-line behaviour against the pinned Go D2 CLI.

The upstream e2etests-cli suite observes most errors through an in-process
xmain.TestState, so it cannot see what a user sees: mainFatal's printed
lines, usage trailers, warnings and success logs. This tier runs both
executables on the same invocations in fresh directories and compares exit
code, stdout and stderr exactly. Two normalizations only: elapsed durations in
success lines, and the program path (Rust runs under Go's basename so help
text and argv[0] spell the same). Stdin carries a small diagram.

Image-asset policy cases need no external network: a loopback HTTP server
(substituted for {PORT} in FILES) serves one SVG and a 404, and the other
remote references are loopback, localhost, a private literal and a
metadata hostname that the public-only policy rejects before dialing.
Leading NAME=value tokens of a case set that environment variable.

An explicit --go <clock-oracle> --epoch <Unix-seconds> profile injects the
same UTC clock for warnings and PDFs without changing compared bytes. Build
that oracle with scripts/build-cli-clock-oracle.py. Ordinary runs remove any
inherited SOURCE_DATE_EPOCH.

usage: cli-diff-scoreboard.py <report.json> [rust-binary]
"""
import argparse
import hashlib
import http.server
import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
GO = references.GO_ORACLE

FILES = {
    "in.d2": "x -> y\n", "uf.d2": "x->y\n", "bad.d2": "x: {\n", "ml.d2": "a\nlayers: {x: {b}}\n",
    # Remote and local image assets (lib/netpolicy, lib/imageasset, lib/localfile).
    "srv.d2": "a: {icon: http://127.0.0.1:{PORT}/ok.svg}\n",
    "srv404.d2": "a: {icon: http://127.0.0.1:{PORT}/missing.svg}\n",
    "lo.d2": "a: {icon: http://127.0.0.1:9/x.svg}\n",
    "lh.d2": "a: {icon: http://localhost:9/x.svg}\n",
    "pv.d2": "a: {icon: http://10.255.255.1:9/x.svg}\n",
    "md.d2": "a: {icon: http://metadata.google.internal/x.svg}\n",
    "np.d2": "a: {icon: //example.invalid/x.svg}\n",
    "mf.d2": "a: {icon: missing.svg}\n",
    "dir.d2": "a: {icon: dir.svg}\n",
    # Mixed: which reference fails first is a Go worker race, so only
    # --bundle=false (local only) and raster (remote unavailable) are scored.
    # Which reference of a mixed board fails first is a Go worker race, so
    # only its local-only (--bundle=false) and raster forms are scored.
    "ok.d2": "a: {icon: ok.svg}\nb: {icon: http://127.0.0.1:9/x.svg}\n",
}
SVG_ASSET = '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>'

CASES = [
    "", "--help", "-h", "version", "--version", "-v", "version extra", "themes", "themes extra",
    "layout", "layout dagre", "layout elk", "layout tala", "layout nope", "layout dagre extra",
    "layout dagre --dagre-nodesep 50 in.d2",
    "fmt", "fmt -", "fmt in.d2", "fmt --check in.d2", "fmt uf.d2", "fmt --check uf.d2", "fmt bad.d2",
    "fmt missing.d2", "validate", "validate -", "validate in.d2", "validate bad.d2", "validate missing.d2",
    "play", "--bogus in.d2", "-Z in.d2", "--theme", "-t", "--pad=abc in.d2", "--scale=x in.d2",
    "--sketch=maybe in.d2 out.svg", "--center=2 in.d2 out.svg", "-s=false in.d2 o.svg",
    "--font-regular=missing.ttf in.d2 out.svg", "in.d2 a.svg b.svg", "in.d2 out.ppt",
    "--animate-interval=10 in.d2 out.png", "--animate-interval=-1 in.d2 o.svg",
    "--theme=12345 in.d2 out.svg", "-t 999 in.d2", "--dark-theme=999 in.d2 out.svg",
    "--dark-theme=200 in.d2 o.png", "--stdout-format=bogus in.d2 -", "--stdout-format=txt in.d2 -",
    "--stdout-format=TXT in.d2 -", "-l elk --stdout-format=txt in.d2 -", "--layout=nope in.d2 out.svg",
    "--layout=ELK in.d2 o.svg", "--layout=tala-rs in.d2 o.svg", "-w -", "-w --target=x in.d2",
    "--target=nope in.d2 out.svg", "--target=x in.d2 out.png", "missing.d2 out.svg", "bad.d2 out.svg",
    "in.d2 out.xyz", "in.d2 dir/", "in.d2 -", "--pad=-5 in.d2 o.svg", "--scale=0 in.d2 o.svg",
    "--timeout=0 in.d2 o.svg", "--browser=0 in.d2 o.svg", "-- -weird.d2", "in.d2 o.pdf",
    "in.d2 o.pptx", "in.d2 o.gif", "in.d2 o.png", "in.d2 o.txt", "--stdout-format=pdf in.d2 -",
    "ml.d2 o.svg", "ml.d2 o.png", "ml.d2 o.pdf", "ml.d2 o.gif", "--animate-interval=100 ml.d2 o.svg",
    # Network policy: loopback/localhost/private/metadata destinations.
    "srv.d2 o.svg", "srv.d2 o.png", "--allow-private-network srv.d2 o.svg",
    "--allow-private-network srv.d2 o.png", "--allow-private-network=true srv.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK=1 srv.d2 o.svg", "D2_ALLOW_PRIVATE_NETWORK=true srv.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK=0 srv.d2 o.svg", "D2_ALLOW_PRIVATE_NETWORK=false srv.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK= srv.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK=true --allow-private-network=false srv.d2 o.svg",
    "--allow-private-network srv404.d2 o.svg", "--allow-private-network srv404.d2 o.png",
    "srv404.d2 o.svg", "lo.d2 o.svg", "lo.d2 o.png", "--allow-private-network lo.d2 o.svg",
    "lh.d2 o.svg", "lh.d2 o.png", "pv.d2 o.svg", "pv.d2 o.png", "md.d2 o.svg",
    "--bundle=false srv.d2 o.svg", "--bundle=false lo.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK=bad in.d2 o.svg", "D2_ALLOW_PRIVATE_NETWORK=TRUE in.d2 o.svg",
    "D2_ALLOW_PRIVATE_NETWORK=yes version", "D2_ALLOW_PRIVATE_NETWORK=bad --help",
    "D2_ALLOW_PRIVATE_NETWORK=bad fmt in.d2", "--allow-private-network=maybe in.d2 o.svg",
    # Local files and source classification.
    "mf.d2 o.svg", "mf.d2 o.png", "--bundle=false mf.d2 o.svg", "dir.d2 o.svg", "dir.d2 o.png",
    "np.d2 o.svg", "np.d2 o.png", "--bundle=false ok.d2 o.svg", "ok.d2 o.png",
]
DURATION = re.compile(r" in [0-9.]+(?:ns|µs|us|ms|s|m[0-9.]+s|h[0-9hms.]+)$", re.MULTILINE)


class AssetHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/ok.svg":
            body = SVG_ASSET.encode()
            self.send_response(200)
            self.send_header("Content-Type", "image/svg+xml")
        else:
            body = b""
            self.send_response(404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


def run(binary, case, directory, epoch=None):
    words = case.split()
    environment = {key: value for key, value in os.environ.items() if key != "D2_ALLOW_PRIVATE_NETWORK"}
    # Clock profiles are explicit; ordinary runs never inherit one silently.
    environment.pop("SOURCE_DATE_EPOCH", None)
    if epoch is not None:
        environment["SOURCE_DATE_EPOCH"] = str(epoch)
    while words and re.fullmatch(r"[A-Z0-9_]+=.*", words[0]):
        key, value = words.pop(0).split("=", 1)
        environment[key] = value
    try:
        result = subprocess.run([str(binary), *words], check=False, cwd=directory, capture_output=True,
                                input=b"a -> b\n", timeout=60, env=environment)
    except subprocess.TimeoutExpired:
        return {"exit": "timeout"}
    program = str(binary).encode()
    return {
        "exit": result.returncode,
        "stdout": result.stdout.replace(program, b"PROG").decode("utf-8", "backslashreplace"),
        "stderr": DURATION.sub(" in T", result.stderr.replace(program, b"PROG").decode("utf-8", "backslashreplace")),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("rust", type=Path, nargs="?", default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--go", type=Path, default=GO)
    parser.add_argument("--epoch", type=int, help="explicit shared UTC log/PDF clock; requires a clock-instrumented trace oracle")
    args = parser.parse_args()
    if args.epoch is not None and not 0 <= args.epoch <= 253402300799:
        parser.error("--epoch must be in 0..=253402300799")
    if args.epoch is not None and args.go.resolve() == GO.resolve():
        parser.error("--epoch requires --go with the instrumented trace oracle")
    output, rust, oracle = args.output, args.rust.resolve(), args.go.resolve()
    work = Path(tempfile.mkdtemp(prefix="cli-diff-"))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), AssetHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = str(server.server_address[1])
    try:
        # Same basename as Go: argv[0] appears in help and usage text.
        program = work / "bin" / oracle.name
        program.parent.mkdir()
        shutil.copy2(rust, program)
        rows = []
        for case in CASES:
            results = {}
            for name, binary in (("go", oracle), ("rust", program)):
                directory = work / name
                shutil.rmtree(directory, ignore_errors=True)
                directory.mkdir()
                for file, text in FILES.items():
                    (directory / file).write_text(text.replace("{PORT}", port))
                (directory / "ok.svg").write_text(SVG_ASSET)
                (directory / "dir.svg").mkdir()
                results[name] = run(binary, case, directory, args.epoch)
                # The scratch directory name differs per engine; compare it as one token.
                for key in ("stdout", "stderr"):
                    if key in results[name]:
                        results[name][key] = results[name][key].replace(str(directory.resolve()), "DIR")
            row = {"args": case, "outcome": "match" if results["go"] == results["rust"] else "mismatch"}
            if row["outcome"] == "mismatch":
                row.update(results)
            rows.append(row)
    finally:
        server.shutdown()
        shutil.rmtree(work, ignore_errors=True)
    summary = {outcome: sum(row["outcome"] == outcome for row in rows) for outcome in ("match", "mismatch")}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({"go": str(oracle), "rust": str(rust),
                                 "go_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest(),
                                 "rust_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
                                 "clock": {"epoch_seconds": args.epoch, "timezone": "UTC"} if args.epoch is not None else None,
                                 "summary": summary, "rows": rows}, indent=1) + "\n")
    print(summary)
    for row in rows:
        if row["outcome"] != "match":
            print("mismatch:", row["args"])


if __name__ == "__main__":
    main()
