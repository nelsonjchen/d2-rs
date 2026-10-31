#!/usr/bin/env python3
"""Audit the three sanity/empty goldens through pinned Go's actual WASM Render.

Synthetic API observations do not credit original Test-function coverage.
Full data/error envelopes compare first; the golden tier keeps the existing
version/newline allowance. Uses shared streaming 1 GiB / 128 MiB / 20 GiB free
limits and finite deadlines. No Rust package rebuild is involved in native replay.
"""
import argparse
import base64
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "wasm_admission", Path(__file__).with_name("wasm-admission-scoreboard.py")
)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
base.TEST_FILES = []
base.TESTS = ["TestD2rsWASMEmptyGoldens"]
base.COUNTS = {base.TESTS[0]: 3}
base.HOOK = base.ROOT / "tools/wasm-empty-scoreboard/hook_test.go"
base.GO_TEST_FLAGS = ["-timeout=60s", "-test.testlogfile="]
base.GO_TIMEOUT = 120


def validate_cases(cases, inputs):
    assert len(cases) == len(inputs) == 3
    expected = {row["Name"]: row["Diagram"] for row in inputs}
    assert len(expected) == 3
    assert {row["board"] for row in cases} == set(expected)
    for row in cases:
        request = json.loads(row["request"])
        assert request["diagram"] == expected[row["board"]]
        assert request["options"] == {"pad": 0, "themeID": 0, "sketch": False}


def golden_outcome(data, golden):
    normalize = base.module("d2svg-golden").normalize
    return (
        "go-reproduces-golden" if normalize(data.decode()) == normalize(golden)
        else "stale-in-go-wasm"
    )


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--capture", type=Path, required=True)
    p.add_argument("--rust", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--reuse", action="store_true")
    args = p.parse_args()
    capture = args.capture.resolve()
    pinned = base.references.verify_source()
    inputs = []
    bindings = {}
    for engine in ["dagre", "elk", "tala"]:
        directory = pinned / "e2etests/testdata/sanity/empty" / engine
        board, golden = directory / "board.exp.json", directory / "sketch.exp.svg"
        inputs.append({"Name": engine, "Diagram": json.loads(board.read_text())})
        for path in [board, golden]:
            bindings[str(path)] = base.sha(path)
    if not args.reuse:
        capture.mkdir(exist_ok=False)
        (capture / "inputs.json").write_text(json.dumps(inputs) + "\n")
        (capture / "inputs-hashes.json").write_text(json.dumps(bindings, indent=2) + "\n")
        base.capture_original(capture)
    assert json.loads((capture / "inputs.json").read_text()) == inputs
    assert json.loads((capture / "inputs-hashes.json").read_text()) == bindings
    validate_cases(base.verify(capture), inputs)
    base.score(capture, args.rust.resolve(), args.output.resolve())
    report = json.loads(args.output.read_text())
    cases = {case["name"]: case for case in base.verify(capture)}
    for row in report["rows"]:
        case = cases[row["name"]]
        engine = case["board"]
        data = base64.b64decode(case["got"]["data"], validate=True)
        golden = pinned / "e2etests/testdata/sanity/empty" / engine / "sketch.exp.svg"
        row["board"] = f"sanity/empty/{engine}"
        row["go_output_sha256"] = base.hashlib.sha256(data).hexdigest()
        row["golden_sha256"] = base.sha(golden)
        row["golden_outcome"] = golden_outcome(data, golden.read_text())
    report["master_added_functions"] = 0
    report["scope"] = "Synthetic Go WASM Render observation; native Rust API replay, no packaged runtime qualification"
    report["golden_summary"] = dict(base.collections.Counter(r["golden_outcome"] for r in report["rows"]))
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["golden_summary"])


if __name__ == "__main__":
    main()
