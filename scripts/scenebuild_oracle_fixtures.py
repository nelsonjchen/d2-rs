#!/usr/bin/env python3
"""Scene-build and image-asset oracle fixtures (tools/fixtures/scenebuild-oracle.toml).

Run only through the runner, e.g.
`uv run --no-project python scripts/regen-fixtures.py --only 'scenebuild-*'`.

Each subcommand runs selected upstream tests at the manifest pin with the
scoreboard hooks of the matching `scripts/*-scoreboard.py` (the same source
rewrites, applied through `go test -overlay`, so the shared upstream tree is
never modified), with every original Go assertion still active. It runs the
capture twice and requires identical output.

- `fallback`   noto_color_emoji_fallback_native_test.go Build calls: complete
               scene JSON (font assets as hashes) or error, per call.
- `assets`     assets_test.go / latex_test.go Build calls: diagram, asset
               options and every resolver result, then scene or error.
- `imageasset SUITE`  lib/imageasset Resolve calls of one scoreboard suite
               (requests with captured files and HTTP exchanges, results).

Host- and run-specific strings are canonicalized consistently in requests
and results: the upstream tree becomes /d2rs/upstream, each `t.TempDir()`
root /d2rs/tmp/<n>, each httptest port a fixed 5-digit port (1000<n>), and
each HTTP response Date header a fixed date. None of them is part of any
resource payload.
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from fixture_recipe import go_env, output, workdir
from references import ROOT, SOURCE_COMMIT

SCENEBUILD = "d2renderers/d2scenebuild"
FIXTURES = "crates/d2-rs-scenebuild/tests/fixtures/"


def upstream() -> Path:
    value = os.environ.get("D2RS_UPSTREAM")
    if not value:
        sys.exit("D2RS_UPSTREAM is unset: this recipe needs `upstream = true`")
    return Path(value)


def scoreboard(name: str):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / f"scripts/{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def rewrite_tests(text: str, names, pattern: str, replacement: str, helper: tuple[str, str] | None = None) -> str:
    """Apply the scoreboard's per-test-function call rewrite."""
    for name in names:
        match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
        assert match, name
        body, count = re.subn(pattern, replacement, match[0])
        if helper:
            body, helper_count = re.subn(helper[0], helper[1], body)
            count += helper_count
        assert count, name
        text = text[: match.start()] + body + text[match.end():]
    return text


def scene_encoding() -> str:
    """tools/scene-probe/scenejson.go as a package test helper with font
    assets reduced to their hashes (as the scenebuild scoreboards do)."""
    helper = (ROOT / "tools/scene-probe/scenejson.go").read_text().replace("package main", "package d2scenebuild")
    start = helper.index("\t\t// Fonts repeat")
    end = helper.index("\n\tcase *d2scene.FontAsset:", start)
    helper = (helper[:start] + '\t\tsum := sha256.Sum256(v.Data)\n\t\treturn object{"FontAsset": object{"MIMEType": '
              'v.MIMEType, "DataSHA256": hex.EncodeToString(sum[:]), "FaceIndex": v.FaceIndex}}' + helper[end:])
    return helper.replace('\n\t"os"', "").replace('\n\t"path/filepath"', "")


def go_test(package: str, files: dict[str, str], tests, env: dict[str, str], tag: str,
            explicit_files: bool = False) -> dict[str, str]:
    """Run `go test -json` for `tests` in the upstream package with `files`
    (name -> contents) overlaid into it. Returns each test's outcome."""
    overlay_dir = workdir() / f"overlay-{tag}"
    shutil.rmtree(overlay_dir, ignore_errors=True)
    overlay_dir.mkdir(parents=True)
    replace = {}
    for name, text in files.items():
        (overlay_dir / name).write_text(text)
        replace[str(upstream() / package / name)] = str(overlay_dir / name)
    overlay = workdir() / f"overlay-{tag}.json"
    overlay.write_text(json.dumps({"Replace": replace}))
    argv = ["go", "test", "-mod=readonly", "-count=1", "-parallel=1", "-json", "-overlay", str(overlay),
            "-run", "^(" + "|".join(sorted({t.split("/")[0] for t in tests})) + ")$"]
    cwd = upstream()
    if explicit_files:
        # Internal-package files only (the scoreboard's choice): external test
        # files would import a second copy that cannot see test-only hooks.
        directory = upstream() / package
        names = {p.name for p in directory.glob("*.go")} | set(files)
        own = sorted(n for n in names if re.search(r"^package d2scenebuild\s*$",
                                                    files.get(n) or (directory / n).read_text(), re.MULTILINE))
        argv += own
        cwd = directory
    else:
        argv.append(f"./{package}")
    result = subprocess.run(argv, cwd=cwd, env={**go_env(), **env}, stdout=subprocess.PIPE, timeout=900)
    outcomes = {}
    for line in result.stdout.decode().splitlines():
        if line.startswith("{"):
            event = json.loads(line)
            if event.get("Test") and event.get("Action") in ("pass", "fail", "skip"):
                outcomes[event["Test"]] = event["Action"]
    failed = {t: outcomes.get(t) for t in tests if outcomes.get(t) != "pass"}
    assert result.returncode == 0 and not failed, f"go test exit {result.returncode}: {failed or outcomes}"
    return outcomes


