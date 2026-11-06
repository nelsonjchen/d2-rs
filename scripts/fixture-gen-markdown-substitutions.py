"""Generator for recipe mathtext-txt-markdown-substitutions (tools/fixtures/mathtext-txt.toml).

Writes crates/d2-rs-text/tests/fixtures/master-markdown-substitutions.json:
every textmeasure.ReplaceSubstitutionsMarkdownBounded call made by the
upstream lib/textmeasure substitution tests at the pin, with its request
(source, variables, limit, context policy) and complete result (text, work
count, error and error kind).

The calls are observed by running those upstream tests with every
ReplaceSubstitutionsMarkdownBounded( call routed through the env-gated
tools/language-master-scoreboard/markdown_hook_test.go observer, supplied via
`go test -overlay` (the patched upstream tree is not modified). The upstream
assertions stay active, so the run fails if any original test fails.
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fixture_recipe  # noqa: E402

LOGICAL = "crates/d2-rs-text/tests/fixtures/master-markdown-substitutions.json"
RELATIVE = "lib/textmeasure/substitutions_test.go"
HOOK = "tools/language-master-scoreboard/markdown_hook_test.go"
TESTS = {
    "TestReplaceSubstitutionsMarkdownBounded": 5,
    "TestMarkdownMatcherBoundsSuffixCandidateWork": 2,
    "TestReplaceSubstitutionsMarkdownChargesIdentityReplacementWork": 2,
    "TestReplaceSubstitutionsMarkdownLeavesUnknownPlaceholdersUnchanged": 1,
    "TestReplaceSubstitutionsMarkdownIgnoresUnreferencedVariablesInLinearTime": 1,
    "TestReplaceSubstitutionsMarkdownBoundedCancellation": 1,
    "TestReplaceSubstitutionsMarkdownMatchesManyPatternsInLinearTime": 1,
}


def main() -> None:
    repo = Path(fixture_recipe._env("D2RS_REPO_ROOT"))
    upstream = Path(fixture_recipe._env("D2RS_UPSTREAM"))
    work = fixture_recipe.workdir()
    dump = work / "dump"
    dump.mkdir()
    rewritten = work / "substitutions_test.go"
    rewritten.write_text(re.sub(r"\bReplaceSubstitutionsMarkdownBounded\(", "d2rsMarkdownReplace(t, ",
                                (upstream / RELATIVE).read_text()))
    overlay = work / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(upstream / RELATIVE): str(rewritten),
        str(upstream / "lib/textmeasure/d2rs_markdown_test.go"): str(repo / HOOK),
    }}))
    env = dict(fixture_recipe.go_env(), D2RS_MASTER_LANGUAGE_DUMP=str(dump))
    subprocess.run(["go", "test", "-mod=readonly", "-count=1", "-overlay", str(overlay),
                    "-run", "^(" + "|".join(TESTS) + ")$", "./lib/textmeasure"],
                   cwd=upstream, env=env, check=True, stdout=sys.stderr)
    cases = [json.loads(p.read_text()) for p in dump.glob("*.json")]
    names = sorted(c["name"] for c in cases)
    expected = sorted(f"{test}#{i}" for test, count in TESTS.items() for i in range(count))
    if names != expected:
        sys.exit(f"observed calls {names} != expected {expected}")
    for case in cases:
        # The hook records time.Until(deadline) at call time, which varies by
        # microseconds between runs. The upstream tests use whole-second
        # context.WithTimeout values that never expire during a call, so the
        # deadline is recorded as that configured timeout (rounded to whole
        # seconds). The adapter re-arms a deadline that far away, so the
        # reader observes the same unexpired deadline.
        policy = case["request"]["context_policy"]
        if policy.get("kind") == "deadline":
            policy["remaining_ns"] = round(policy["remaining_ns"] / 1e9) * 1_000_000_000
    cases.sort(key=lambda c: hashlib.sha256(c["name"].encode()).hexdigest())
    manifest = json.loads((repo / "reference-manifest.json").read_text())
    fixture_recipe.output(LOGICAL).write_text(json.dumps(
        {"source_commit": manifest["d2"]["commit"], "source_path": RELATIVE, "cases": cases},
        separators=(",", ":"), ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
