#!/usr/bin/env python3
"""Original imgbundler-network and localfile tests, with actual sockets/files.

Keep every original assertion. Open/read/close errors and bytes compare exactly.
Random cache keys retain their raw bytes in both captures; the tested contract
is their complete equality matrix, not equality of independent random scopes.
1 GiB capture / 128 MiB file / 20 GiB free streaming guards, finite deadlines.
"""

import argparse
import collections
import hashlib
import json
import os
import re
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import references

ROOT = Path(__file__).resolve().parents[1]
TESTS = {
    "network": {"TestBundleRemoteNetworkPolicy": 4},
    "localfile": {
        t: 1
        for t in [
            "TestZeroPolicyDeniesLocalFiles",
            "TestRootedPolicyContainsOpens",
            "TestRootedPolicyRejectsSymlinkEscape",
            "TestRootedPolicyPinsDirectoryAcrossPathReplacement",
            "TestRootedPolicyCacheScopeIsPerInstance",
            "TestUnrestrictedPolicyIsExplicitAndCacheScoped",
            "TestRootedPolicyCloseIsSharedAndCopySafe",
            "TestPolicyRejectsFIFOWithoutBlocking",
        ]
    },
}
FILES = {
    "network": ["lib/imgbundler/network_policy_test.go"],
    "localfile": [
        "lib/localfile/localfile_test.go",
        "lib/localfile/localfile_unix_test.go",
    ],
}
LOCALFILE_OPERATIONS = dict(
    zip(TESTS["localfile"], [4, 16, 7, 13, 7, 9, 40, 8], strict=True)
)
NETWORK_CALLS = {
    "TestBundleRemoteNetworkPolicy/blocks_direct_private_target#0",
    "TestBundleRemoteNetworkPolicy/explicit_opt-in_allows_trusted_private_target#1",
    "TestBundleRemoteNetworkPolicy/cache_is_separated_by_policy#2",
    "TestBundleRemoteNetworkPolicy/cache_is_separated_by_policy#3",
}


def module(name):
    spec = spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    value = module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source(tier):
    pinned = ROOT / references.MANIFEST["d2"]["checkout"]
    paths = FILES[tier] + ["lib/localfile/localfile.go", "lib/localfile/open_unix.go"]
    if tier == "network":
        paths += ["lib/imgbundler/imgbundler.go", "lib/netpolicy/netpolicy.go"]
    return {
        "commit": references.SOURCE_COMMIT,
        "files": {p: sha(pinned / p) for p in paths},
        "hook": sha(
            ROOT
            / "tools"
            / (
                "imgbundler-network-scoreboard"
                if tier == "network"
                else "localfile-scoreboard"
            )
            / "hook_test.go"
        ),
    }


def hashes(capture):
    paths = [
        capture / p for p in ["source.json", "inventory.json", "go.jsonl", "run.json"]
    ]
    paths += sorted((capture / "go").glob("*.json"))
    return {str(p.relative_to(capture)): sha(p) for p in paths}


def verify(capture, tier):
    assert json.loads((capture / "source.json").read_text()) == source(tier)
    assert json.loads((capture / "inventory.json").read_text()) == TESTS[tier]
    assert json.loads((capture / "hashes.json").read_text()) == hashes(capture)
    assert json.loads((capture / "run.json").read_text()) == {
        "exit": 0,
        "failure": None,
    }
    events = module("cli-scoreboard").events(capture / "go.jsonl")
    assert all(events.get(t) == "pass" for t in TESTS[tier]), events
    assert all(action != "fail" for action in events.values()), events
    cases = [json.loads(p.read_text()) for p in sorted((capture / "go").glob("*.json"))]
    if tier == "network":
        assert len(cases) == 4 and {c["name"] for c in cases} == NETWORK_CALLS
        assert all(events.get(c["test"]) == "pass" for c in cases)
    else:
        assert len(cases) == 8 and {c["name"] for c in cases} == set(TESTS[tier])
        assert all(
            len(c["operations"]) == LOCALFILE_OPERATIONS[c["name"]] for c in cases
        )
        assert all(
            events.get("TestPolicyRejectsFIFOWithoutBlocking/" + name) == "pass"
            for name in ["rooted", "unrestricted"]
        )
    sizes = [p.stat().st_size for p in capture.rglob("*") if p.is_file()]
    assert sizes and max(sizes) <= 128 << 20 and sum(sizes) <= 1 << 30
    return cases


def instrument(original, tier):
    if tier == "network":
        return original.replace(
            "BundleRemoteWithPolicy(", "d2rsBundleRemoteWithPolicy(t, &privateHits, "
        ).replace("BundleRemote(", "d2rsBundleRemote(t, &privateHits, ")
    text = original.replace("var policy Policy", "policy := d2rsZero(t)")
    text = text.replace("(Policy{})", "d2rsZero(t)")
    text = re.sub(r"\bPolicy\b", "d2rsPolicy", text)
    text = text.replace("Rooted(", "d2rsRooted(t, ").replace(
        "Unrestricted()", "d2rsUnrestricted(t)"
    )
    text = text.replace("t.TempDir()", "d2rsTempDir(t)")
    for old, new in [
        ("os.Mkdir(", "d2rsMkdir(t, "),
        ("os.WriteFile(", "d2rsWriteFile(t, "),
        ("os.Symlink(", "d2rsSymlink(t, "),
        ("os.Rename(", "d2rsRename(t, "),
        ("syscall.Mkfifo(", "d2rsMkfifo(t, "),
    ]:
        text = text.replace(old, new)
    if "os." not in text:
        text = text.replace('\n\t"os"', "")
    return text