def build_cases(capture: Path, tests: dict[str, int]) -> list[dict]:
    """Cases written by the Build hooks: go/<test>/<index>/{input.json,expected.*}."""
    paths = sorted((capture / "go").rglob("input.json"))
    inputs = [json.loads(p.read_text()) for p in paths]
    assert dict(collections.Counter(i["test"] for i in inputs)) == tests, collections.Counter(i["test"] for i in inputs)
    cases = []
    for path, data in zip(paths, inputs):
        if (path.parent / "expected.err").exists():
            cases.append({"input": data, "expected": (path.parent / "expected.err").read_text(), "error": True})
        else:
            cases.append({"input": data, "expected": json.loads((path.parent / "expected.json").read_text()),
                          "error": False})
    return cases


def twice(capture_once) -> str:
    runs = [capture_once(attempt) for attempt in range(2)]
    if runs[0] != runs[1]:
        at = next(i for i, (a, b) in enumerate(zip(*runs)) if a != b) if min(map(len, runs)) else 0
        sys.exit(f"repeated captures differ at byte {at}:\n  {runs[0][max(0, at - 200):at + 200]!r}\n"
                 f"  {runs[1][max(0, at - 200):at + 200]!r}")
    return runs[0]


class Canonical:
    """Consistent replacement of host-, run- and port-specific substrings."""

    def __init__(self, tmp: Path):
        self.tmp = str(tmp)
        self.roots: dict[str, str] = {}
        self.ports: dict[str, str] = {}

    def __call__(self, text: str) -> str:
        text = text.replace(str(upstream()), "/d2rs/upstream")

        def root(match):
            return self.roots.setdefault(match[0], f"/d2rs/tmp/{len(self.roots)}")
        text = re.sub(re.escape(self.tmp) + r"/[^/\"\\]+", root, text)

        def port(match):
            return "127.0.0.1:" + self.ports.setdefault(match[1], str(10000 + len(self.ports)))
        text = re.sub(r"127\.0\.0\.1:(\d+)", port, text)
        # net/http stamps each response with the wall-clock Date header.
        return re.sub(r'"Date":\["[^"]*"\]', '"Date":["Mon, 01 Jan 2001 00:00:00 GMT"]', text)


