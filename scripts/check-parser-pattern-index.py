#!/usr/bin/env python3
"""Qualify Go/Rust parser behavior when wildcard patterns cross substitutions."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

from references import MANIFEST, REFERENCE, ROOT, digest, verify_oracle, verify_source


GO = REFERENCE / "bin/parser-oracle"
RUST = ROOT / "target/release/examples/parse-oracle"
REPORT = ROOT / "oracle-cache/reports/parser-pattern-index-parity.json"
GO_HARNESS = ROOT / "tools/parser-oracle/main.go"
GO_PATTERN_SITE = b"d2parser/parse.go:1208"
RUST_PATTERN_SITE = b"parser/strings.rs:396"


def cases() -> list[str]:
    prefixes = ["", "a", "x", "ab", "aaa", "é", "😀", "aé", "éa", "a\\*", "a\\n", "a ", "  "]
    tails = ["", "a", "aa", "aaa", "é", "😀", "a*", "é*", "😀*", "a*é", "*", "**", "${y}", "a${y}", "é${y}", "\\n", "\\t", " a"]
    inputs = []
    for prefix in prefixes:
        for tail in tails:
            inputs.extend(
                (
                    f"a: {prefix}*${{x}}{tail}",
                    f"a: {prefix}*${{x}}{tail}*",
                    f"a: {prefix}*${{x}}{tail}${{y}}z*",
                )
            )
    return inputs


def run(binary: Path, source: str) -> subprocess.CompletedProcess[bytes]:
    request = [
        {
            "hex": source.encode().hex(),
            "mode": "public_parse",
            "utf16": False,
            "options": "none",
            "path": "pattern-fuzz.d2",
        }
    ]
    return subprocess.run(
        [str(binary)],
        input=json.dumps(request).encode(),
        capture_output=True,
        timeout=5,
        check=False,
    )


def canonical_digest(value: object) -> str:
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def main() -> int:
    source = verify_source()
    if not GO.is_file() or not RUST.is_file():
        raise SystemExit("build parser-oracle and target/release/examples/parse-oracle first")
    provenance = verify_oracle(GO)
    if provenance.get("harness_sha256") != digest(GO_HARNESS):
        raise SystemExit("stale parser oracle harness; rebuild with scripts/build-parser-oracle.py")

    inputs = cases()
    classifications: list[dict[str, str]] = []
    output_matches = 0
    shared_panics = 0
    for index, source_text in enumerate(inputs):
        go = run(GO, source_text)
        rust = run(RUST, source_text)
        go_panicked = go.returncode != 0
        rust_panicked = rust.returncode != 0

        if go_panicked:
            if GO_PATTERN_SITE not in go.stderr or b"slice bounds out of range" not in go.stderr:
                raise RuntimeError(
                    f"unclassified Go failure in case {index}: {source_text!r}: "
                    f"{go.stderr.decode(errors='replace')[:500]}"
                )
            if not rust_panicked:
                raise RuntimeError(f"Go-only panic in case {index}: {source_text!r}")
            if RUST_PATTERN_SITE not in rust.stderr or b"range start index" not in rust.stderr:
                raise RuntimeError(
                    f"unclassified Rust panic in case {index}: {source_text!r}: "
                    f"{rust.stderr.decode(errors='replace')[:500]}"
                )
            shared_panics += 1
            classifications.append({"input": source_text, "classification": "shared_upstream_panic"})
            continue

        if go.returncode != 0:
            raise RuntimeError(f"Go parser exited {go.returncode} in case {index}: {source_text!r}")
        if rust_panicked:
            raise RuntimeError(
                f"Rust-only panic in case {index}: {source_text!r}: "
                f"{rust.stderr.decode(errors='replace')[:500]}"
            )
        try:
            go_value = json.loads(go.stdout)
            rust_value = json.loads(rust.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"invalid oracle JSON in case {index}: {source_text!r}") from error
        if go_value != rust_value:
            raise RuntimeError(f"parser output mismatch in case {index}: {source_text!r}")
        output_matches += 1
        classifications.append(
            {
                "input": source_text,
                "classification": "exact_output_match",
                "result_sha256": canonical_digest(go_value),
            }
        )

    report = {
        "kind": "d2-v0.9.0-parser-pattern-index-differential",
        "source_commit": MANIFEST["d2"]["commit"],
        "go_toolchain": MANIFEST["d2"]["go_toolchain"],
        "go_oracle_sha256": digest(GO),
        "go_harness_sha256": digest(GO_HARNESS),
        "go_parser_source_sha256": digest(source / "d2parser/parse.go"),
        "rust_binary_sha256": digest(RUST),
        "rust_parser_source_sha256": digest(ROOT / "crates/d2-rs-language/src/parser/strings.rs"),
        "generator_sha256": digest(Path(__file__)),
        "cases_sha256": hashlib.sha256("\n".join(inputs).encode()).hexdigest(),
        "total": len(inputs),
        "exact_output_matches": output_matches,
        "shared_upstream_panics": shared_panics,
        "go_only_panics": 0,
        "rust_only_panics": 0,
        "output_mismatches": 0,
        "panic_classification": (
            "Go v0.9.0 panics at d2parser/parse.go:1208 while slicing the pattern suffix; "
            "Rust panics at parser/strings.rs:396 on the corresponding suffix slice. "
            "These cases are recorded as upstream parser limitations, not successful parses."
        ),
        "cases": classifications,
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(
        json.dumps(
            {key: report[key] for key in ("total", "exact_output_matches", "shared_upstream_panics", "output_mismatches")},
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
