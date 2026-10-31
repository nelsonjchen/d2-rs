#!/usr/bin/env python3
"""Audit the physical correspondence between pinned Go TALA and d2-tala-rs.

The report is deliberately structural.  It does not infer behavioral parity
from matching filenames; it records package coverage and collapsed modules so
that source and trace investigations can find the corresponding Rust area.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys


GO_FUNCTION = re.compile(r"\bfunc(?:\s*\([^)]*\))?\s+([A-Za-z_]\w*)\s*\(")
RUST_FUNCTION = re.compile(
    r"\b(?:pub(?:\s*\([^)]*\))?\s+)?fn\s+([A-Za-z_]\w*)\s*\("
)


def package_files(root: Path, package: str, suffix: str) -> list[str]:
    package_root = root / package if package else root
    if not package_root.is_dir():
        return []
    # TALA keeps most implementation files in one or more nested modules.
    # A shallow glob silently undercounted those files and made mapped modules
    # look missing, so the structural audit must follow the full package tree.
    return sorted(
        path.relative_to(package_root).as_posix()
        for path in package_root.rglob(f"*{suffix}")
        if path.is_file()
    )


def package_names(root: Path, suffix: str | None = None) -> list[str]:
    return sorted(
        path.name
        for path in root.iterdir()
        if path.is_dir()
        and not path.name.startswith(".")
        and (suffix is None or any(path.rglob(f"*{suffix}")))
    )


def classify(go_files: list[str], rust_files: list[str]) -> str:
    if not rust_files:
        return "missing"
    if len(rust_files) < len(go_files):
        return "collapsed"
    return "mapped"


def symbols(root: Path, files: list[str], pattern: re.Pattern[str]) -> list[str]:
    """Return stable function names found in a package's non-test sources."""
    names: set[str] = set()
    for filename in files:
        try:
            text = (root / filename).read_text(encoding="utf-8")
        except UnicodeDecodeError:
            text = (root / filename).read_text(encoding="utf-8", errors="replace")
        names.update(pattern.findall(text))
    return sorted(names)


def symbol_key(name: str) -> str:
    """Compare Go CamelCase and Rust snake_case spellings by identity."""
    return re.sub(r"[^a-z0-9]", "", name.lower())


def build_report(go_root: Path, rust_root: Path) -> dict[str, object]:
    go_packages = package_names(go_root / "internal", ".go")
    rust_packages = package_names(rust_root, ".rs")
    packages: list[dict[str, object]] = []
    for package in go_packages:
        go_package_root = go_root / "internal" / package
        go_files = package_files(go_root / "internal", package, ".go")
        rust_files = package_files(rust_root, package, ".rs")
        go_tests = [name for name in go_files if name.endswith("_test.go")]
        go_sources = [name for name in go_files if not name.endswith("_test.go")]
        go_symbol_names = symbols(go_package_root, go_sources, GO_FUNCTION)
        rust_package_root = rust_root / package
        rust_symbol_names = symbols(rust_package_root, rust_files, RUST_FUNCTION)
        go_symbol_keys = {symbol_key(name) for name in go_symbol_names}
        rust_symbol_keys = {symbol_key(name) for name in rust_symbol_names}
        packages.append(
            {
                "name": package,
                "upstream_path": go_package_root.relative_to(go_root).as_posix(),
                "upstream_source_files": go_sources,
                "upstream_test_files": go_tests,
                "rust_path": (rust_root / package).relative_to(rust_root.parent).as_posix(),
                "rust_files": rust_files,
                "classification": classify(go_sources, rust_files),
                "upstream_symbols": go_symbol_names,
                "rust_symbols": rust_symbol_names,
                "missing_symbols": sorted(
                    name for name in go_symbol_names if symbol_key(name) not in rust_symbol_keys
                ),
                "extra_symbols": sorted(
                    name for name in rust_symbol_names if symbol_key(name) not in go_symbol_keys
                ),
            }
        )

    rust_only = sorted(set(rust_packages) - set(go_packages))
    return {
        "schema": 1,
        "authority": "D2 v0.9.0 d2layouts/d2talalayout",
        "upstream_commit": "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
        "upstream_go_version": "go1.27.0",
        "upstream_root": "<d2>/d2layouts/d2talalayout",
        "rust_root": "<d2-rs>/crates/d2-tala-rs/src/internal",
        "upstream_area_count": len(go_packages),
        "rust_area_count": len(rust_packages),
        "rust_only_packages": rust_only,
        "packages": packages,
        "totals": {
            "upstream_source_files": sum(
                len(package["upstream_source_files"]) for package in packages
            ),
            "upstream_test_files": sum(
                len(package["upstream_test_files"]) for package in packages
            ),
            "rust_files": sum(len(package["rust_files"]) for package in packages),
            "upstream_symbols": sum(len(package["upstream_symbols"]) for package in packages),
            "rust_symbols": sum(len(package["rust_symbols"]) for package in packages),
            "missing_symbols": sum(len(package["missing_symbols"]) for package in packages),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--upstream",
        type=Path,
        default=Path.home() / "code/d2/d2layouts/d2talalayout",
        help="pinned Go TALA directory",
    )
    parser.add_argument(
        "--rust",
        type=Path,
        default=Path("crates/d2-tala-rs/src/internal"),
        help="Rust TALA internal source directory",
    )
    parser.add_argument("--output", type=Path, help="write JSON to this path")
    parser.add_argument("--check", action="store_true", help="fail if --output differs")
    args = parser.parse_args()

    if not args.upstream.is_dir():
        parser.error(f"upstream directory does not exist: {args.upstream}")
    if not args.rust.is_dir():
        parser.error(f"Rust directory does not exist: {args.rust}")

    report = build_report(args.upstream.resolve(), args.rust.resolve())
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        if args.check:
            try:
                existing = args.output.read_text(encoding="utf-8")
            except FileNotFoundError:
                print(f"missing report: {args.output}", file=sys.stderr)
                return 1
            if existing != encoded:
                print(f"stale report: {args.output}", file=sys.stderr)
                return 1
        else:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
