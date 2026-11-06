#!/usr/bin/env python3
"""Score complete upstream parser/IR/compiler goldens and editor text separately.

Build: cargo build --profile oracle -p d2-rs-editor --example language-scoreboard
Dump: scripts/dump-language-scoreboard.py (env-gated pinned Go test hooks).
No source range, reference, ordering, null/empty, or diagnostic normalization.
Every retained golden is inventoried; orphan goldens and missing native APIs
are explicit non-passing outcomes. Go must reproduce a golden before Rust
can receive a golden-match. Editor text never counts as complete graph parity.
"""
from __future__ import annotations
import argparse
import collections
import concurrent.futures
import hashlib
import json
from pathlib import Path
import subprocess
from references import verify_source
import references

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT.parent / "d2"
PACKAGES = ("d2parser", "d2ir", "d2compiler", "d2oracle")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def same_json(expected, actual):
    # Python considers True == 1. JSON booleans are a separate type.
    if isinstance(expected, bool) or isinstance(actual, bool):
        return type(expected) is type(actual) and expected == actual
    if isinstance(expected, dict) and isinstance(actual, dict):
        return expected.keys() == actual.keys() and all(same_json(expected[key], actual[key]) for key in expected)
    if isinstance(expected, list) and isinstance(actual, list):
        return len(expected) == len(actual) and all(same_json(left, right) for left, right in zip(expected, actual))
    return expected == actual


def differences(expected, actual, path="", limit=12):
    if same_json(expected, actual):
        return []
    if isinstance(expected, dict) and isinstance(actual, dict):
        result = []
        for key in sorted(expected.keys() | actual.keys()):
            name = path + "." + key
            if key not in expected or key not in actual:
                result.append({"path": name, "expected": expected.get(key, "<absent>"), "actual": actual.get(key, "<absent>")})
            else:
                result.extend(differences(expected[key], actual[key], name, limit))
            if len(result) >= limit:
                return result[:limit]
        return result
    if isinstance(expected, list) and isinstance(actual, list) and len(expected) == len(actual):
        result = []
        for index, (left, right) in enumerate(zip(expected, actual)):
            result.extend(differences(left, right, path + f"[{index}]", limit))
            if len(result) >= limit:
                return result[:limit]
        return result
    # Keep summaries compact without changing the bytes used for comparison.
    def brief(value):
        text = json.dumps(value, ensure_ascii=True)
        return value if len(text) <= 240 else text[:240] + "..."
    return [{"path": path, "expected": brief(expected), "actual": brief(actual)}]


def import_dump(directory, output):
    cases = []
    for package in PACKAGES:
        for path in sorted((directory / package).glob("*.json")):
            row = json.loads(path.read_text())
            golden = UPSTREAM / "testdata" / package / (row["name"] + ".exp.json")
            row["golden"] = str(golden.relative_to(UPSTREAM)) if golden.is_file() else None
            row["golden_sha256"] = digest(golden) if golden.is_file() else None
            row["go_reproduces_golden"] = same_json(json.loads(golden.read_text()), row["go"]) if golden.is_file() else None
            if row["request"].get("result_kind") == "text":
                row["go_reproduces_text"] = (
                    row["error"] == row["request"]["expected_error"]
                    and (bool(row["error"]) or row["go"] == row["request"]["expected_text"])
                )
                # UpdateImport's current test checks a string. Older graph
                # goldens with the same name have no current graph producer.
                row["go_reproduces_golden"] = None
            # Expected objects stay in the pristine tree; no duplicate graph
            # captures or full source snapshots enter the retained case dump.
            del row["go"]
            cases.append(row)
    assert cases, "empty language dump"
    output.parent.mkdir(parents=True, exist_ok=True)
    assert not output.exists(), "case dump is immutable; choose a fresh path"
    output.write_text(json.dumps({"upstream_commit": references.SOURCE_COMMIT, "cases": cases}, indent=1) + "\n")
    return cases


