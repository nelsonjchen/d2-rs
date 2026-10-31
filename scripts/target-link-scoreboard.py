#!/usr/bin/env python3
"""Compare complete CLI SVGs for board links after --target selection.

Source contracts: d2cli/main.go clears selected descendants before resolveLinks
and relink. Inputs include the L15 negative witness and mixed node/edge links.
This finite source corpus adds no upstream golden claims or SVG normalization.
The shared CLI runner enforces 1 GiB total / 128 MiB file / 20 GiB free limits.
"""

from __future__ import annotations

import argparse
import collections
import json
import xml.etree.ElementTree as ET
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
SPEC = spec_from_file_location("iso_cli", ROOT / "scripts/isometric-cli-scoreboard.py")
CLI = module_from_spec(SPEC)
SPEC.loader.exec_module(CLI)

SOURCES = {
    "internal-link-tooltip": "a: {link: layers.detail; tooltip: Open detail}\nlayers: {detail: {b}}\n",
    "node-edge-links": """a: {link: layers.detail}
b: {link: root}
a -> b: {link: layers.detail; tooltip: Detail}
layers: {
  detail: {
    c: {link: layers.child}
    d: {link: root}
    c -> d: {link: layers.child; tooltip: Child}
    layers: {child: {e: {link: root.layers.sibling}}}
  }
  sibling: {s: {link: root.layers.detail}}
}
""",
}


def cases():
    rows = []
    for source_name, source in SOURCES.items():
        for engine in ("dagre", "elk", "tala"):
            for label, target in (
                ("default", None),
                ("root", ""),
                ("all", "*"),
                ("named", "layers.detail"),
                ("short", "detail"),
                ("subtree", "layers.detail.*"),
            ):
                for stdout in (False, True):
                    args = ["--pad=0", f"--layout={engine}", "--tala-seeds=1"]
                    if target is not None:
                        args.append(f"--target={target}")
                    if stdout:
                        args.extend(["--stdout-format=svg", "-"])
                    rows.append(
                        {
                            "name": f"{source_name}-{engine}-{label}-{'stdout' if stdout else 'file'}",
                            "source": source,
                            "args": args,
                            "env": {},
                            "suffix": "svg",
                            "fail": stdout and label in ("default", "all"),
                        }
                    )
    return rows


def validate(directory, result, *, expect_error=False):
    # Include empty and unexpected files in the inventory, too.
    result["outputs"] = {
        str(path.relative_to(directory)): CLI.digest(path)
        for path in directory.rglob("*")
        if path.is_file() and path.name not in ("stdout.log", "stderr.log")
    }
    if expect_error:
        return
    # A named recursive target can turn '-' into a literal output directory
    # in Go (planBoardOutput prepends its board name). Validate those SVG
    # payloads even when their file names have no extension.
    svgs = [
        directory / name
        for name in result["outputs"]
        if not name.endswith(".json")
        and not (name == "stdout.svg" and not (directory / name).stat().st_size)
    ]
    if not svgs:
        raise ValueError("missing SVG inventory")
    for path in svgs:
        if ET.fromstring(path.read_bytes()).tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError(f"invalid SVG root: {path}")
    for manifest in directory.rglob(CLI.RASTER.BOARD_OUTPUT_MANIFEST):
        files = {
            str(path.relative_to(manifest.parent)): path
            for path in manifest.parent.rglob("*")
            if path.is_file() and path != manifest
        }
        CLI.RASTER.board_output_manifest_contract(manifest.parent, manifest, files)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    references.verify_source()
    capture = args.capture.resolve()
    capture.mkdir(parents=True, exist_ok=False)
    rows = []
    for case in cases():
        work = capture / case["name"]
        work.mkdir()
        source = work / "source.d2"
        source.write_text(case["source"])
        results = {}
        for name, binary in (
            ("go", references.GO_ORACLE),
            ("rust", args.rust.resolve()),
        ):
            directory = work / name
            result = CLI.run(binary, case, directory, source, capture)
            try:
                validate(directory, result, expect_error=case["fail"])
            except (ValueError, OSError, ET.ParseError) as error:
                result["failure"] = f"invalid-artifact: {error}"
            results[name] = result
        outcome = CLI.compare(
            case, results["go"], results["rust"], work / "go", work / "rust"
        )
        rows.append(dict(**case, **results, outcome=outcome))
    report = {
        "pin": references.SOURCE_COMMIT,
        "capture": str(capture),
        "go": str(references.GO_ORACLE),
        "go_sha256": CLI.digest(references.GO_ORACLE),
        "rust": str(args.rust.resolve()),
        "rust_sha256": CLI.digest(args.rust),
        "budgets": {"total": 1 << 30, "file": 128 << 20, "free": 20 << 30},
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
