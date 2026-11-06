# Oracle fixtures: recipes, cache and test tiers

This doc explains how Go-oracle test fixtures are regenerated and how the
two test tiers use them. Oracle output is never committed.

The repository stores only what is needed to rebuild the fixtures:

- the upstream pin (`reference-manifest.json`) and its patch series
  (`d2.trace_patches`, under `tools/patches/`);
- recipes (`tools/fixtures/*.toml`) and the hooks, harnesses and generators
  they run;
- small, hand-meaningful seed inputs (`tools/fixtures/inputs/`,
  `tools/fixtures/seeds/`).

`scripts/regen-fixtures.py` rebuilds every artifact into the top-level,
gitignored `oracle-cache/` directory, and tests read them from there.

```
oracle-cache/
  fixtures/<logical path>             regenerated artifact
  fixtures/<logical path>.stamp.json  cache key, size, sha256, key material
  upstream-<key>/                     patched upstream tree (removed after a run)
  stage/ work/ logs/                  per-recipe scratch (failure logs kept in logs/)
  reports/                            default output of scripts/check-*.py and other checkers
```

Checker reports (Go-vs-Rust parity reports, oracle qualifications, fixture
inventories) are regenerated output too. Scripts write them to
`oracle-cache/reports/` by default; `--output` or `--report` overrides that.
A script that reads another script's report fails with the command that
regenerates it (`references.read_report`).

