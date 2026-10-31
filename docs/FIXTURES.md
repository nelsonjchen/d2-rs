# Oracle fixtures: recipes, cache and test tiers

Go-oracle output is **not committed**. The repository stores the upstream pin
(`reference-manifest.json`), the patch series (`d2.trace_patches`), recipes
(`tools/fixtures/*.toml`), the hooks/harnesses/generators they run, and small
hand-meaningful seed inputs. `scripts/regen-fixtures.py` regenerates the
artifacts into the uncommitted, visible top-level `oracle-cache/` directory
(gitignored), and tests read them from there.

```
oracle-cache/
  fixtures/<logical path>             regenerated artifact
  fixtures/<logical path>.stamp.json  cache key, size, sha256, key material
  upstream-<key>/                     disposable patched upstream tree (removed after a run)
  stage/ work/ logs/                  per-recipe scratch (removed; failure logs kept in logs/)
  reports/                            default output of scripts/check-*.py and other checkers
```

Checker reports (Go-vs-Rust parity ledgers, oracle qualifications, the
engine fixture inventory) are regenerated output too: scripts write them to
`oracle-cache/reports/` by default (`--output`/`--report` still override), and a
script that reads another script's report fails with the command that
regenerates it (`references.read_report`). Never write them under `docs/`.

The **logical path** of an artifact is its workspace-relative former tracked
location, e.g. `crates/d2-rs-text/tests/fixtures/link.json`.
`D2RS_FIXTURES_DIR` overrides `oracle-cache/fixtures` for both the runner and
the tests (relative values resolve against the workspace root). Go module and
build caches stay where AGENTS.md puts them (absolute shared `GOMODCACHE`
`.reference/go-mod/pkg/mod` and `GOCACHE` `.reference/go-build` of the main
checkout; override with `D2RS_GOMODCACHE` / `D2RS_GOCACHE`).

## Two test tiers

| Tier | Command | Needs |
| --- | --- | --- |
| Default | `cargo test --workspace` | nothing beyond Rust; oracle tests are not compiled |
| Oracle | `scripts/test-oracle.sh [cargo test args]` | Go toolchain + regenerated cache |

Tests that read regenerated artifacts are compiled only with their crate's
`oracle-fixtures` feature, gated with `cfg` (never `ignore`): put
`#![cfg(feature = "oracle-fixtures")]` at the top of an integration-test file
whose tests all need artifacts, or `#[cfg(feature = "oracle-fixtures")]` on
individual tests/modules otherwise. Each such crate declares

```toml
[features]
oracle-fixtures = []

[dev-dependencies]
d2-rs-testdata.workspace = true
```

`scripts/test-oracle.sh` regenerates stale artifacts (`D2RS_REGEN_ARGS`,
default `--jobs 4`), then runs `cargo test` with every crate's
`<crate>/oracle-fixtures` feature. By hand:

```sh
uv run --no-project python scripts/regen-fixtures.py --only 'goldmark-*'
cargo test -p d2-rs-text --features oracle-fixtures
```

With the feature on, a missing artifact, missing/malformed stamp, or an
artifact whose size/sha256 differs from its stamp **panics** with the exact
command, e.g.
`uv run --no-project python scripts/regen-fixtures.py --only crates/d2-rs-text/tests/fixtures/link.json`.
Nothing is skipped. (Tests verify artifact-vs-stamp integrity; key freshness
against the current sources is `regen-fixtures.py --check`.)

## Reading artifacts from Rust

```rust
// was: include_str!("fixtures/link.json")
let text: &'static str = d2_rs_testdata::oracle_fixture!("tests/fixtures/link.json");
let bytes: &'static [u8] = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/font.ttf");
let path: std::path::PathBuf = d2_rs_testdata::oracle_fixture_path!("tests/fixtures/cases");
```

Paths are relative to the calling crate's manifest directory (like
`include_str!` from `tests/`), so the logical path is
`<crate dir>/<given path>`. Contents load once per call site (`OnceLock`).
`oracle_fixture_path!` names one stamped file; a recipe that produces many
files lists each in `outputs`.

