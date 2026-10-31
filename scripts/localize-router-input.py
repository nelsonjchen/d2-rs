#!/usr/bin/env python3
"""Localize diagnostic router ingress differences without discarding ordering.

Accepts the saved Go and Rust stderr files from check-grid-trace.py. This is
an input localization tool, not a final-output or operation parity gate.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def callbacks(path: Path, prefix: str = "ROUTER_INPUT") -> list[list[str]]:
    result: list[list[str]] = []
    active: list[str] | None = None
    for line in path.read_text().splitlines():
        if line.startswith(f"{prefix} begin "):
            if active is not None:
                raise ValueError(f"nested router callback in {path}")
            active = [line]
        elif line == f"{prefix} end":
            if active is None:
                raise ValueError(f"unpaired router end in {path}")
            active.append(line)
            result.append(active)
            active = None
        elif active is not None and line.startswith(("ROUTER_", f"{prefix} ")):
            active.append(line)
    if active is not None:
        raise ValueError(f"unfinished router callback in {path}")
    return result


def key(line: str) -> tuple[str, ...]:
    words = line.split()
    if words[1] in ("begin", "end"):
        return tuple(words[:2])
    return tuple([words[0]] + [w for w in words[1:] if w.startswith(("node=", "edge=", "endpoint="))])


def compare(go: list[str], rust: list[str]) -> dict:
    go_keys, rust_keys = list(map(key, go)), list(map(key, rust))
    if len(set(go_keys)) != len(go_keys) or len(set(rust_keys)) != len(rust_keys):
        raise ValueError("duplicate diagnostic identity; cannot localize losslessly")
    go_map, rust_map = dict(zip(go_keys, go)), dict(zip(rust_keys, rust))
    return {
        "ordered_equal": go == rust,
        "inventory_order_equal": go_keys == rust_keys,
        "missing_from_rust": [go_map[k] for k in go_keys if k not in rust_map],
        "extra_in_rust": [rust_map[k] for k in rust_keys if k not in go_map],
        "property_differences": [
            {"key": k, "go": go_map[k], "rust": rust_map[k]}
            for k in go_keys if k in rust_map and go_map[k] != rust_map[k]
        ],
        "go_ordered": go,
        "rust_ordered": rust,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("go", type=Path)
    parser.add_argument("rust", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prefix", choices=("ROUTER_INPUT", "ROUTER_STAGE", "LABEL_PLACEMENT", "ROUTER_COSTS"), default="ROUTER_INPUT")
    args = parser.parse_args()
    go, rust = callbacks(args.go, args.prefix), callbacks(args.rust, args.prefix)
    result = {
        "prefix": args.prefix,
        "go_callback_count": len(go),
        "rust_callback_count": len(rust),
        "callbacks": [compare(g, r) for g, r in zip(go, rust)],
        "unpaired_go_callbacks": go[len(rust):],
        "unpaired_rust_callbacks": rust[len(go):],
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"go_callbacks": len(go), "rust_callbacks": len(rust), "differences": [len(c["property_differences"]) for c in result["callbacks"]]}))


if __name__ == "__main__":
    main()
