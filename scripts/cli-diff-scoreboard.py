#!/usr/bin/env python3
"""Score d2-rs command-line behavior against the pinned Go D2 CLI.

The upstream e2etests-cli suite observes most errors through an in-process
xmain.TestState, so it cannot see what a user sees: mainFatal's printed
lines, usage trailers, warnings and success logs. This tier runs both
executables on the same invocations in fresh directories and compares exit
code, stdout and stderr exactly. Three normalizations only: elapsed durations
in success lines; the program path (Rust runs under Go's basename so help
text and argv[0] spell the same); and each binary's own reported version,
where it is the whole of stdout (`version`, `--version`) or the help header,
since d2-rs reports its release where Go reports D2's. Output metadata such
as data-d2-version is compared unchanged. Stdin carries a small diagram.

Image-asset policy cases need no external network: a loopback HTTP server
(substituted for {PORT} in FILES) serves one SVG and a 404, and the other
remote references are loopback, localhost, a private literal and a
metadata hostname that the public-only policy rejects before dialing.
Leading NAME=value tokens of a case set that environment variable.

An explicit --go <clock-oracle> --epoch <Unix-seconds> profile injects the
same UTC clock for warnings and PDFs without changing compared bytes. Build
that oracle with scripts/build-cli-clock-oracle.py. Ordinary runs remove any
inherited SOURCE_DATE_EPOCH.

A mismatching invocation is rerun by the seedable-map Go oracle (also under
Go's basename) under map seeds 1-16 and classified by scripts/mapseed.py
(--go-mapseed, --mapseed-seeds, --record-mapseed): mapseed-match and
known-mapseed pass; go-nondeterministic-no-rust-match and
mapseed-oracle-missing fail. With --epoch the seeded oracle must be a
clock+mapseed build (build-cli-clock-oracle.py --mapseed) given as --go-mapseed.

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
import sys
import tempfile
import threading
from pathlib import Path

import mapseed
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


def normalize_version(stdout, version):
    """Replace a binary's own version where the CLI reports it, nowhere else."""
    if stdout == version + "\n":
        return "VERSION\n"
    # Help starts "<argv[0] basename> <version>", then "Usage:".
    return re.sub(r"\A(\S+) " + re.escape(version) + r"\nUsage:\n", r"\1 VERSION\nUsage:\n", stdout, count=1)


