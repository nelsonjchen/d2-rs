# Scoreboard-first workflow

> **Current baselines (D2 master 6a7b48378):** every scoreboard's latest
> report lives in `.reference/scoreboard-master-6a7b4837/` (e2e, raster per
> engine, CLI, cli-diff, flag matrix, golden tiers, inline, language). The
> dated `*-2026102x/` directories below hold the v0.9.0-era baselines.

This is how parity work on d2-rs is chosen, run and verified as of
2026-10-22. `docs/PLAN.md` (execution strategy) states the policy; this file
is the operating manual. `AGENTS.md` makes it binding for every agent.

## Why this exists: the Codex period

Most of d2-rs up to commit `c8fb9af` was written by OpenAI Codex agents. Much
of the port is good, and its source-mapped style is worth keeping. The
problem was work selection. The last stretch of that work went into
ever-narrower TALA qualification:

- compaction rollback and mutation-control matrices (7,196 source cases,
  14,392 native invocations per build);
- live-candidate poll strings and point-identity checks;
- dozens of `docs/tala-*-20261002.md` witness notes.

None of it was tied to a failing user-visible output, and **nothing measured
d2-rs against the full set of upstream goldens**. STATUS reported thousands of
matching internal controls while basic board-level parity was unknown.

The first full scoreboard (this workflow) showed:

- Only **750 of 944** upstream e2e boards matched at the target level.
- The largest losses were not deep TALA states. They were cheap
  exporter/style bugs repeated on every engine. One wrong `isCurve` guess cost
  59 boards. Theme special rules, dashed stroke and Markdown bold cost 42 more.
- The Rust SVG is its own `d2-rs-*` dialect, so **0** boards match upstream
  SVG bytes.

One session of scoreboard-driven fixes took target parity to **875 / 944**
with zero regressions. The TALA bug actually fixed in that session (hierarchy
eligibility ignoring tree sentinels) came from reducing a failing board to a
6-line witness, not from a control matrix.

**Lesson for any agent, Codex included:** internal-state qualification is
evidence, not progress. Pick work by how many upstream goldens it fixes.

## The loop

1. **Check disk** (AGENTS.md ceilings). Stop and clean up if any is exceeded.
2. **Build and snapshot the binary.** Concurrent builds (including subagents)
   rewrite `target/diagnostic/d2-rs` mid-run, so score a copy:

   ```bash
   cargo build --profile diagnostic -p d2-rs-cli --bin d2-rs
   ```
   ```bash
   cp -c target/diagnostic/d2-rs target/diagnostic/d2-rs-score
   ```
3. **Score everything:**

   ```bash
   TMPDIR=<scratchpad> uv run --no-project python scripts/e2e-scoreboard.py --rust target/diagnostic/d2-rs-score --jobs 8 --timeout 120 --output .reference/scoreboard-master-6a7b4837/e2e-new.json
   ```
4. **Check the delta.** It must show zero regressions:

   ```bash
   uv run --no-project python scripts/e2e-scoreboard-delta.py .reference/scoreboard-master-6a7b4837/e2e.json .reference/scoreboard-master-6a7b4837/e2e-new.json
   ```
5. **Cluster the remaining mismatches by field** and fix the biggest cluster
   first. Fields that fail identically on dagre, elk and tala are
   exporter/style bugs and pay off three times. The diff tool needs the
   traces build (`--features diagnostic-traces`, copied to
   `target/diagnostic/d2-rs-traces`):

   ```bash
   uv run --no-project python scripts/e2e-target-diff.py stable/direction --engine dagre
   ```
6. **Reduce layout failures to a minimal witness** before reading code:

   ```bash
   uv run --no-project python scripts/reduce-d2-witness.py in.d2 -- sh -c '<rust cli> "$0" out.svg 2>&1 | grep -q "<symptom>"'
   ```
   Confirm the pinned Go CLI (`reference-manifest.json` `d2.oracle`) behaves
   correctly on the witness. Then do static source comparison and fix the
   earliest divergence (AGENTS.md).
7. **Record the result.** Put the scoreboard table, the fix list with Go
   source references and the remaining queue in `docs/STATUS.md`. Replace
   `latest.json`, and delete intermediate reports.

