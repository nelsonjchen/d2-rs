#!/usr/bin/env python3
"""Inventory every master-added Go Test function against v0.9.0.

Native observation means a qualified scoreboard row names the original test.
Passing a whole-board corpus does not imply coverage of unnamed inline tests.
This read-only audit does not execute or qualify any previously uncovered API.
"""

import argparse
import collections
import json
import re
import subprocess
from pathlib import Path

import references

ROOT = references.ROOT
FUNCTION = re.compile(r"^func (Test\w+)\(\w+ \*testing\.T\)", re.MULTILINE)


def inventory():
    source = references.verify_source()
    paths = subprocess.check_output(
        ["git", "-C", str(source), "ls-files", "--", "*_test.go"], text=True
    ).splitlines()
    old_names = collections.defaultdict(set)
    current = []
    changed_existing = []
    for name in paths:
        path = Path(name)
        text = (source / path).read_text()
        old = subprocess.run(
            ["git", "-C", str(source), "show", f"v0.9.0:{name}"],
            capture_output=True,
            text=True,
            check=False,
        )
        old_names[str(path.parent)].update(FUNCTION.findall(old.stdout))
        for match in FUNCTION.finditer(text):
            row = {
                "path": name,
                "package": str(path.parent),
                "test": match[1],
                "line": text[: match.start()].count("\n") + 1,
            }
            current.append(row)
            if match[1] in FUNCTION.findall(old.stdout) and old.stdout != text:
                changed_existing.append(row)
    added = [r for r in current if r["test"] not in old_names[r["package"]]]
    assert len({(r["package"], r["test"]) for r in added}) == len(added)
    return added, changed_existing, current


def observations(reports):
    observed = collections.defaultdict(list)
    for report in reports:
        document = json.loads(report.read_text())
        pin = document.get("source_commit", document.get("pin"))
        if pin and pin != references.SOURCE_COMMIT:
            continue
        for row in document.get("rows", []):
            if row.get("outcome") not in {
                "match",
                "source-match",
                "error-match",
                "assertion-match",
            }:
                continue
            name = row.get("test", row.get("case", row.get("name", "")))
            if not isinstance(name, str):
                continue
            name = name.split("/")[0].split("#")[0]
            if re.fullmatch(r"Test\w+", name):
                # Explicit package attribution disambiguates identical Test
                # names captured by different native API scoreboards.
                package = document.get("package")
                if not isinstance(package, str):
                    package = None
                if report.name == "language-master.json":
                    package = "d2compiler"
                elif report.name.startswith("language-master-ir"):
                    package = "d2ir"
                observed[name].append(
                    {
                        "report": str(report.relative_to(ROOT)),
                        "package": package,
                        "outcome": row["outcome"],
                    }
                )
    return observed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--extra-report", action="append", type=Path, default=[])
    args = parser.parse_args()
    added, changed, current = inventory()
    reports = sorted((ROOT / ".reference/scoreboard-master-6a7b4837").glob("*.json"))
    reports += [p.resolve() for p in args.extra_report]
    observed = observations(reports)
    duplicates = collections.Counter(row["test"] for row in added)
    for row in added:
        # A package-ambiguous name is never credited automatically.
        evidence = [
            entry
            for entry in observed.get(row["test"], [])
            if entry["package"] == row["package"]
            or (entry["package"] is None and duplicates[row["test"]] == 1)
        ]
        evidence = list({(e["report"], e["outcome"]): e for e in evidence}.values())
        row["observation_reports"] = evidence
        row["coverage"] = (
            "named-native-observation" if evidence else "no-named-native-observation"
        )
    uncovered = [r for r in added if not r["observation_reports"]]
    result = {
        "source_commit": references.SOURCE_COMMIT,
        "comparison_tag": "v0.9.0",
        "added_functions": len(added),
        "named_observed_functions": len(added) - len(uncovered),
        "uncovered_functions": len(uncovered),
        "uncovered_by_package": dict(
            collections.Counter(r["package"] for r in uncovered)
        ),
        "d2js_wasm": [r for r in added if r["package"].startswith("d2js/")],
        "d2lsp": {
            "current_functions": [r for r in current if r["package"] == "d2lsp"],
            "added_functions": [r for r in added if r["package"] == "d2lsp"],
            "existing_functions_in_changed_files": [
                r for r in changed if r["package"] == "d2lsp"
            ],
        },
        "rows": added,
        "notes": [
            "Names absent from the same v0.9.0 package are master-added; TestMain, benchmarks and examples are not Test functions.",
            "Named observation is inventory coverage only, not proof of every assertion or full API parity.",
            "Finite CLI witnesses and whole-board tiers do not credit unnamed original Test functions.",
            "Changed pre-existing functions are outside the added-function count; d2lsp is checked explicitly.",
        ],
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                k: v
                for k, v in result.items()
                if k.endswith("functions") or k == "uncovered_by_package"
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