def score(binary, case, timeout):
    row = {key: case[key] for key in ["package", "name", "golden", "go_reproduces_golden"]}
    request = dict(case["request"], package=case["package"])
    try:
        process = subprocess.run([str(binary.resolve())], input=json.dumps(request).encode(), capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return dict(row, outcome="timeout")
    if process.returncode != 0 or not process.stdout:
        return dict(row, outcome="native-failed", stderr=process.stderr.decode(errors="replace")[-1500:])
    try:
        actual = json.loads(process.stdout)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return dict(row, outcome="invalid-output")
    if case["request"].get("result_kind") == "text":
        if row["golden"] is not None:
            assert digest(UPSTREAM / row["golden"]) == case["golden_sha256"]
        row.update(outcome="unbound-golden" if row["golden"] else "no-golden",
                   reason="current pinned test compares text only; no current graph golden binding")
        row["text_outcome"] = text_outcome(case, actual, case["go_reproduces_text"])
        if row["text_outcome"] == "text-mismatch":
            row["text_difference"] = differences(case["request"]["expected_text"], actual.get("text"))
        if row["text_outcome"] == "error-mismatch":
            row.update(expected_error=case["error"], actual_error=actual.get("error"))
        return row
    if row["golden"] is None:
        return dict(row, outcome="no-golden")
    golden = UPSTREAM / row["golden"]
    assert digest(golden) == case["golden_sha256"], f"golden changed: {golden}"
    expected = json.loads(golden.read_text())
    if not row["go_reproduces_golden"]:
        row["outcome"] = "stale-golden"
    elif "unsupported" in actual:
        row.update(outcome="unsupported", reason=actual["unsupported"])
    elif actual.get("graph_unsupported"):
        row.update(outcome="unsupported-graph", reason="native editor exposes source, not complete graph")
    elif "value" not in actual:
        row["outcome"] = "missing-output"
    elif same_json(actual["value"], expected):
        row["outcome"] = "golden-match"
    else:
        row.update(outcome="golden-mismatch", differences=differences(expected, actual["value"]))
    if case["package"] == "d2oracle":
        if "unsupported" in actual:
            row["text_outcome"] = "unsupported"
        elif case["error"]:
            row["text_outcome"] = "error-match" if actual.get("error") == case["error"] else "error-mismatch"
            if row["text_outcome"] != "error-match":
                row.update(expected_error=case["error"], actual_error=actual.get("error"))
        else:
            row["text_outcome"] = "text-match" if actual.get("text") == case["request"]["expected_text"] else "text-mismatch"
            if row["text_outcome"] != "text-match":
                row["text_difference"] = differences(case["request"]["expected_text"], actual.get("text"))
    return row


def text_outcome(case, actual, go_reproduces):
    if not go_reproduces:
        return "stale-text"
    if "unsupported" in actual:
        return "unsupported"
    if case["error"]:
        return "error-match" if actual.get("error") == case["error"] else "error-mismatch"
    return "text-match" if actual.get("text") == case["request"]["expected_text"] and not actual.get("error") else "text-mismatch"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dump", type=Path, help="import a fresh upstream hook dump")
    parser.add_argument("--cases", type=Path, default=ROOT / ".reference/language-scoreboard-20261022/cases-v2.json")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/oracle/examples/language-scoreboard")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=20)
    args = parser.parse_args()
    verify_source()
    cases = import_dump(args.dump, args.cases) if args.dump else json.loads(args.cases.read_text())["cases"]
    assert cases, "empty language case inventory"
    keys = [(case["package"], case["name"]) for case in cases]
    assert len(keys) == len(set(keys)), "duplicate language case keys"
    for package in PACKAGES:
        assert any(case["package"] == package for case in cases), f"missing case package: {package}"
        assert any((UPSTREAM / "testdata" / package).rglob("*.exp.json")), f"empty golden inventory: {package}"
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        rows = list(pool.map(lambda case: score(args.rust, case, args.timeout), cases))
    bound = {(row["package"], row["name"]) for row in rows}
    for package in PACKAGES:
        for golden in sorted((UPSTREAM / "testdata" / package).rglob("*.exp.json")):
            name = golden.relative_to(UPSTREAM / "testdata" / package).as_posix()[:-len(".exp.json")]
            if (package, name) not in bound:
                rows.append(dict(package=package, name=name, golden=str(golden.relative_to(UPSTREAM)), outcome="unbound-golden", reason="no current test hook input; retained golden is not counted as passing"))
    summary = collections.defaultdict(collections.Counter)
    for row in rows:
        summary[row["package"]][row["outcome"]] += 1
        if "text_outcome" in row:
            summary[row["package"] + "-text"][row["text_outcome"]] += 1
    report = dict(rust=str(args.rust), rust_sha256=digest(args.rust), cases=str(args.cases), cases_sha256=digest(args.cases), summary=dict(summary), rows=rows)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(report["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