Regenerate the case dump only when the pinned source changes. The env-gated
hook is `tools/patches/d2-e2e-case-dump.patch`, applied in the trace checkout
(`../d2-trace`); the dump directory is the manifest's `d2.e2e_cases`:

```bash
D2RS_E2E_DUMP=/Users/nelson/code/d2-rs/.reference/e2e-cases-6a7b4837 GOMODCACHE=/Users/nelson/code/d2-rs/.reference/go-mod/pkg/mod GOCACHE=/Users/nelson/code/d2-rs/.reference/go-build GOFLAGS=-mod=mod GOPROXY=off go test ./e2etests -run TestE2E -count=1
```

## Rules

- **Every investigation names the boards it fixes.** Trace captures and
  control matrices are tools for a failing board, not deliverables.
- **No open-ended qualification.** If no scoreboard failure needs an internal
  state, don't qualify it. Retained TALA control corpora stay as regression
  evidence; they are not the work queue.
- **Zero regressions, reported as a delta.** Never claim a fix without the
  before/after scoreboard.
- **Go CLI first for suspicious cases.** Before chasing a mismatch, run the
  Go CLI through the same harness (`--go`) to rule out harness artifacts.
- **Inapplicable is never passing.** Measured-text cases, empty-diagram CLI
  output and boards without goldens are reported separately.
- **Delegate only well-scoped witnesses.** A subagent gets a minimal
  witness, the Go reference behavior, files it may and may not touch, and the
  scoreboard command to verify with.

## Tiers and what's next

| tier | meaning | status 2026-10-22 |
|---|---|---|
| error parity | expected compile/layout errors match | 11 / 11 |
| target hash | D2's hash of shapes/connections/root/config | 875 / 944 |
| SVG bytes | normalized d2svg output | 0 / 944 (needs the d2svg port) |

Scoreboards still to add, in order: d2compiler (375 goldens), d2oracle (445),
d2ir (103), d2parser (45), d2exporter (19), d2sketch (39), e2etests-cli (61),
then raster/PDF/PPTX through the existing raster gate.

## Language scoreboard (started in run 7)

`scripts/dump-language-scoreboard.py` installs env-gated hooks only in the
instrumentation checkout's Go tests. Run it through `run-bounded.py` with a
fresh output directory inside `.reference/codex-runs/scratch/`. It uses the
absolute shared Go caches and verifies the pristine pinned source first.
The dump preserves raw input bytes as hex (including the UTF-16 parser case),
source paths and virtual import files. Go must reproduce each retained golden.

Build the Rust-only runner with
`cargo build --profile oracle -p d2-rs-editor --example language-scoreboard`.
Then run `scripts/language-scoreboard.py --dump <fresh-dump> --output <report>`
to import the inputs once, or omit `--dump` to reuse the immutable cases at
`.reference/language-scoreboard-20261022/cases-v2.json`. Run it with uv under the
same bounded wrapper. `scripts/language-scoreboard-delta.py <before> <after>`
rejects missing keys and lost matches at either tier.

Every retained parser/IR/compiler/editor golden remains inventoried. Unbound
historical goldens, stale Go goldens, native errors, timeouts and unsupported
APIs are non-passing outcomes. Parser and IR compare complete JSON, including
references, ranges and null slices. The compiler adapter constructs the
pre-SetDimensions graph from native source records; all 344 live compiler
goldens match (31 historical goldens remain unbound). Editor formatted-source/error
parity is a separate tier from complete graphs. Both tiers now match all 426
live editor cases using the same native compiler projection (19 editor goldens
remain unbound).
`scripts/test_language_scoreboard.py` tests the harness failure gates.

Run 11 adds the 19 current `TestUpdateImport` text contracts. These bring the
editor source/error tier to 445 live matches, while complete editor graphs stay
at 426. Fifteen historical UpdateImport graph files have the same names as
current text tests; they remain `unbound-golden`, and four new relative-path
tests have `no-golden` graph outcomes. A text match never counts as a graph
match. The other 38 unbound goldens have no current test name in the pinned
runner (`run11-unbound-audit.json`, backed by `--json` Go test events).
Keep all 53 non-passing historical rows. Do not reconstruct source inputs from
expected graphs or assume renamed tests reproduce obsolete goldens.

## CLI contracts and retained goldens (run 18)

