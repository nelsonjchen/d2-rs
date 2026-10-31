#!/usr/bin/env python3
"""Full scene JSON and asset hashes from upstream's bundled fallback builds."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "TestBuildResolvesAndRendersBundledNotoColorEmojiWithinFontBudget": 2,
    "TestBuildVariationSelectorEmojiUsesBundledBaseCoverage": 1,
    "TestBuildReusesBundledNotoAcrossStyleBuckets": 1,
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture_go(capture):
    source = references.verify_source()
    trace = references.TRACE_CHECKOUT
    assert (
        subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=trace, text=True
        ).strip()
        == references.SOURCE_COMMIT
    )
    package = trace / "d2renderers/d2scenebuild"
    path = package / "noto_color_emoji_fallback_native_test.go"
    original = path.read_text()
    assert original == (source / "d2renderers/d2scenebuild" / path.name).read_text()
    hook = package / "d2rs_fallback_hook_test.go"
    encoding = package / "d2rs_fallback_encoding_test.go"
    assert not hook.exists() and not encoding.exists()
    try:
        text = original
        for name, expected_count in TESTS.items():
            match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
            assert match, name
            body, count = re.subn(r"\bBuild\(", "d2rsFallbackBuild(t, ", match[0])
            assert count == expected_count, (name, count)
            body = body.replace(
                "d2fonts.NewBundledFallbackResolver(", "d2rsNewBundledFallback(t, "
            )
            text = text[: match.start()] + body + text[match.end() :]
        path.write_text(text)
        hook.write_bytes(
            (ROOT / "tools/scenebuild-fallback-scoreboard/hook_test.go").read_bytes()
        )
        helper = (
            (ROOT / "tools/scene-probe/scenejson.go")
            .read_text()
            .replace("package main", "package d2scenebuild")
        )
        start = helper.index("\t\t// Fonts repeat")
        end = helper.index("\n\tcase *d2scene.FontAsset:", start)
        helper = (
            helper[:start]
            + '\t\tsum := sha256.Sum256(v.Data)\n\t\treturn object{"FontAsset": object{"MIMEType": v.MIMEType, "DataSHA256": hex.EncodeToString(sum[:]), "FaceIndex": v.FaceIndex}}'
            + helper[end:]
        )
        helper = helper.replace('\n\t"os"', "").replace('\n\t"path/filepath"', "")
        encoding.write_text(helper)
        env = os.environ.copy()
        env.update(
            D2RS_FALLBACK_CAPTURE=str(capture),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        with (capture / "events.jsonl").open("w") as log:
            process = subprocess.run(
                [
                    "go",
                    "test",
                    "-json",
                    "./d2renderers/d2scenebuild",
                    "-run",
                    "^(" + "|".join(TESTS) + ")$",
                    "-count=1",
                ],
                cwd=trace,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                timeout=600,
                check=False,
            )
        events = [
            json.loads(line)
            for line in (capture / "events.jsonl").read_text().splitlines()
            if line.startswith("{")
        ]
        outcomes = {
            e["Test"]: e["Action"]
            for e in events
            if "Test" in e and e.get("Action") in ("pass", "fail", "skip")
        }
        assert process.returncode == 0 and all(
            outcomes.get(name) == "pass" for name in TESTS
        ), outcomes
        cases = sorted((capture / "go").rglob("input.json"))
        assert (
            dict(collections.Counter(json.loads(p.read_text())["test"] for p in cases))
            == TESTS
        )
        manifest = {
            "source_commit": references.SOURCE_COMMIT,
            "outcomes": outcomes,
            "source_sha256": hashlib.sha256(original.encode()).hexdigest(),
            "hook_sha256": digest(hook),
            "encoding_sha256": digest(encoding),
            "files": {
                str(p.relative_to(capture)): digest(p)
                for p in capture.rglob("*")
                if p.is_file()
            },
        }
        (capture / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    finally:
        path.write_text(original)
        hook.unlink(missing_ok=True)
        encoding.unlink(missing_ok=True)


def verify_capture(capture):
    manifest = json.loads((capture / "manifest.json").read_text())
    assert manifest["source_commit"] == references.SOURCE_COMMIT
    files = {
        str(p.relative_to(capture))
        for p in capture.rglob("*")
        if p.is_file() and p.name != "manifest.json"
    }
    assert files == set(manifest["files"]), "changed source inventory"
    for name, expected in manifest["files"].items():
        assert digest(capture / name) == expected, name
    cases = sorted((capture / "go").rglob("input.json"))
    assert (
        dict(collections.Counter(json.loads(p.read_text())["test"] for p in cases))
        == TESTS
    )
    inventory = set()
    for case in cases:
        assert manifest["outcomes"].get(json.loads(case.read_text())["test"]) == "pass"
        expected = [
            p for p in case.parent.glob("expected.*") if p.suffix in (".json", ".err")
        ]
        assert len(expected) == 1 and expected[0].stat().st_size > 0
        inventory.update([case, expected[0]])
    assert {p for p in (capture / "go").rglob("*") if p.is_file()} == inventory
    return cases


def compare(expected, actual, exit_code):
    if exit_code or not actual.is_file() or set(actual.parent.iterdir()) != {actual}:
        return False
    if expected.suffix == ".err":
        return expected.read_bytes() == actual.read_bytes()
    # Complete, type-aware JSON comparison used by the language scoreboard.
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "language_scoreboard", ROOT / "scripts/language-scoreboard.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    try:
        return not module.differences(
            json.loads(expected.read_text()), json.loads(actual.read_text())
        )
    except (ValueError, UnicodeError):
        return False


def score(capture, rust, output):
    cases = verify_capture(capture)
    native = output.with_suffix(".native")
    native.mkdir()
    rows = []
    for index, case in enumerate(cases):
        expected = next(case.parent.glob("expected.*"))
        directory = native / str(index)
        directory.mkdir()
        actual = directory / expected.name
        result = subprocess.run(
            [str(rust), str(case), str(actual)],
            capture_output=True,
            timeout=120,
            check=False,
        )
        rows.append(
            {
                "case": str(case.relative_to(capture)),
                "input_sha256": digest(case),
                "outcome": "match"
                if compare(expected, actual, result.returncode)
                else "mismatch",
                "expected_sha256": digest(expected),
                "actual_sha256": digest(actual) if actual.is_file() else None,
                "exit": result.returncode,
                "stderr": result.stderr.decode(errors="replace")[:2000],
            }
        )
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "capture": str(capture),
        "rust_sha256": digest(rust),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "rows": rows,
    }
    output.write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(report["summary"]))


def delta(before, after):
    left = {r["case"]: r for r in before["rows"]}
    right = {r["case"]: r for r in after["rows"]}
    assert left and left.keys() == right.keys(), "changed comparison inventory"
    for name in left:
        assert (
            left[name]["input_sha256"] == right[name]["input_sha256"]
            and left[name]["expected_sha256"] == right[name]["expected_sha256"]
        ), name
    return {
        "before_matches": sum(r["outcome"] == "match" for r in left.values()),
        "after_matches": sum(r["outcome"] == "match" for r in right.values()),
        "improvements": [
            name
            for name in left
            if left[name]["outcome"] != "match" and right[name]["outcome"] == "match"
        ],
        "regressions": [
            name
            for name in left
            if left[name]["outcome"] == "match" and right[name]["outcome"] != "match"
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--rust", type=Path, default=ROOT / "target/diagnostic/examples/fallback-probe"
    )
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture.mkdir(parents=True)
        capture_go(capture)
    score(capture, args.rust.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
