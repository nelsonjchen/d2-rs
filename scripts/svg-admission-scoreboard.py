#!/usr/bin/env python3
"""Capture the original pinned d2svg paint/link tests, then compare complete native output.

All original assertions stay active. Complete errors, nil outputs and SVG bytes
compare without normalization. Fresh capture/replay paths,
1 GiB total / 128 MiB per file / 20 GiB free streaming guard, finite deadlines.
"""

import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path

import references

ROOT = references.ROOT
TEST_FILES = [
    "d2renderers/d2svg/" + n
    for n in [
        "paint_validation_test.go",
        "color_attribute_escaping_test.go",
        "padding_test.go",
        "legend_link_security_test.go",
    ]
]
TESTS = [
    "TestRenderRejectsTargetColorAttributeInjection",
    "TestRenderRejectsDangerousLegendShapeLinksFromJSON",
    "TestRenderRejectsDangerousOrdinaryLinks",
    "TestRenderRejectsExternalPaintServersFromRawJSON",
    "TestRenderMultiboardRejectsExternalPaintServerInNestedBoard",
    "TestRenderRejectsFillPatternAttributeInjectionFromRawJSON",
    "TestRenderPreservesSupportedPaintGrammar",
    "TestRenderCanonicalizesSupportedFillPatterns",
]
COUNTS = dict(zip(TESTS, [14, 2, 2, 19, 1, 3, 16, 3]))
HOOK = ROOT / "tools/svg-admission-scoreboard/hook_test.go"


def transform_tests(text):
    text = text.replace("d2svg.Render(", "d2rsRender(t, ").replace(
        "d2svg.RenderMultiboard(", "d2rsRenderMultiboard(t, "
    )
    if "d2svg." not in text:
        text = text.replace('\t"github.com/d2lang/d2/d2renderers/d2svg"\n', "")
    return text


def structural_suite():
    """Select JSON-representable structural tests; direct cycles/Inf stay uncredited."""
    global TEST_FILES, TESTS, COUNTS
    TEST_FILES = ["d2renderers/d2svg/render_target_validation_test.go"]
    TESTS = [
        "TestRenderRejectsMalformedJSONTargetsWithoutPanicking",
        "TestRenderDefaultsMissingOrEmptyJSONFontFamilies",
        "TestRenderAcceptsValidJSONTarget",
        "TestRenderAcceptsLegendConnectionWithSynthesizedRoute",
        "TestRenderMultiboardRetainsNestedLinkValidation",
    ]
    COUNTS = dict(zip(TESTS, [26, 2, 1, 1, 1]))


def labels_suite():
    """Original escaping and LaTeX admission calls, with exact SVG/error bytes."""
    global TEST_FILES, TESTS, COUNTS
    TEST_FILES = ["d2renderers/d2svg/" + n for n in [
        "class_escaping_test.go", "clip_path_escaping_test.go",
        "id_reference_security_test.go", "sql_table_constraint_escaping_test.go",
        "latex_limits_test.go",
    ]]
    TESTS = [
        "TestRenderEscapesShapeAndConnectionClasses",
        "TestRenderEscapesRoundedSpecialShapeClipPathIDs",
        "TestRenderUsesUniqueResolvableIDsForUntrustedShapeIDs",
        "TestRenderEscapesSQLTableConstraints",
        "TestRenderRejectsExcessiveLatexGroupNesting",
    ]
    COUNTS = dict.fromkeys(TESTS, 2)


def module(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "scripts" / (name + ".py")
    )
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source():
    pinned = references.verify_source()
    result = {
        "commit": references.SOURCE_COMMIT,
        "tests": {n: sha(pinned / n) for n in TEST_FILES},
        "implementation": {
            n: sha(pinned / ("d2renderers/d2svg/" + n))
            for n in ["d2svg.go", "paint_validation.go"]
        },
        "hook": sha(HOOK),
    }
    if TESTS[0] == "TestRenderRejectsMalformedJSONTargetsWithoutPanicking":
        result["target_validation"] = sha(pinned / "d2target/render_validation.go")
    return result