`scripts/cli-scoreboard.py` runs the pinned Go `e2etests-cli` test bodies,
then the same bodies through a native subprocess adapter. Its checked-in hook
is `tools/cli-scoreboard/hook_test.go`; installation changes only the trace
checkout. It inventories all 64 main cases, six PNG/GIF fixture cases and all
61 retained goldens. A passing current assertion never makes a historical
golden pass: Go must reproduce that artifact independently. Skipped, unbound,
stale, unsupported, timed-out and missing results remain non-passing.

```sh
UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project --with pillow python scripts/cli-scoreboard.py --rust target/diagnostic/d2-rs --capture .reference/cli-scoreboard-20261023/fresh-capture --output .reference/cli-scoreboard-20261023/new.json
uv run --no-project python scripts/cli-scoreboard-delta.py .reference/cli-scoreboard-20261023/latest.json .reference/cli-scoreboard-20261023/new.json
```

Capture paths must be fresh. The streaming guard covers active temporary
outputs and logs: 1 GiB total, 128 MiB per file, 20 GiB minimum free space.
`--reuse` rescans a completed capture and verifies the native executable hash.
PNG comparisons reuse the complete existing inventory/pixel gate; GIF compares
every composited RGBA frame, delay, canvas and loop count. SVG allows only the
existing e2e XML/version/newline normalization; the Go golden qualification is
strict. PDF/PPTX comparisons currently conservatively require exact bytes in
addition to upstream assertions, so incidental encoding differences can remain
non-passing. Two injected Go writer tests cannot be represented by this process
adapter and are explicitly unsupported. Empty fixture inputs are recorded
separately and never accepted as empty rendered output.

The CLI delta also guards the original Go/Rust test assertions independently
of artifact bytes. An assertion improvement with an artifact mismatch does not
count as a contract/golden match. Capture failures, unsupported native writers
and upstream skips cannot pass this assertion tier.

For reproducible strict PDF bytes, pass `--pdf-epoch <Unix-seconds>` to the
CLI scoreboard. It explicitly gives both implementations `SOURCE_DATE_EPOCH`,
uses UTC for both PDF dates, and selects FPDF's documented `SetCatalogSort(true)`
test control in the trace checkout. Rust already emits that stable catalog
order. The manifest records these controls; `--reuse` rejects a changed clock.
Without this option the harness removes any inherited `SOURCE_DATE_EPOCH` and
tests the ordinary clocks. Complete PDF bytes still compare exactly, and the
original page-count/positive-dimension assertions run in both profiles. Retained
historical PDF goldens still require independent Go reproduction. Clock-controlled
matches are qualified under this explicit profile, not ordinary wall-clock byte
identity. No timestamp or catalog normalization is performed by the comparator.

For the four PPTX output cases, `--pptx-release` additionally runs the original
Go tests in a separate binary with upstream's release linker stamp
(`ci/release/_build.sh:57`, `-X github.com/d2lang/d2/lib/version.Version=v0.9.0`).
It retains the ordinary source run and its explicit AppVersion mismatches in
`source_comparison`, then compares complete release-Go/native PPTX bytes.
Both source and release assertions must pass. Historical goldens still qualify
only against the ordinary source capture. No metadata is rewritten and the
native release version is unchanged. The opt-in profile is manifest-bound;
`--reuse` cannot switch it. Combine with `--pdf-epoch 946782245` for the
separate release/shared-clock profile, published as `latest-release-clock.json`.
The source profiles remain `latest.json` and `latest-clock.json`.

The adapter preserves the original test's split between `ms.PWD` for document
paths and process cwd for custom font reads (`d2cli/main.go:1109`). It removes
the executable's `err: ` transport prefix once per line (bare `err:` denotes
a blank line); `xmain.TestState.Wait` supplies
the in-process program-name prefix once. Complete diagnostic bodies and raw
output captures remain. The hook's `TestD2rsCLIProcessError` checks the complete
multiline error through that actual xmain boundary.
Run 35 also captures `validateCmd`'s direct process-stdout write
(`d2cli/validate.go:37`). An env-gated hook tees the same formatted bytes to
the capture sink while preserving process stdout. Go and Rust sequentially
use the same fresh per-case input path (`D2RS_CLI_WORK`), with stale-directory
reuse rejected. Validation stdout therefore compares exact bytes, including
the absolute input filename; no path or success-text normalization is allowed.
The manifest records all hook files and the pinned validation source.
Run `scripts/test_cli_scoreboard.py` with `--with pillow` to check failure gates.