## Runner

```
uv run --no-project python scripts/regen-fixtures.py [--only GLOB]... [--force] [--jobs N]
    [--list] [--check] [--keep] [--budget-total-mib 1024] [--budget-file-mib 128] [--min-free-gib 20]
```

- `--only` matches recipe names or logical output paths (fnmatch, repeatable);
  `after` dependencies are included automatically.
- Recipes whose key matches every output's stamp are skipped; `--force`
  reruns the selected ones.
- `--check` computes keys without running anything and lists
  stale/missing/corrupt recipes (exit 1 if any). `--list` shows all.
- Budget (AGENTS.md): staged outputs plus logs of the whole run are limited to
  1 GiB, 128 MiB per file, with ≥ 20 GiB free on the volume, checked every
  0.25 s while a step runs. Exhaustion kills the step and fails the recipe.
  Nonzero exit, timeout (default 1800 s per recipe), and missing or empty
  outputs fail too. Failed recipes publish nothing.
- Publishing is atomic per file: the old stamp is removed, the artifact is
  renamed into place, then the new stamp is written, so an interrupted run
  reads as stale.
- The patched upstream tree is `git archive` of the pin from `../d2` (the main
  checkout's sibling) plus the manifest patch series applied in order with
  `git apply --check` first. It is shared by all recipes of a run, keyed by
  pin+patch hashes, verified on reuse, and removed at the end unless `--keep`.
  It is only created when a selected recipe uses it.
- Contract tests for the runner: `uv run --no-project python scripts/test_regen_fixtures.py`.

## Recipe schema

A recipe file holds an optional `[defaults]` table, merged into each
`[[recipe]]` (`env` and `key_deps` merge per key; their lists concatenate).
Unknown fields are errors.

| Field | Kinds | Meaning |
| --- | --- | --- |
| `name` | all | unique, `[a-z0-9][a-z0-9._-]*` |
| `kind` | all | `command`, `go-program` or `go-test-hook` |
| `outputs` | all | logical paths produced (nonempty; each owned by one recipe) |
| `description` | all | free text (not part of the key) |
| `inputs` | all | committed seed files (repo paths), or `{ path = "x.json", generate = [argv] }` run first from the repo root, writing `$D2RS_RECIPE_OUTPUT` (= `workdir/inputs/x.json`) |
| `after` | all | recipe names that must be fresh first (their keys feed this key) |
| `env` | all | extra environment (placeholders expanded) |
| `timeout` | all | seconds for all steps of the recipe (default 1800) |
| `cwd` | all | `repo` (default), `workdir` or `upstream` |
| `upstream` | all | use the patched upstream tree (default: true for `go-test-hook` and `go-program` with `module = "upstream"`) |
| `stdout` | command, go-program | capture stdout into this output instead of the log |
| `argv` | command | argv list |
| `program` | go-program | repo dir of a `main` package (non-test `*.go` copied into a scratch module) |
| `module` | go-program | `upstream` (default: `tools/oracle/go.mod`/`go.sum` with `github.com/d2lang/d2` replaced by the patched tree) or a repo dir holding `go.mod` + `go.sum` |
| `args`, `stdin` | go-program | program arguments; file piped to stdin (e.g. `"{input}"`) |
| `package`, `hooks`, `test` | go-test-hook | upstream package dir, hook `_test.go` files, test function; runs `go test -mod=readonly -count=1 -overlay … -run '^Test$' ./package` in the upstream tree |
| `test_args` | go-test-hook | passed after `-args` |
| `overlay` | go kinds | `{ "upstream/relative/target.go" = "repo/source.go" }` added to the `-overlay` (adds or replaces files without touching the shared tree) |
| `go_flags` | go kinds | extra `go build`/`go test` flags |
| `key_deps` | all | see below |

Placeholders in `argv`, `args`, `stdin`, `env`, `go_flags`, `test_args`:
`{repo}`, `{upstream}`, `{workdir}`, `{fixtures}`, `{stage}`, `{output}` (first
staged output), `{input}` (first input), `{python}` (the runner's
interpreter), `{name}`.

### Cache key

`sha256` of: schema version; the normalized recipe (all fields except
`description`); content hashes of files (automatic: `inputs`, generator argv
entries that are repo files, `hooks`, `overlay` sources, `program` dir and
its `go.mod`/`go.sum`; plus `key_deps.files`); the Go toolchain (go kinds or
`key_deps.go_toolchain = true`); `key_deps.go_modules` versions; for upstream
recipes the pin, patch-series hashes and git tree hashes of
`key_deps.upstream_paths` at the pin (default `.` = whole tree); `key_deps.extra`;
and the keys of `after` recipes.

| `key_deps` field | Meaning |
| --- | --- |
| `files` | repo files, directories (recursive) or globs |
| `upstream_paths` | upstream paths whose tree hashes at the pin matter; narrow these so unrelated upstream changes don't invalidate (e.g. `["d2renderers/d2svg", "lib/textmeasure"]`) |
| `go_modules` | module paths whose versions come from `reference-manifest.json` `dependencies`, or literal `path@version` |
| `go_toolchain` | include the manifest Go toolchain for `command` recipes that run Go |
| `extra` | any value; change it to force a key bump |

Key on what the output actually depends on: a third-party module family (like
Goldmark) keys on the module version, not the D2 pin.

## Environment given to every step

| Variable | Value |
| --- | --- |
| `D2RS_RECIPE_NAME` | recipe name |
| `D2RS_RECIPE_OUTPUT` | staged path of `outputs[0]` |
| `D2RS_RECIPE_OUTPUTS` | staged paths of all outputs, in order, `os.pathsep`-separated |
| `D2RS_RECIPE_STAGE` | stage root; staged path = `$D2RS_RECIPE_STAGE/<logical path>` |
| `D2RS_RECIPE_INPUT` / `D2RS_RECIPE_INPUTS` | first / all input paths |
| `D2RS_RECIPE_WORKDIR` | disposable scratch dir |
| `D2RS_UPSTREAM` | patched upstream tree (empty if unused) |
| `D2RS_REPO_ROOT`, `D2RS_FIXTURES_DIR` | repo root; cache root (read artifacts of `after` recipes here) |
| `GOMODCACHE`, `GOCACHE`, `GOTOOLCHAIN`, `GOFLAGS=-buildvcs=false` | absolute shared caches, manifest toolchain |
| `GOARCH`, `D2RS_GOARCH` | oracle architecture from `--goarch` (default `arm64`) |

Write only to the staged outputs and the workdir. Never write into the
repository, `../d2`, or the upstream tree.

## Hook conventions (`go-test-hook`)

A hook is a `_test.go` drop-in under `tools/fixtures/hooks/<recipe>/`,
overlaid into the upstream package (same package clause). It must be
env-gated so it is inert anywhere else:

```go
func TestD2RSCaptureFoo(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	in := os.Getenv("D2RS_RECIPE_INPUT") // optional seed/generated input
	// ... call package internals, encode JSON deterministically ...
	if err := os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
```

Name tests `TestD2RS…` so they cannot collide with upstream tests; the
runner selects exactly one with `-run '^Name$'`. Prefer hooks and overlays
over new patches; put a change in the manifest patch series only when every
recipe may see it.

## Adding a recipe

1. Pick the artifact's logical path (its current tracked path, if any).
2. Write the hook/harness/generator; make it write `$D2RS_RECIPE_OUTPUT(S)`
   and nothing else, and do **not** gate on Rust equality — `cargo test`
   does the comparison. Python generators can use `scripts/fixture_recipe.py`
   (`output()`, `workdir()`, `go_env()`, `write_request_fixture()`).
3. Add the `[[recipe]]` to a `tools/fixtures/<family>.toml` with tight
   `key_deps`.
4. Switch the Rust reader to `d2_rs_testdata::oracle_fixture!`, gate the
   test with `oracle-fixtures`, add the feature and dev-dependency.
5. Verify on a host with Go:
   `uv run --no-project python scripts/regen-fixtures.py --only <name> --force`,
   compare against the last tracked copy (`git show <rev>:<logical path> | cmp - oracle-cache/fixtures/<logical path>`),
   then `cargo test -p <crate> --features oracle-fixtures` and plain
   `cargo test -p <crate>`; `--check` must report it fresh.
6. `git rm` the tracked artifact.

## Host architecture

Go oracle output depends on the CPU architecture it was built for. The Go
compiler fuses `x*y + z` into one FMA instruction on arm64, which rounds
once, but not on amd64, which rounds twice. Float-heavy captures (ELK, TALA
geometry, render paths, rasters, isometric scenes) therefore differ in the
last bits between the two.

d2-rs mirrors this per architecture. About 69
`cfg(target_arch = "aarch64")` sites in 24 files reproduce Go's arm64 fused
operations, often in inline assembly to keep Go's operand order. Other
architectures use plain arithmetic, as Go does on amd64. A d2-rs build
therefore aims to match Go *built for the same architecture*.

- **Reference platform: arm64.** All parity evidence so far (e2e scoreboard,
  perf-bench, fixtures) was qualified against Go on arm64: Apple Silicon and
  Linux c4a VMs.
- **`--goarch arm64` (default)** builds every Go oracle for arm64 on any host.
  The captures are then the same everywhere, and the oracle tier on an arm64
  host is the qualified gate.
- **`--goarch amd64`** builds Go oracles for amd64. Running the oracle tier
  with these captures on an x86-64 host is how Intel parity is measured.
  That has not been done yet. On x86-64 at 9b706ac1, about a dozen
  default-tier tests (ELK trig/math/radial/random, export raster, isometric
  page/paint, TALA bounds) fail against arm64 captures, which is expected.
  Whether d2-rs on x86-64 matches Go on amd64 is open (`docs/STATUS.md`).
- The architecture is part of every cache key, so switching `--goarch`
  regenerates everything. A foreign architecture must run transparently:
  qemu-user-static with binfmt on Linux, or Rosetta on macOS. The runner checks
  this before running any recipe.
- Golden SVG boards round coordinates and match on both architectures upstream.
  The difference shows up in internal float oracles and raster bytes.

Oracle-tier failures that appear only on one architecture are real
per-architecture parity gaps. Do not paper over them by forcing the other
architecture's captures.

## What may be committed

- Recipes, hooks, harnesses, generators, patches, small `go.mod`/`go.sum`.
- Seed inputs only if small and hand-meaningful (a handful of D2 sources,
  a short case list). Anything bulky or enumerable belongs in a generator.
- **Never** Go-oracle-generated output, stamps, or `oracle-cache/`.
- Generated *source* used by the library at runtime (e.g.
  `crates/d2-rs-text/src/markdown/delimiter/unicode.rs`) stays committed; its
  recipe verifies it and rewrites it only on explicit opt-in
  (`D2RS_UPDATE_UNICODE_TABLE=1`).

## Reference family: Goldmark (`tools/fixtures/goldmark-markdown.toml`)

Ten `command` recipes (`goldmark-*`) producing the eleven
`crates/d2-rs-text/tests/fixtures/*.json` artifacts read by
`crates/d2-rs-text/tests/{aliases,arena,block_reader,context,delimiter,inline_delimiter,link,reader,segments,text_merge}.rs`.
Generators are `scripts/check-markdown-*.py` (via `scripts/goldmark_oracle.py`);
they build `tools/markdown-*-oracle` in a minimal module requiring only
`github.com/yuin/goldmark` at the manifest version, so the key depends on that
version and the Go toolchain, not the D2 pin. Oracle timeouts in the
block-reader capture are kept in the fixture's `oracle_timeouts` field. The
historical `docs/markdown-*-parity.json` reports are no longer rewritten.