def hashes(capture):
    paths = [
        capture / n for n in ["source.json", "inventory.json", "go.jsonl", "run.json"]
    ]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture):
    assert json.loads((capture / "source.json").read_text()) == source()
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(t) == "pass" for t in TESTS), events
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    inventory = json.loads((capture / "inventory.json").read_text())
    assert {
        t: sum(n for name, n in inventory.items() if name.split("/")[0] == t)
        for t in TESTS
    } == COUNTS
    assert {x.split("/")[0] for x in inventory} == set(TESTS)
    expected = {f"{t}#{i}" for t, n in inventory.items() for i in range(n)}
    assert len(cases) == len(expected) and {c["name"] for c in cases} == expected
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def capture_original(capture):
    pinned = references.verify_source()
    originals = {
        references.TRACE_CHECKOUT / n: (pinned / n).read_bytes() for n in TEST_FILES
    }
    helper = references.TRACE_CHECKOUT / "d2renderers/d2svg/d2rs_admission_test.go"
    assert (
        all(p.read_bytes() == data for p, data in originals.items())
        and not helper.exists()
    )
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "source.json").write_text(json.dumps(source(), indent=2) + "\n")
    env = dict(
        os.environ,
        D2RS_SVG_ADMISSION_DUMP=str(capture / "go"),
        GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
        GOCACHE=str(ROOT / ".reference/go-build"),
        GOFLAGS="-mod=mod",
        GOPROXY="off",
    )
    env.pop("TA", None)
    env.pop("TESTDATA_ACCEPT", None)
    try:
        for path, original in originals.items():
            path.write_text(transform_tests(original.decode()))
        helper.write_bytes(HOOK.read_bytes())
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./d2renderers/d2svg",
                "-ldflags=-X github.com/d2lang/d2/lib/version.Version=v0.9.0",
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
        for path, original in originals.items():
            path.write_bytes(original)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    counts = collections.Counter(
        json.loads(p.read_text())["test"] for p in (capture / "go").glob("*.json")
    )
    (capture / "inventory.json").write_text(json.dumps(dict(counts), indent=2) + "\n")
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture)


def score(capture, runner, output):
    cases = verify(capture)
    work = output.parent / (output.stem + "-native")
    work.mkdir(exist_ok=False)
    runner_hash = sha(runner)
    rows = []
    for case in cases:
        key = hashlib.sha256(case["name"].encode()).hexdigest()
        request, log = work / (key + ".request.json"), work / (key + ".json")
        request.write_text(json.dumps(case["request"]))
        run = module("cli-scoreboard").bounded_run(
            [str(runner.resolve()), str(request.resolve())],
            os.environ.copy(),
            work,
            log,
            30,
        )
        row = {"name": case["name"], "test": case["test"]}
        if run["failure"] or run["exit"]:
            row["outcome"] = run["failure"] or "rust-failed"
        else:
            try:
                actual = json.loads(log.read_text())
            except (ValueError, UnicodeError):
                row["outcome"] = "invalid-output"
                rows.append(row)
                continue
            row["outcome"] = (
                "unsupported"
                if isinstance(actual, dict) and "unsupported" in actual
                else "match"
                if actual == case["got"]
                else "mismatch"
            )
            if row["outcome"] != "match":
                row["expected"], row["actual"] = case["got"], actual
        rows.append(row)
    assert sha(runner) == runner_hash
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner": str(runner),
        "runner_sha256": runner_hash,
        "capture": str(capture),
        "capture_hashes": hashes(capture),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "master_added_functions": len(TESTS),
        "rows": rows,
    }
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--suite", choices=["paint", "structural", "labels"], default="paint")
    args = parser.parse_args()
    if args.suite == "structural":
        structural_suite()
    elif args.suite == "labels":
        labels_suite()
    if not args.reuse:
        capture_original(args.capture.resolve())
    if args.output:
        assert args.rust
        score(args.capture.resolve(), args.rust.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