def run(binary, case, directory, epoch=None, map_seed=None):
    words = case.split()
    environment = {key: value for key, value in os.environ.items() if key not in ("D2_ALLOW_PRIVATE_NETWORK", mapseed.ENV)}
    # Clock profiles are explicit; ordinary runs never inherit one silently.
    environment.pop("SOURCE_DATE_EPOCH", None)
    if epoch is not None:
        environment["SOURCE_DATE_EPOCH"] = str(epoch)
    if map_seed is not None:
        environment[mapseed.ENV] = str(map_seed)
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
    mapseed.add_arguments(parser, "seedable-map Go oracle; with --epoch a clock+mapseed build is required "
                                  "(default without --epoch: the --go path plus -mapseed)")
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
        versions = {name: subprocess.run([str(binary), "--version"], check=True, capture_output=True,
                                         text=True, timeout=60).stdout.strip()
                    for name, binary in (("go", oracle), ("rust", program))}
        seeded_source = (args.go_mapseed.resolve() if args.go_mapseed else
                          None if args.epoch is not None else mapseed.default_oracle(oracle))
        seeded_program = None
        if seeded_source is not None and seeded_source.is_file():
            # Under Go's basename too: argv[0] appears in help and usage text.
            seeded_program = work / "seeded-bin" / oracle.name
            seeded_program.parent.mkdir()
            shutil.copy2(seeded_source, seeded_program)
            versions["go-mapseed"] = subprocess.run([str(seeded_program), "--version"], check=True, capture_output=True,
                                                    text=True, timeout=60).stdout.strip()

        def capture(name, binary, case, map_seed=None):
            directory = work / name
            shutil.rmtree(directory, ignore_errors=True)
            directory.mkdir()
            for file, text in FILES.items():
                (directory / file).write_text(text.replace("{PORT}", port))
            (directory / "ok.svg").write_text(SVG_ASSET)
            (directory / "dir.svg").mkdir()
            result = run(binary, case, directory, args.epoch, map_seed)
            if "stdout" in result:
                result["stdout"] = normalize_version(result["stdout"], versions[name if map_seed is None else "go-mapseed"])
            # The scratch directory name differs per engine; compare it as one token.
            for key in ("stdout", "stderr"):
                if key in result:
                    result[key] = result[key].replace(str(directory.resolve()), "DIR")
            return result

        def probe(binary):
            """Map seed unset: a render, a PDF (dates follow the PDF clock) and a
            fmt --check warning on an unformatted file (its timestamp follows
            the log clock). Stdin is already formatted, so the file is needed.
            Without --epoch both clocks are the wall clock, so only the render runs."""
            results = []
            clocked = ("--stdout-format=pdf - -", "fmt --check probe.d2") if args.epoch is not None else ()
            for case in ("--layout=dagre - -", *clocked):
                directory = work / "probe"
                shutil.rmtree(directory, ignore_errors=True)
                directory.mkdir()
                (directory / "probe.d2").write_text("a->b\n")
                result = run(binary, case, directory, args.epoch)
                for key in ("stdout", "stderr"):
                    if key in result:
                        result[key] = result[key].replace(str(directory.resolve()), "DIR")
                results.append(result)
            return results

        mapseed_oracle = mapseed.Oracle(seeded_program or seeded_source, oracle, probe,
                                        mapseed.CLOCK_BUILD_COMMAND if args.epoch is not None else mapseed.BUILD_COMMAND)
        registry, recorded = mapseed.Registry(args.mapseed_registry), []
        rows = []
        for case in CASES:
            results = {name: capture(name, binary, case) for name, binary in (("go", oracle), ("rust", program))}
            known = registry.known(mapseed.identity(case, ["cli-diff-scoreboard"]))
            row = {"args": case, "outcome": "match" if results["go"] == results["rust"] else "mismatch"}
            if (row["outcome"] == "mismatch" or known) and results["go"].get("exit") != "timeout":
                resolution = mapseed.resolve(
                    [results["go"]], [results["rust"]],
                    lambda seed: capture("go-mapseed", mapseed_oracle.path, case, seed), mapseed_oracle,
                    args.mapseed_seeds, known, usable=lambda result: result.get("exit") != "timeout")
                row["outcome"] = resolution.classification
                row["mapseed"] = resolution.record()
                if resolution.go_variants > 1:
                    recorded.append((mapseed.identity(case, ["cli-diff-scoreboard"]), case, ["cli-diff-scoreboard"],
                                     f"cli-diff-scoreboard.py case {case!r}", resolution))
            if row["outcome"] not in mapseed.PASSING:
                row.update(results)
            rows.append(row)
    finally:
        server.shutdown()
        shutil.rmtree(work, ignore_errors=True)
    summary = mapseed.counts(dict(row, classification=row["outcome"]) for row in rows)
    summary.setdefault("match", 0)
    summary.setdefault("mismatch", 0)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({"go": str(oracle), "rust": str(rust),
                                 "go_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest(),
                                 "rust_sha256": hashlib.sha256(rust.read_bytes()).hexdigest(),
                                 "clock": {"epoch_seconds": args.epoch, "timezone": "UTC"} if args.epoch is not None else None,
                                 "summary": summary, "rows": rows}, indent=1) + "\n")
    print(summary)
    for row in rows:
        if row["outcome"] not in mapseed.PASSING:
            print(f"{row['outcome']}:", row["args"])
        elif row["outcome"] != "match":
            print(f"{row['outcome']} (seeds {','.join(map(str, row['mapseed']['mapseed_seeds']))}):", row["args"])
    if summary.get(mapseed.ORACLE_MISSING):
        print(f"error: {mapseed_oracle.message()}", file=sys.stderr)
    if args.record_mapseed:
        added = mapseed.record_cases(registry, recorded, "cli-diff-scoreboard")
        print(f"recorded {added} new map-order-dependent inputs in {registry.path}")
    return int(any(row["outcome"] not in mapseed.PASSING for row in rows))


if __name__ == "__main__":
    main()