An artifact's **logical path** is a workspace-relative path such as
`crates/d2-rs-text/tests/fixtures/link.json`. `D2RS_FIXTURES_DIR` overrides
`oracle-cache/fixtures` for both the runner and the tests (relative values
resolve against the workspace root). Go caches default to
`.reference/go-mod/pkg/mod` and `.reference/go-build` in the main checkout
(a Git worktree shares its main checkout's caches), resolved to absolute
paths; override them with `D2RS_GOMODCACHE` and `D2RS_GOCACHE`.

## Two test tiers

| Tier | Command | Needs |
| --- | --- | --- |
| Default | `cargo test --workspace` | Rust only; oracle tests are not compiled |
| Oracle | `scripts/test-oracle.sh [cargo test args]` | Go toolchain and the regenerated cache |

Tests that read regenerated artifacts compile only with their crate's
`oracle-fixtures` feature. They are gated with `cfg`, never `ignore`: put
`#![cfg(feature = "oracle-fixtures")]` at the top of an integration-test file
whose tests all need artifacts, or `#[cfg(feature = "oracle-fixtures")]` on
single tests or modules. Each such crate declares

```toml
[features]
oracle-fixtures = []

[dev-dependencies]
d2-rs-testdata.workspace = true
```

`scripts/test-oracle.sh` regenerates stale artifacts (arguments from
`D2RS_REGEN_ARGS`, default `--jobs 4`), then runs `cargo test` with every
crate's `<crate>/oracle-fixtures` feature. By hand:

```sh
uv run --no-project python scripts/regen-fixtures.py --only 'goldmark-*'
cargo test -p d2-rs-text --features oracle-fixtures
```

With the feature on, a missing artifact, a missing or malformed stamp, or an
artifact whose size or sha256 differs from its stamp **panics** with the
exact regen command, for example
`uv run --no-project python scripts/regen-fixtures.py --only crates/d2-rs-text/tests/fixtures/link.json`.
Nothing is skipped. Tests check artifact-vs-stamp integrity; whether a key is
fresh against current sources is `regen-fixtures.py --check`.

## Reading artifacts from Rust

```rust
let text: &'static str = d2_rs_testdata::oracle_fixture!("tests/fixtures/link.json");
let bytes: &'static [u8] = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/font.ttf");
let path: std::path::PathBuf = d2_rs_testdata::oracle_fixture_path!("tests/fixtures/cases");
```

Paths are relative to the calling crate's manifest directory, like
`include_str!` from `tests/`, so the logical path is
`<crate dir>/<given path>`. Contents load once per call site (`OnceLock`).
`oracle_fixture_path!` names one stamped file; a recipe that produces many
files lists each in `outputs`.

## Runner

```
uv run --no-project python scripts/regen-fixtures.py [--only GLOB]... [--force] [--jobs N]
    [--list] [--check] [--keep] [--goarch arm64|amd64] [--verbose]
    [--budget-total-mib 1024] [--budget-file-mib 128] [--min-free-gib 20]
```

- `--only` matches recipe names or logical output paths (fnmatch,
  repeatable). `after` dependencies are included automatically.
- A recipe whose key matches every output's stamp is skipped. `--force`
  reruns the selected recipes.
- `--check` computes keys without running anything and lists stale, missing
  or corrupt recipes (exit 1 if any). `--list` shows all recipes and their
  status.
- Output guard: staged outputs plus logs of the whole run are limited by
  `--budget-total-mib` and `--budget-file-mib`, and the volume must keep
  `--min-free-gib` free. The guard checks every 0.25 s while a step runs;
  exhaustion kills the step and fails the recipe. A nonzero exit, a timeout
  (default 1800 s per recipe) and missing or empty outputs also fail it.
  A failed recipe publishes nothing.
- Publishing is atomic per file: the old stamp is removed, the artifact is
  renamed into place, then the new stamp is written. An interrupted run
  therefore reads as stale.
- The patched upstream tree is a `git archive` of the pin from `../d2`, with
  the manifest's patch series applied in order (`git apply --check` first).
  One tree serves every recipe in a run. It is keyed by pin and patch hashes,
  verified on reuse, created only when a selected recipe needs it, and
  removed at the end unless `--keep` is given.
- Every run first checks Go module pins. The manifest's `dependencies` must
  equal the pinned upstream `go.mod`, and every direct require in a
  `tools/**/go.mod` must equal the manifest. A mismatch stops the run, so a
  stale oracle module cannot silently agree with a stale Rust port. When
  moving the pin, update the manifest from the new upstream `go.mod`, then
  `go get <module>@<version>` in each oracle module the check names.
- Contract tests for the runner:
  `uv run --no-project python scripts/test_regen_fixtures.py`.

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
| `inputs` | all | committed seed files (repo paths), or `{ path = "x.json", generate = [argv] }`, run first from the repo root and writing `$D2RS_RECIPE_OUTPUT` (= `workdir/inputs/x.json`) |
| `after` | all | recipe names that must be fresh first (their keys feed this key) |
| `env` | all | extra environment (placeholders expanded) |
| `timeout` | all | seconds for all steps of the recipe (default 1800) |
| `cwd` | all | `repo` (default), `workdir` or `upstream` |
| `upstream` | all | use the patched upstream tree (default: true for `go-test-hook`, and for `go-program` with `module = "upstream"`) |
| `stdout` | command, go-program | capture stdout into this output instead of the log |
| `argv` | command | argv list |
| `program` | go-program | repo dir of a `main` package (non-test `*.go` copied into a scratch module) |
| `module` | go-program | `upstream` (default: `tools/oracle/go.mod`/`go.sum` with `github.com/d2lang/d2` replaced by the patched tree) or a repo dir holding `go.mod` and `go.sum` |
| `args`, `stdin` | go-program | program arguments; file piped to stdin (e.g. `"{input}"`) |
| `package`, `hooks`, `test` | go-test-hook | upstream package dir, hook `_test.go` files, test function; runs `go test -mod=readonly -count=1 -overlay … -run '^Test$' ./package` in the upstream tree |
| `test_args` | go-test-hook | passed after `-args` |
| `overlay` | go kinds | `{ "upstream/relative/target.go" = "repo/source.go" }`, added to `-overlay` (adds or replaces files without touching the shared tree) |
| `go_flags` | go kinds | extra `go build`/`go test` flags |
| `key_deps` | all | see below |

Placeholders in `argv`, `args`, `stdin`, `env`, `go_flags` and `test_args`:
`{repo}`, `{upstream}`, `{workdir}`, `{fixtures}`, `{stage}`, `{output}` (first
staged output), `{input}` (first input), `{python}` (the runner's
interpreter) and `{name}`.

### Cache key

The key is the `sha256` of:

- the schema version and the normalized recipe (every field except
  `description`);
- content hashes of files: `inputs`, generator argv entries that are repo
  files, `hooks`, `overlay` sources, the `program` dir with its
  `go.mod`/`go.sum`, and `key_deps.files`;
- the Go toolchain (go kinds, or `key_deps.go_toolchain = true`) and the
  oracle architecture;
- `key_deps.go_modules` versions;
- for upstream recipes, the pin, the patch-series hashes and the git tree
  hashes of `key_deps.upstream_paths` at the pin (default `.`, the whole
  tree);
- `key_deps.extra` and the keys of `after` recipes.

| `key_deps` field | Meaning |
| --- | --- |
| `files` | repo files, directories (recursive) or globs |
| `upstream_paths` | upstream paths whose tree hashes at the pin matter; narrow them so unrelated upstream changes don't invalidate the key (e.g. `["d2renderers/d2svg", "lib/textmeasure"]`) |
| `go_modules` | module paths whose versions come from `reference-manifest.json` `dependencies`, or literal `path@version` |
| `go_toolchain` | include the manifest Go toolchain for `command` recipes that run Go |
| `extra` | any value; change it to force a new key |

Key on what the output really depends on. A third-party module family such
as Goldmark keys on the module version, not on the D2 pin.

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
repository, `../d2` or the upstream tree.

## Hook conventions (`go-test-hook`)

A hook is a `_test.go` drop-in under `tools/fixtures/hooks/<recipe>/`,
overlaid into the upstream package (same package clause). It must be
env-gated so that it does nothing anywhere else:

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
over new patches. Add to the manifest patch series only a change that every
recipe may see.

## Adding a recipe

1. Pick the artifact's logical path.
2. Write the hook, harness or generator. It writes `$D2RS_RECIPE_OUTPUT(S)`
   and nothing else, and it does **not** check Rust equality: `cargo test`
   does the comparison. Python generators can use `scripts/fixture_recipe.py`
   (`output()`, `workdir()`, `go_env()`, `write_request_fixture()`).
3. Add the `[[recipe]]` to a `tools/fixtures/<family>.toml` with tight
   `key_deps`.
4. Read it from Rust with `d2_rs_testdata::oracle_fixture!`, gate the test
   on `oracle-fixtures`, and add the feature and dev-dependency.
5. On a host with Go, run
   `uv run --no-project python scripts/regen-fixtures.py --only <name> --force`,
   then `cargo test -p <crate> --features oracle-fixtures` and plain
   `cargo test -p <crate>`. `--check` must report the recipe fresh.

## Host architecture

Go oracle output depends on the CPU architecture Go built it for. The Go
compiler fuses `x*y + z` into one FMA instruction on arm64, which rounds
once, but not on amd64, which rounds twice. Float-heavy captures (ELK, TALA
geometry, render paths, rasters, isometric scenes) therefore differ in the
last bits between the two.

d2-rs mirrors this per architecture. About 69
`cfg(target_arch = "aarch64")` sites in 25 files reproduce Go's arm64 fused
operations, often in inline assembly to keep Go's operand order. Other
architectures use plain arithmetic, as Go does on amd64. A d2-rs build
therefore matches Go *built for the same architecture*.

- **Reference platform: arm64.** Parity is qualified first against Go on
  arm64 (Apple Silicon and Linux arm64).
- **`--goarch arm64` (default, or `D2RS_GOARCH`)** builds every Go oracle for arm64 on any host.
  The captures are then the same everywhere, and the oracle tier on an arm64
  host is the qualifying gate.
- **`--goarch amd64`** builds Go oracles for amd64. Running the oracle tier
  with these captures on an x86-64 host measures x86-64 parity. The oracle
  tier, the default tier and the e2e scoreboard pass on x86-64 Linux against
  amd64 captures, as they do on arm64 against arm64 captures.
- The architecture is part of every cache key, so switching `--goarch`
  regenerates everything. A foreign architecture must run transparently:
  qemu-user-static with binfmt on Linux, or Rosetta on macOS. The runner
  checks this before running any recipe.
- Upstream's golden SVG boards round coordinates and match on both
  architectures. The difference shows up in internal float oracles and
  raster bytes.

An oracle-tier failure that appears on only one architecture is a real
parity gap on that architecture. Don't hide it by using the other
architecture's captures.

## What may be committed

- Recipes, hooks, harnesses, generators, patches, small `go.mod`/`go.sum`.
- Seed inputs, only if small and hand-meaningful (a handful of D2 sources, a
  short case list). Anything bulky or enumerable belongs in a generator.
- **Never** Go-oracle output, stamps or `oracle-cache/`.
- Generated *source* that the library uses at runtime (for example
  `crates/d2-rs-text/src/markdown/delimiter/unicode.rs`) stays committed. Its
  recipe verifies it and rewrites it only on explicit opt-in
  (`D2RS_UPDATE_UNICODE_TABLE=1`).

## Example family: Goldmark (`tools/fixtures/goldmark-markdown.toml`)

Ten `command` recipes (`goldmark-*`) produce the
`crates/d2-rs-text/tests/fixtures/*.json` artifacts read by the Markdown
tests in `crates/d2-rs-text/tests/`. The generators are
`scripts/check-markdown-*.py` (through `scripts/goldmark_oracle.py`). They
build small Go oracles in a minimal module that requires only
`github.com/yuin/goldmark` at the manifest version, so the key depends on
that version and the Go toolchain, not on the D2 pin.