def capture_original(capture, tier):
    pinned = references.verify_source()
    originals = {p: (pinned / p).read_bytes() for p in FILES[tier]}
    package = "lib/imgbundler" if tier == "network" else "lib/localfile"
    helper = references.TRACE_CHECKOUT / package / "d2rs_asset_policy_test.go"
    hook = (
        ROOT
        / "tools"
        / (
            "imgbundler-network-scoreboard"
            if tier == "network"
            else "localfile-scoreboard"
        )
        / "hook_test.go"
    )
    assert not helper.exists()
    assert all(
        (references.TRACE_CHECKOUT / p).read_bytes() == b for p, b in originals.items()
    )
    (capture / "go").mkdir(parents=True, exist_ok=False)
    (capture / "fs").mkdir()
    (capture / "source.json").write_text(json.dumps(source(tier), indent=2) + "\n")
    (capture / "inventory.json").write_text(json.dumps(TESTS[tier], indent=2) + "\n")
    try:
        for p, b in originals.items():
            (references.TRACE_CHECKOUT / p).write_text(instrument(b.decode(), tier))
        helper.write_bytes(hook.read_bytes())
        env = dict(
            os.environ,
            TMPDIR=str(capture / "fs"),
            GOMODCACHE=str(ROOT / ".reference/go-mod/pkg/mod"),
            GOCACHE=str(ROOT / ".reference/go-build"),
            GOFLAGS="-mod=mod",
            GOPROXY="off",
        )
        env[
            "D2RS_IMGBUNDLER_NETWORK_DUMP"
            if tier == "network"
            else "D2RS_LOCALFILE_DUMP"
        ] = str(capture / "go")
        env.pop("TA", None)
        env.pop("TESTDATA_ACCEPT", None)
        result = module("cli-scoreboard").bounded_run(
            [
                "go",
                "test",
                "./" + package,
                "-run",
                "^(" + "|".join(sorted(TESTS[tier])) + ")$",
                "-json",
                "-count=1",
                "-parallel=1",
            ],
            env,
            capture,
            capture / "go.jsonl",
            600,
        )
    finally:
        for p, b in originals.items():
            (references.TRACE_CHECKOUT / p).write_bytes(b)
        helper.unlink(missing_ok=True)
    (capture / "run.json").write_text(json.dumps(result) + "\n")
    assert result == {"exit": 0, "failure": None}, result
    (capture / "hashes.json").write_text(json.dumps(hashes(capture), indent=2) + "\n")
    verify(capture, tier)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", choices=TESTS, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--reuse", action="store_true")
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    capture = args.capture.resolve()
    if not args.reuse:
        capture_original(capture, args.tier)
    if not args.output:
        return
    cases = sorted(
        verify(capture, args.tier),
        key=lambda c: (
            c["name"].rsplit("#", 1)[-1] if args.tier == "network" else c["name"]
        ),
    )
    work = args.output.parent / (args.output.stem + "-native")
    work.mkdir(exist_ok=False)
    request, log = work / "request.json", work / "actual.json"
    request.write_text(
        json.dumps(
            {
                "tier": args.tier,
                "fixture_root": str(capture / "fs"),
                "cases": [
                    {k: v for k, v in c.items() if k not in ["got", "operations"]}
                    | (
                        {"operations": [op["request"] for op in c["operations"]]}
                        if args.tier == "localfile"
                        else {}
                    )
                    for c in cases
                ],
            }
        )
    )
    assert args.rust and args.rust.is_file()
    digest = sha(args.rust)
    run = module("cli-scoreboard").bounded_run(
        [str(args.rust.resolve()), str(request.resolve())],
        os.environ.copy(),
        work,
        log,
        60,
    )
    rows = []
    actual = (
        json.loads(log.read_text()) if run == {"exit": 0, "failure": None} else None
    )
    if actual is not None:
        assert isinstance(actual, list) and len(actual) == len(cases)
    for i, case in enumerate(cases):
        row = {"name": case["name"], "test": case["test"]}
        expected = (
            case["got"]
            if args.tier == "network"
            else [op["got"] for op in case["operations"]]
        )
        if actual is None:
            row["outcome"] = run["failure"] or "rust-failed"
        else:
            got = actual[i]
            # Raw cache keys are deliberately retained separately; the original
            # test contract compares equality, not unpredictable random bytes.
            observed = (
                got.get("observations")
                if isinstance(got, dict) and "observations" in got
                else got
            )
            row["outcome"] = (
                "unsupported"
                if isinstance(got, dict) and "unsupported" in got
                else "match"
                if expected == observed
                else "mismatch"
            )
            if row["outcome"] == "mismatch":
                row["differences"] = module("language-scoreboard").differences(
                    expected, observed
                )
        rows.append(row)
    assert sha(args.rust) == digest
    report = {
        "source_commit": references.SOURCE_COMMIT,
        "runner_sha256": digest,
        "capture_hashes": hashes(capture),
        "summary": dict(collections.Counter(r["outcome"] for r in rows)),
        "captured_functions": len(TESTS[args.tier]),
        "same_native_process": True,
        "rows": rows,
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(report["summary"])


if __name__ == "__main__":
    main()
