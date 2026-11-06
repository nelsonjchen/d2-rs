#!/usr/bin/env python3
"""Compare pinned Chroma code-block SVGs with the native Rust lexer engine.

Run after building the diagnostic CLI. Samples are source inputs, including
short excerpts of Chroma's MIT-licensed lexer tests; no oracle output is stored
in the tree. Every vendored XML definition must have a sample. Outputs and the
JSON report go into oracle-cache/reports/chroma (1 GiB total, 128 MiB/file).
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

import references

ROOT = references.ROOT
ASSETS = ROOT / "crates/d2-rs-text/assets/chroma"


def block(language: str, source: str) -> str:
    fence = "|" * (1 + max((len(m[0]) for m in re.finditer(r"\|+", source)), default=0))
    return f"x: {fence}{language}\n" + "\n".join("  " + line for line in source.splitlines()) + f"\n{fence}\n"


def compare(row, index, args):
    directory = args.output / f"{index:03d}"
    directory.mkdir(parents=True, exist_ok=True)
    source = directory / "input.d2"
    source.write_text(block(row["language"], row["source"]))
    result = {"language": row["language"], "lexer": row["lexer"], "status": "match"}
    outputs = []
    for name, binary in (("go", args.go), ("rust", args.rust)):
        output = directory / f"{name}.svg"
        output.unlink(missing_ok=True)
        command = [str(binary.resolve()), "--layout=dagre", "--omit-version", str(source), str(output)]
        try:
            run = subprocess.run(command, capture_output=True, timeout=args.timeout, check=False)
        except subprocess.TimeoutExpired:
            result[name] = {"timeout": True}
            outputs.append(None)
            continue
        result[name] = {"exit": run.returncode, "stderr": run.stderr.decode(errors="replace")[-2000:]}
        if output.is_file() and output.stat().st_size > 128 * 1024**2:
            raise RuntimeError(f"output exceeds 128 MiB: {output}")
        outputs.append(output.read_bytes() if output.is_file() else None)
    if any(result[impl].get("exit") != 0 for impl in ("go", "rust")):
        result["status"] = "process-failure"
    elif outputs[0] is None or outputs[1] is None or not outputs[0] or not outputs[1]:
        result["status"] = "missing-output"
    elif outputs[0] != outputs[1]:
        result["status"] = "mismatch"
        go, rust = outputs
        offset = next((i for i, (a, b) in enumerate(zip(go, rust)) if a != b), min(len(go), len(rust)))
        result["offset"] = offset
        result["difference"] = {name: data[max(0, offset - 80):offset + 180].decode(errors="replace")
                                for name, data in zip(("go", "rust"), outputs)}
    return result


def provenance(args):
    version = subprocess.check_output([str(args.go.resolve()), "--version"], text=True).strip()
    metadata = subprocess.check_output(["go", "version", "-m", str(args.go.resolve())], text=True)
    chroma_version = references.MANIFEST["dependencies"]["github.com/alecthomas/chroma/v2"]
    expected = ["vcs.revision=" + references.SOURCE_COMMIT, "vcs.modified=false",
                "GOARCH=" + references.host_name().split("-")[-1],
                "\tdep\tgithub.com/alecthomas/chroma/v2\t" + chroma_version + "\t"]
    if version != "v0.9.0" or any(item not in metadata for item in expected):
        raise RuntimeError("Go oracle must be the pristine, host-architecture pin, stamped v0.9.0, with pinned Chroma")
    upstream = references.ROOT / references.MANIFEST["d2"]["checkout"]
    go_head = subprocess.check_output(["git", "-C", str(upstream), "rev-parse", "HEAD"], text=True).strip()
    if go_head != references.SOURCE_COMMIT:
        raise RuntimeError("Go checkout is not at the manifest pin")
    return {
        "pin": references.SOURCE_COMMIT, "go_head": go_head, "go_version": version,
        "go_sha256": references.digest(args.go), "chroma_version": chroma_version,
        "rust_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "rust_version": subprocess.check_output([str(args.rust.resolve()), "--version"], text=True).strip(),
        "rust_sha256": references.digest(args.rust),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust", type=Path, default=ROOT / "target/diagnostic/d2-rs")
    parser.add_argument("--go", type=Path, default=references.GO_ORACLE)
    parser.add_argument("--output", type=Path, default=references.REPORTS / "chroma")
    parser.add_argument("--go-tokens", type=Path, help="tools/chroma-code-oracle executable (optional exact token check)")
    parser.add_argument("--rust-tokens", type=Path, default=ROOT / "target/debug/examples/code-token-oracle")
    parser.add_argument("--languages", default="", help="optional comma-separated names or aliases")
    parser.add_argument("--jobs", type=int, default=5)
    parser.add_argument("--timeout", type=int, default=60)
    args = parser.parse_args()
    stamp = provenance(args)
    samples = json.loads((ASSETS / "samples.json").read_text())
    definitions = set()
    for path in [*(ASSETS / "lexers").glob("*.xml"), *(ASSETS / "custom").glob("*.xml")]:
        # Configs have no repeated attributes; Go's repeated state attributes
        # elsewhere in these inputs are deliberately preserved in the source.
        xml = path.read_text()
        config = ET.fromstring(xml[xml.index("<config>"):xml.index("</config>") + len("</config>")])
        definitions.add(config.findtext("name"))
    if not definitions or not definitions.issubset({s["lexer"] for s in samples}):
        raise RuntimeError("incomplete XML lexer sample inventory")
    if args.languages:
        languages = set(args.languages.split(","))
        samples = [row for row in samples if row["language"] in languages or row["lexer"] in languages]
        if not samples:
            raise RuntimeError("no samples selected")
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    with ThreadPoolExecutor(max_workers=args.jobs) as workers:
        for result in workers.map(lambda item: compare(item[1], item[0], args), enumerate(samples)):
            rows.append(result)
            if result["status"] != "match":
                print(f"{result['language']}: {result['status']}", flush=True)
    if sum(p.stat().st_size for p in args.output.rglob("*") if p.is_file()) > 1024**3:
        raise RuntimeError("capture exceeds 1 GiB output budget")
    token_failures = []
    if args.go_tokens:
        data = json.dumps(samples)
        token_outputs = []
        for binary in (args.go_tokens, args.rust_tokens):
            run = subprocess.run([str(binary.resolve())], input=data, text=True, capture_output=True,
                                 check=True, timeout=300)
            token_outputs.append(json.loads(run.stdout))
        go_tokens, rust_tokens = token_outputs[0]["cases"], token_outputs[1]
        if len(go_tokens) != len(samples) or len(rust_tokens) != len(samples):
            raise RuntimeError("incomplete token output inventory")
        for sample, go, rust in zip(samples, go_tokens, rust_tokens):
            if go["tokens"] != rust["tokens"]:
                token_failures.append(sample["language"])
        print(f"{len(samples) - len(token_failures)}/{len(samples)} token streams match")
    report = {"provenance": stamp, "token_failures": token_failures, "rows": rows, "match": sum(row["status"] == "match" for row in rows), "total": len(rows)}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"{report['match']}/{report['total']} SVGs match")
    return int(report["match"] != report["total"] or bool(token_failures))


if __name__ == "__main__":
    sys.exit(main())