## Sketch and exporter golden tiers (run 19)

`scripts/golden-tier-scoreboard.py --tier sketch|exporter` inventories every
retained golden and captures the original Go test bodies through env-gated
hooks in the instrumentation checkout. The native example reuses the CLI's
compile/measure/layout/export path. Build it once:

```sh
cargo build --profile oracle -p d2-rs-cli --example golden-tier-scoreboard
UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project python scripts/golden-tier-scoreboard.py --tier sketch --capture .reference/golden-tier-scoreboard-20261023/new-sketch --output .reference/golden-tier-scoreboard-20261023/new-sketch.json
UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project python scripts/golden-tier-scoreboard.py --tier exporter --capture .reference/golden-tier-scoreboard-20261023/new-exporter --output .reference/golden-tier-scoreboard-20261023/new-exporter.json
uv run --no-project python scripts/golden-tier-scoreboard-delta.py <before.json> <after.json>
```

Capture paths must be fresh. `--reuse` verifies the native executable hash and
every retained capture file before rescoring. The streaming budget includes
Go's live temporary `.got` files beside the goldens: 1 GiB total, 128 MiB per
file, at least 20 GiB free. Missing events, empty/corrupt artifacts, timeouts,
stale Go results and unbound goldens remain non-passing.

Sketch compares complete SVGs using only the existing e2e XML/version/newline
allowance. Its historical `dots` golden has no current test input; keep it
unbound. Exporter compares complete JSON after precisely the geometry resets
in upstream `export_test.go:284–297`. Shape `labelPosition` is omitted when
empty; connection `labelPosition` remains an empty string (`d2target.go:812,918`).
Root, legend and arrowhead geometry are preserved. The two original pre-reset
assertions (image dimensions and sequence group position) have a separate
assertion tier. The delta checker guards both tiers. JSON comparison reuses
the language scoreboard's type-aware comparator.

First qualified results: sketch **38/39**, exporter **19/19**, exporter
assertions **2/2**. These are the pinned golden contracts, not full sketch or
exporter API coverage. Run `scripts/test_golden_tier_scoreboard.py` for gates.

### d2svg dark_theme and appendix tiers (2026-10-23, Claude)

`--tier dark` and `--tier appendix` score `d2renderers/d2svg/{dark_theme,
appendix}/testdata` the same way, through the same `SVG_TIERS` table. The hook
records each test's own options: dark_theme uses ThemeID 200 with
`FontFamily: HandDrawn` but no sketch rendering (native `hand_drawn_font`), and
appendix renders with `tc.themeID` then `appendix.Append`. Both hash the root
diagram with its descendant boards, as `d2svg.Render` does. Results: dark
**12/12**, appendix **6/6**; baselines `dark-latest.json`, `appendix-latest.json`.

## Harness gotchas

- Rust's non-animated SVG also carries `d2Version=`. The harness decides
  "animated" from the golden only. For animated boards the target tier reads
  `d2Transition-d2-<master>-0`.
- Don't wait on cargo with `pgrep -f "<cargo command>"`: the loop matches its
  own command line and never exits.
- Thin-LTO object temporaries and incremental state used to grow `target/`
  past its ceiling within one test cycle. The `oracle` profile now has
  `incremental = false`. Still prune `*.rcgu.o` when idle.
- Keep scratch files in a subdirectory; agents sharing the scratchpad have
  wildcard-deleted each other's files.

## Inline-expectation tiers (2026-10-23, Claude)

`scripts/inline-scoreboard.py --suite format|lsp-completion` scores upstream
tests whose expectations live in the test code instead of goldens. An
env-gated hook (`tools/inline-scoreboard/hook_test.go`) records each subtest's
request and Go's own result beside the original assertion. A case passes only
when that Go subtest passes and the native result equals Go's under the
upstream comparison:

- `format`: d2format `TestPrint` through `d2-rs fmt`, exact bytes.
- `format-escape`: d2format `TestEscape{SingleQuoted,DoubleQuoted,Unquoted,
  BlockString}`; the runner rebuilds each test's AST string node and formats
  it with `d2_rs_language::format`, exact bytes.
- `lsp-completion`: d2lsp `TestGetCompletionItems` and `...UTF16` through
  `d2_rs_editor::completion_items` (runner: `cargo build --profile oracle -p
  d2-rs-editor --example inline-scoreboard`), compared with upstream
  `equalCompletionSets`; Go's style completions range over a map.

```sh
uv run --no-project python scripts/inline-scoreboard.py --suite lsp-completion --capture .reference/inline-scoreboard-20261023/<fresh> --output .reference/inline-scoreboard-20261023/<fresh>.json
```

Results: format **35/35**, format-escape **28/28**, lsp-completion **40/40**
(was 26/40); baselines `<suite>-latest.json`.

## Exact-time isometric frames (run62)

The exact-time isometric capture tier is separate:
`scripts/isometric-frame-scoreboard.py --capture <fresh> --rust <frames-runner> --output <report>`.
Build `cargo build --profile diagnostic -p d2-rs-isometric --example isometric-frames`;
`--reuse` replays a hash-verified source capture without instrumenting Go again.
The hook observes original CaptureFrames calls and five named GIF prerequisite
board witnesses, keeping original Go assertions active. The current corpus
has **15 sessions / 32 complete PNG frames**, including emitted prefixes on
callback failure and cancellation. All frame inventories use the shared raster
gates, followed by complete PNG byte comparison; no animation normalization.
The capture has streaming **1 GiB total / 128 MiB file / 20 GiB free** limits
and 600 s subprocess deadlines. `scripts/test_isometric_frame_scoreboard.py`
checks missing/extra/corrupt frames, complete errors and emitted prefixes.
This is CaptureFrames qualification, not GIF/timeline encoder or CLI support.

## Original PNG test calls (Lane L74)

`scripts/e2e-png-scoreboard.py --rust <frozen-isometric-png-runner>
--capture <fresh-directory> --output <fresh.json>` observes the original
`e2etests/png` `TestPNG` calls. Build the runner with `cargo build --locked
--profile diagnostic -p d2-rs-isometric --example isometric-png`, then snapshot
it before capture. All original assertions remain active against private
snapshot copies; pinned fixtures are never written. Native input contains
only the original diagrams and the runner uses the original 1200×1200,
fit-content PNG options. All 20 outputs must reproduce Go and the goldens
with exact decoded pixels through the existing complete raster gates.

`--reuse` verifies captured hashes and requires fresh native/output paths.
Captures have streaming 1 GiB total / 128 MiB file / 20 GiB free guards and
finite deadlines. Run `scripts/test_e2e_png_scoreboard.py` for missing/extra,
corrupt, empty, timed-out and unreproduced-golden failure gates. Coverage of
this named original function adds no new golden matches to the existing
20/20 PNG tier. Upstream snapshot-maintenance, HTML-report and npm publishing
helper tests are separately recorded as infrastructure, never native passes.

## Isometric GIF image API (run63)

`scripts/isometric-gif-scoreboard.py --capture <fresh> --rust <gif-runner> --output <report>`
observes original Render/admission tests and five named traffic/node/legend
boards. Build `cargo build --profile diagnostic -p d2-rs-isometric --example isometric-gif`.
`--reuse` verifies the pinned source, complete call/event inventories and
original Go assertions before replaying the captured requests. Current result:
**17/17 calls**, comprising six GIFs / **592 frames** and 11 complete errors.

GIF comparison reuses the CLI gate: every composited RGBA frame, delay,
canvas and loop count must match. Native tests additionally bind the global
palette and every delta rectangle. Encoded GIF byte identity is not claimed.
Missing/extra results, empty/corrupt images, timeouts and budget exhaustion fail.
Limits are **1 GiB total / 128 MiB file / 20 GiB free**, with streaming guards,
a capture-local Go temporary directory and 600-second subprocess deadlines.
Run `scripts/test_isometric_gif_scoreboard.py` for the gates. The baseline is
`isometric-gif.json`; RenderTimeline and animated CLI output remain separate.

## Target-selection board links (run61)