def write_json(logical: str, value) -> None:
    output(logical).write_text(json.dumps(value, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")


def fallback(_args) -> None:
    board = scoreboard("scenebuild-fallback-scoreboard")
    test_file = "noto_color_emoji_fallback_native_test.go"
    text = (upstream() / SCENEBUILD / test_file).read_text()
    for name, expected_count in board.TESTS.items():
        match = re.search(r"(?ms)^func " + name + r"\(.*?(?=^func |\Z)", text)
        assert match, name
        body, count = re.subn(r"\bBuild\(", "d2rsFallbackBuild(t, ", match[0])
        assert count == expected_count, (name, count)
        body = body.replace("d2fonts.NewBundledFallbackResolver(", "d2rsNewBundledFallback(t, ")
        text = text[: match.start()] + body + text[match.end():]
    files = {test_file: text, "d2rs_fallback_hook_test.go":
             (ROOT / "tools/scenebuild-fallback-scoreboard/hook_test.go").read_text(),
             "d2rs_fallback_encoding_test.go": scene_encoding()}

    def once(attempt):
        capture = workdir() / f"fallback-{attempt}"
        go_test(SCENEBUILD, files, board.TESTS, {"D2RS_FALLBACK_CAPTURE": str(capture)}, f"fallback-{attempt}")
        return json.dumps(build_cases(capture, board.TESTS), sort_keys=True)
    write_json(FIXTURES + "bundled-fallback-master.json",
               {"source_commit": SOURCE_COMMIT, "cases": json.loads(twice(once))})


def assets(_args) -> None:
    board = scoreboard("scenebuild-asset-scoreboard")
    latex = scoreboard("scenebuild-latex-scoreboard")
    production = (upstream() / SCENEBUILD / "assets.go").read_text()
    old = "b.options.Assets.Resolver.Resolve(b.ctx, source)"
    assert production.count(old) >= 1
    production = production.replace(old, "d2rsAssetResolve(b.ctx, b.options.Assets.Resolver, source)", 1)
    for logical, test_file, tests in [("image-assets-master.json", "assets_test.go", board.TESTS),
                                      ("latex-master.json", "latex_test.go", latex.assets.TESTS)]:
        text = rewrite_tests((upstream() / SCENEBUILD / test_file).read_text(),
                             sorted({t.split("/")[0] for t in tests}), r"\bBuild\(", "d2rsAssetBuild(t, ")
        files = {"assets.go": production, test_file: text,
                 "d2rs_asset_hook_test.go": (ROOT / "tools/scenebuild-asset-scoreboard/hook_test.go").read_text(),
                 "d2rs_asset_encoding_test.go": scene_encoding()}

        def once(attempt):
            capture = workdir() / f"{test_file}-{attempt}"
            tmp = workdir() / f"tmp-{test_file}-{attempt}"
            tmp.mkdir(parents=True)
            go_test(SCENEBUILD, files, tests, {"D2RS_ASSET_CAPTURE": str(capture), "TMPDIR": str(tmp)},
                    f"{test_file}-{attempt}", explicit_files=True)
            return Canonical(tmp)(json.dumps(build_cases(capture, tests), sort_keys=True, ensure_ascii=False))
        write_json(FIXTURES + logical, {"source_commit": SOURCE_COMMIT, "test_file": f"{SCENEBUILD}/{test_file}",
                                        "cases": json.loads(twice(once))})


IMAGEASSET_OUTPUTS = {
    "raster": "imageasset-local-data-master.jsonl", "svg": "imageasset-svg-master.jsonl",
    "svg-codecs": "imageasset-svg-codecs-master.jsonl", "http": "imageasset-http-master.jsonl",
    "http-headers": "imageasset-headers-master.jsonl", "cache": "imageasset-cache-master.jsonl",
    "cache-local": "imageasset-cache-local-master.jsonl", "cache-namespace": "imageasset-cache-namespace-master.jsonl",
}


def imageasset(args) -> None:
    board = scoreboard("imageasset-scoreboard")
    tests = board.SUITES[args.suite]
    package = "lib/imageasset"
    test_file = ("remediation_test.go" if tests in [board.SVG_CODEC_TESTS, board.HTTP_HEADER_TESTS,
                                                    board.NAMESPACE_TESTS]
                 else "security_test.go" if tests == board.CACHE_TESTS else "imageasset_test.go")
    helper = (r"\bmustResolve\(", "d2rsMustResolve(") if tests in [board.SVG_CODEC_TESTS, board.HTTP_HEADER_TESTS,
                                                                   board.NAMESPACE_TESTS] else None
    text = rewrite_tests((upstream() / package / test_file).read_text(), sorted({t.split("/")[0] for t in tests}),
                         r"\b([a-zA-Z_]\w*)\.Resolve\(", r"d2rsResolve(t, \1, ", helper)
    files = {test_file: text, "d2rs_capture_test.go": (ROOT / "tools/imageasset-scoreboard/hook_test.go").read_text()}
    if board.uses_http_hook(tests):
        resolve = (upstream() / package / "resolve.go").read_text()
        assert resolve.count("r.client.Do(request)") == 1
        files["resolve.go"] = resolve.replace("r.client.Do(request)", "d2rsHTTPDo(r, request)")

    def once(attempt):
        capture = workdir() / f"imageasset-{args.suite}-{attempt}"
        tmp = workdir() / f"tmp-{args.suite}-{attempt}"
        capture.mkdir(parents=True)
        tmp.mkdir(parents=True)
        go_test(package, files, tests, {"D2RS_IMAGEASSET_CAPTURE": str(capture), "TMPDIR": str(tmp)},
                f"{args.suite}-{attempt}")
        lines = (capture / "calls.jsonl").read_text().splitlines()
        assert collections.Counter(json.loads(line)["request"]["test"] for line in lines) == tests
        canonical = Canonical(tmp)
        return "".join(canonical(line) + "\n" for line in lines)
    output(FIXTURES + IMAGEASSET_OUTPUTS[args.suite]).write_text(twice(once))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("fallback").set_defaults(func=fallback)
    sub.add_parser("assets").set_defaults(func=assets)
    p = sub.add_parser("imageasset")
    p.add_argument("suite", choices=sorted(IMAGEASSET_OUTPUTS))
    p.set_defaults(func=imageasset)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
