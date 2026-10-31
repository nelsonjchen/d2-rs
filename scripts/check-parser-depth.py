#!/usr/bin/env python3
"""Compare bounded nested-input depth against pinned Go and release Rust parsers."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, digest, verify_oracle, verify_source


GO = REFERENCE / "bin/parser-oracle"
RUST = ROOT / "target/release/examples/parse-oracle"
REPORT = ROOT / "oracle-cache/reports/parser-depth-parity.json"
DEPTHS = (1, 2, 4, 8, 16, 32, 64, 128, 256, 512)
TIMEOUT_SECONDS = 5


def cases() -> list[tuple[str, str]]:
    rows = []
    for depth in DEPTHS:
        rows.extend(
            (
                (f"array-{depth}", "a: " + "[" * depth + "x" + "]" * depth),
                (f"map-{depth}", "a: " + "{b: " * depth + "x" + "}" * depth),
                (f"substitution-{depth}", "a: " + "${b." * depth + "x" + "}" * depth),
            )
        )
    return rows


def run(binary: Path, source: str) -> dict:
    request = {
        "hex": source.encode().hex(),
        "mode": "public_parse",
        "utf16": False,
        "options": "default",
        "path": "depth.d2",
    }
    try:
        process = subprocess.run(
            [str(binary)],
            input=json.dumps([request]).encode(),
            capture_output=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {"exit_code": 124, "timeout": True}

    observation = {"exit_code": process.returncode}
    if process.returncode == 0:
        value = json.loads(process.stdout)
        if len(value) != 1:
            raise RuntimeError(f"{binary} returned {len(value)} parse results")
        canonical = json.dumps(
            value[0], ensure_ascii=False, sort_keys=True, separators=(",", ":")
        ).encode()
        observation["result_sha256"] = hashlib.sha256(canonical).hexdigest()
        observation["result_bytes"] = len(process.stdout)
        observation["result"] = value[0]
    else:
        observation["stderr"] = process.stderr.decode(errors="replace")[:1000]
    return observation


def main() -> int:
    source = verify_source()
    go_record = verify_oracle(GO)
    if not RUST.is_file():
        raise SystemExit("build target/release/examples/parse-oracle first")

    results = []
    for name, source_text in cases():
        go = run(GO, source_text)
        rust = run(RUST, source_text)
        matched = (
            go.get("exit_code") == rust.get("exit_code") == 0
            and go.get("result") == rust.get("result")
        )
        row = {
            "case": name,
            "input_sha256": hashlib.sha256(source_text.encode()).hexdigest(),
            "input_bytes": len(source_text.encode()),
            "go": {key: value for key, value in go.items() if key != "result"},
            "rust": {key: value for key, value in rust.items() if key != "result"},
            "matched": matched,
        }
        if not matched:
            row["go_result"] = go.get("result")
            row["rust_result"] = rust.get("result")
        results.append(row)
        print(f"{name}: {'PASS' if row['matched'] else 'FAIL'}", flush=True)
        if not row["matched"]:
            REPORT.parent.mkdir(parents=True, exist_ok=True)
            REPORT.write_text(
                json.dumps(
                    {
                        "kind": "d2-v0.9.0-parser-nesting-depth-parity",
                        "source_commit": go_record["source_commit"],
                        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
                        "timeout_seconds": TIMEOUT_SECONDS,
                        "results": results,
                    },
                    ensure_ascii=False,
                    indent=2,
                )
                + "\n"
            )
            return 1

    report = {
        "kind": "d2-v0.9.0-parser-nesting-depth-parity",
        "source_commit": go_record["source_commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "go_oracle_sha256": digest(GO),
        "go_harness_sha256": go_record["harness_sha256"],
        "go_parser_sha256": digest(source / "d2parser/parse.go"),
        "generator_sha256": digest(Path(__file__)),
        "rust_binary_sha256": digest(RUST),
        "rust_parser_source_sha256": {
            path.relative_to(ROOT).as_posix(): digest(path)
            for path in sorted(
                [ROOT / "crates/d2-rs-language/src/parse.rs"]
                + list((ROOT / "crates/d2-rs-language/src/parser").glob("*.rs"))
            )
        },
        "timeout_seconds": TIMEOUT_SECONDS,
        "depths": list(DEPTHS),
        "shapes": ["array", "map", "substitution"],
        "maximum_depth": max(DEPTHS),
        "total": len(results),
        "cases_sha256": hashlib.sha256(
            "\n".join(source_text for _, source_text in cases()).encode()
        ).hexdigest(),
        "matched": sum(row["matched"] for row in results),
        "timeouts": sum(
            row[engine].get("timeout", False)
            for row in results
            for engine in ("go", "rust")
        ),
        "scope": "Bounded recursive syntax depth checks through public Parse; five-second per-process timeout. This does not claim an explicit input-size or recursion limit.",
        "results": results,
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("maximum_depth", "total", "matched", "timeouts")}, indent=2))
    return int(report["matched"] != report["total"] or report["timeouts"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())