`scripts/target-link-scoreboard.py --rust <frozen-cli> --capture <fresh> --output <report.json>`
compares complete ordinary SVG bytes, logs, output inventories and board
manifest hashes with the pinned CLI. The 72 finite source cases cover the
L15 internal-link/tooltip witness and a nested node/connection-link document,
all built-in layouts, root/named/wildcard targets and file/stdout destinations.
No SVG normalization is applied. Each capture has streaming limits of 1 GiB
total, 128 MiB/file and 20 GiB free. Missing/corrupt artifacts and timeouts fail;
`scripts/test_target_link_scoreboard.py` checks the failure gates.

Baseline: `target-links.json`, **66/72** (54 complete SVGs, 12 exact errors).
The six named-subtree stdout invocations remain non-passing: Go's board planner
turns `-` into a literal directory and emits extensionless SVG files; Rust
rejects them. This corpus adds no upstream golden claims.

## Master isometric CLI source contracts (run 57)

`scripts/isometric-cli-scoreboard.py` compares complete native SVG/PNG
artifacts and complete errors with the pinned Go CLI on 50 named invocations
from the isometric CLI test contracts. It tests flag/environment/source
precedence, static boards and selected subtrees, file/stdout output, scale
admission and SVG document options. It is a finite subprocess corpus, not
execution of all in-process isometric CLI tests. Source comparisons are
explicitly named `source-match`/`error-match`; no additional golden is claimed.

```sh
cargo build --profile diagnostic -p d2-rs-cli --bin d2-rs
UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project --with pillow python scripts/isometric-cli-scoreboard.py --rust target/diagnostic/d2-rs --capture <fresh-directory> --output <report.json>
```

Both sides share the same input filename. Only existing CLI-diff duration and
working-directory allowances apply to logs; artifact bytes and complete
inventories must match. PNG additionally uses `collect_png_outputs` and
`compare_png_outputs`, including multiboard manifest validation. Writers have
streaming 1 GiB total / 128 MiB file / 20 GiB free guards and a 120-second
per-invocation limit. Missing/corrupt outputs, timeouts and exhausted budgets
cannot pass. `scripts/test_isometric_cli_scoreboard.py` checks these gates.
GIF/timeline qualification remains queued; watch output has the separate
subprocess corpus below.

Run59 adds `--suite paged` (24 new cases from `isometric_paged_test.go`) and
`--suite all` (the original 50 plus those 24). Paged cases compare complete
documents and errors, and require failed file exports to preserve bytes and
mode. `--go <clock-controlled-release-oracle> --pdf-epoch 946782245` selects
the same explicit UTC/FPDF-catalog-sort and release-PPTX controls described
above; no document bytes are normalized. The ordinary oracle remains the
default, and `--suite static` remains the default 50-case corpus. The current
paged result is **24/24** after Lane L15's shared compiler link/tooltip sizing
fix (run60 requalification). The static corpus remains **50/50**.

### Master isometric watch source contracts (run60)

`scripts/isometric-watch-scoreboard.py` drives the real watch servers through
authenticated HTTP/WebSocket requests. It captures 20 stages across source
mode reloads, root/selected/sibling navigation and folder-first previews, with
both SVG and PNG file outputs. Complete preview JSON and artifact bytes compare;
the finite cases follow `d2cli/isometric_watch_test.go`, rather than claiming
to execute all of its in-process assertions.

```sh
UV_CACHE_DIR=$PWD/.reference/uv-cache uv run --no-project --with pillow --with websocket-client python scripts/isometric-watch-scoreboard.py --rust <frozen-cli> --capture <fresh-directory> --output <report.json>
```

The gate inventories every output file, validates every SVG and PNG, and
checks board manifests. Watch can retain a previous multiboard directory
beside a selected-leaf file: isolated byte copies pass the existing
`collect_png_outputs` / `compare_png_outputs` gates, preserving their ordinary
ambiguity rejection. Missing updates, empty/corrupt artifacts, early exits,
shutdown timeouts and streaming budget exhaustion remain failures. Limits:
1 GiB per capture, 128 MiB per file, 20 GiB free, 30 seconds per update and
10 seconds for shutdown. Run `scripts/test_isometric_watch_scoreboard.py`.
Baseline: `isometric-cli-watch.json`, **20/20**. GIF watch is not qualified.
