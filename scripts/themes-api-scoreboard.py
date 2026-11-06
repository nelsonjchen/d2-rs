#!/usr/bin/env python3
"""Original d2themes assertions plus native element serialization/admission.

Full SVG bytes and panic admission compare; panic diagnostic text does not.
Captures retain original assertions and have streaming 1 GiB total / 128 MiB
file / 20 GiB free guards, with finite deadlines and fresh writable outputs.
"""

import argparse
import collections
import importlib.util
import json
import os
import re
from pathlib import Path

import references

ROOT = references.ROOT
TEST_FILE = "d2themes/element_test.go"
TESTS = {
    "TestThemableElementEscapesPaintAttributes": 1,
    "TestThemableElementEscapesAllStringAttributes": 1,
    "TestThemableElementLegacyAttributesAreChecked": 4,
    "TestThemableElementLegacyLocalReferencesAreChecked": 4,
    "TestThemableElementTextAndTrustedContentBoundaries": 2,
    "TestThemableElementRejectsNonFiniteNumbers": 3,
}


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "scripts" / (name + ".py")
    )
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module("asset-policy-scoreboard")


def source():
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    paths = [TEST_FILE, "d2themes/element.go", "lib/svg/markup.go", "lib/svg/text.go"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: base.sha(pinned / p) for p in paths},
        "hook": base.sha(ROOT / "tools/themes-api-scoreboard/hook_test.go"),
    }


def instrument(text):
    text = text.replace("NewThemableElement(", "d2rsThemeNew(t, ")
    text = re.sub(
        r"\b(element|text|group)\.Render\(\)", r"d2rsThemeRender(t, \1)", text
    )
    text = text.replace("text.SetText(", "d2rsThemeText(t, text, ")
    text = text.replace("group.SetInnerSVG(", "d2rsThemeSVG(t, group, ")
    return text.replace("element.SetMaskUrl(", "d2rsThemeMask(t, element, ")


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "inventory.json").read_text()) == TESTS
    assert json.loads((capture / "hashes.json").read_text()) == base.hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(test) == "pass" for test in TESTS), events
    assert all(action != "fail" for action in events.values())
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    expected = {f"{test}#{i}" for test, count in TESTS.items() for i in range(count)}
    assert len(cases) == len(expected) and {c["name"] for c in cases} == expected
    assert collections.Counter(c["test"] for c in cases) == TESTS
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    path = references.TRACE_CHECKOUT / TEST_FILE
    original = (pinned / TEST_FILE).read_bytes()
    helper = path.with_name("d2rs_themes_test.go")
    assert path.read_bytes() == original and not helper.exists()
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "tmp").mkdir()
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS, indent=2) + "\n")
    try:
        path.write_text(instrument(original.decode()))
        helper.write_bytes(
            (ROOT / "tools/themes-api-scoreboard/hook_test.go").read_bytes()
        )
        env = dict(
            os.environ,
            TMPDIR=str(capture / "tmp"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
            D2RS_THEMES_DUMP=str(capture / "go"),
        )
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./d2themes",
                "-run",
                "^(" + "|".join(TESTS) + ")$",
                "-json",
                "-count=1",
                "-parallel=1",
            ],
            env,
            capture,
            capture / "go.jsonl",
            300,
        )
    finally:
        path.write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(
        json.dumps(base.hashes(capture), indent=2) + "\n"
    )
    verify(capture)


def compare(cases, actual):
    assert isinstance(actual, list) and len(actual) == len(cases)
    rows = []
    for case, got in zip(cases, actual, strict=True):
        row = {"name": case["name"], "test": case["test"]}
        row["outcome"] = (
            "unsupported"
            if isinstance(got, dict) and "unsupported" in got
            else "match"
            if module("language-scoreboard").same_json(case["got"], got)
            else "mismatch"
        )
        if row["outcome"] != "match":
            row["expected"], row["actual"] = case["got"], got
        rows.append(row)
    return rows


def functions(rows):
    grouped = collections.defaultdict(list)
    for row in rows:
        grouped[row["test"]].append(row["outcome"])
    return [
        {"name": name, "outcome": next((v for v in values if v != "match"), "match")}
        for name, values in sorted(grouped.items())
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture)
    if not args.output:
        return
    cases = sorted(verify(capture), key=lambda c: c["name"])
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, log = work / "request.json", work / "actual.json"
    request.write_text(json.dumps([c["request"] for c in cases]) + "\n")
    assert args.rust and args.rust.is_file()
    digest = base.sha(args.rust)
    run = module("cli-scoreboard").bounded_run(
        [str(args.rust.resolve()), str(request.resolve())],
        os.environ.copy(),
        work,
        log,
        30,
    )
    assert base.sha(args.rust) == digest
    rows = (
        compare(cases, json.loads(log.read_text()))
        if run == {"exit": 0, "failure": None}
        else [
            {
                "name": c["name"],
                "test": c["test"],
                "outcome": run["failure"] or "rust-failed",
            }
            for c in cases
        ]
    )
    function_rows = functions(rows)
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner_sha256": digest,
        "capture": str(capture.relative_to(ROOT)),
        "capture_hashes": base.hashes(capture),
        "captured_functions": len(TESTS),
        "operations": sum(TESTS.values()),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "function_summary": dict(
            collections.Counter(r["outcome"] for r in function_rows)
        ),
        "function_rows": function_rows,
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
