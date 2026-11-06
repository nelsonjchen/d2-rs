# Reference setup

This doc sets up the upstream Go D2 references that the scoreboards and the
oracle test tier compare against. Plain `cargo test --workspace` needs none
of it.

## What is pinned

`reference-manifest.json` is the single source of truth:

- `d2.commit`: upstream Go D2 `master` at 6a7b48378 (2026-10-11), the
  compatibility target, including its bundled TALA engine;
- `d2.go_toolchain`: Go 1.27.0;
- `d2.checkout` (`../d2`) and `d2.trace_checkout` (`../d2-trace`): the
  pristine and the instrumented checkouts, beside this repository;
- `d2.trace_patches`: the patches applied to the trace checkout;
- `d2.oracle`: the Go D2 CLI built from the pin, with its build command and
  sha256 under `source_builds`;
- `d2.e2e_cases`: the dump of upstream's e2e test cases;
- `dependencies`: the Go module versions from the pinned `go.mod`;
- `releases`: checksums of the official v0.9.0 release archives.

Scripts read these paths through `scripts/references.py`; none hard-codes
them.

## Set up

Run these from the repository root. `go` must be able to fetch Go 1.27.0
(`GOTOOLCHAIN`).

```sh
# 1. Pristine upstream checkout at the pin
git clone https://github.com/d2lang/d2.git ../d2
git -C ../d2 checkout --detach 6a7b4837838a3907604a1d9eec57ee98b6f3cd01

# 2. Instrumented checkout: a worktree at the pin plus the manifest's patches
git -C ../d2 worktree add --detach ../d2-trace 6a7b4837838a3907604a1d9eec57ee98b6f3cd01
git -C ../d2-trace apply "$PWD/tools/patches/d2-e2e-case-dump.patch"
git -C ../d2-trace apply "$PWD/tools/patches/mathtext-svgimport-admission.patch"

# 3. Go modules, downloaded and verified into .reference/go-mod/pkg/mod
uv run python scripts/references.py dependencies

# 4. The Go D2 oracle CLI (the manifest's source_builds command)
GOTOOLCHAIN=go1.27.0 GOMODCACHE=$PWD/.reference/go-mod/pkg/mod GOCACHE=$PWD/.reference/go-build \
  go -C ../d2 build -mod=readonly \
  -ldflags '-X github.com/d2lang/d2/lib/version.Version=v0.9.0' \
  -o "$PWD/.reference/bin/d2-master-6a7b4837-go1.27" .

# 5. The official release archive for this host, then verify everything
uv run python scripts/references.py fetch
uv run python scripts/references.py verify
```

The oracle keeps the last release's `v0.9.0` version stamp. Unstamped master
reports `v0.8.1-HEAD`, and version-sensitive CLI comparisons would otherwise
differ on that label alone.

`references.py` actions:

- `dependencies` downloads and verifies the pinned Go modules, using a copy
  of the pinned `go.mod` so that `../d2` stays untouched.
- `fetch` downloads the v0.9.0 release archive for this host (`--host` picks
  another), checks its sha256 and extracts it under `.reference/releases/`.
- `verify` checks that `../d2` is clean and at the pin, that every
  `source_builds` binary exists with the recorded sha256, Go toolchain and
  version, and that the extracted release matches its archive.

## e2e case dump

`scripts/e2e-scoreboard.py` reads upstream's e2e test cases from the dump at
`d2.e2e_cases`. The env-gated hook from `d2-e2e-case-dump.patch` writes it
when `D2RS_E2E_DUMP` is set. Regenerate it only when the pin moves:

```sh
D2RS_E2E_DUMP=$PWD/.reference/e2e-cases-6a7b4837 \
  GOTOOLCHAIN=go1.27.0 GOMODCACHE=$PWD/.reference/go-mod/pkg/mod GOCACHE=$PWD/.reference/go-build \
  GOFLAGS=-mod=mod GOPROXY=off \
  go -C ../d2-trace test ./e2etests -run TestE2E -count=1
```

## Diagnostic oracle

`scripts/build-oracle.py` builds `tools/oracle` against `../d2` into
`.reference/bin/d2-oracle`. It exposes parse, format, compile, measure,
layout and position as separate operations, and some `scripts/check-*.py`
scripts use it to compare one stage at a time. Rust code never calls it.

## Moving the pin

Update `reference-manifest.json` (commit, `dependencies` from the new
upstream `go.mod`, oracle path and checksum, case-dump path), rebase the
patches in `tools/patches/`, rebuild both checkouts, the oracle and the case
dump, then regenerate the fixtures (`docs/FIXTURES.md`) and rerun every
scoreboard (`docs/SCOREBOARD-WORKFLOW.md`). Do these together.
