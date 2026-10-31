# NALA fork (`crates/d2-nala-rs`)

The `crates/d2-nala-rs` crate forks TALA into NALA (Nelson Advanced Layout
Algorithm), a non-parity layout engine built to Nelson's own spec. Its goal is
edge legibility: no overlapping or ambiguously diverging edges. TALA stays
available as its own parity engine; NALA is added alongside it, never in its
place. The rules in this section override the parity contract below **only
for NALA code**.

- NALA lives in its own crate, `crates/d2-nala-rs`, started as a copy of
  `crates/d2-tala-rs`. Register it as an in-process engine named `nala`
  alongside dagre, ELK and TALA. Never change `d2-tala-rs`, `d2-rs-dagre`,
  `d2-rs-elk` or other parity paths to make NALA work. Shared-crate changes
  are allowed only when they are additive and leave parity output
  byte-identical.
- The spec is `docs/NALA.md`, not Go D2 or TALA output. NALA output is
  expected to diverge from TALA. The Go and JS oracles, trace parity and
  static source comparison do not apply to NALA routing changes.
- Spec invariants are checkable rules, enforced by a validator over the test
  corpus that serves as NALA's scoreboard:
  1. No two edges share a collinear segment; parallel edges keep at least the
     configured lane separation.
  2. Every edge gets its own port slot on a node side, ordered so that edges
     leaving the same side do not cross near the node.
  3. Edges sharing a corridor keep a consistent relative order (transit-map
     lanes) and swap only at nodes.
  4. Edge crossings occur at no less than the spec's minimum angle (default
     60°).
  Track violation counts per invariant against the TALA baseline. Count a
  change as progress only when it reduces violations without regressing
  others.
- Approach: keep TALA placement and rework port assignment, routing and lane
  nudging. Use exact solvers (SAT, MaxSAT, ILP) only for small, local
  subproblems such as corridor lane ordering and port-slot assignment. Each
  subproblem needs a size bound and a heuristic fallback, so that global
  layout never depends on solver time. Keep solver dependencies in pure Rust
  where practical.
- Still binding for NALA: Rust-native algorithms, contract-level tests, no
  fixture-specific outputs, compile-out diagnostics, and every storage,
  retention and cleanup rule below.

# Development contract

- This repository implements D2 in Rust. The compatibility oracle is upstream
  Go D2 `master` at the commit pinned in `reference-manifest.json` (currently
  6a7b48378, 2026-10-11, 107 commits after v0.9.0), including its bundled,
  in-process TALA layout engine. TALA is part of full-D2 parity, not a
  separate external engine project. The v0.9.0 target is finished: its final
  d2-rs checkpoint is the tag `d2-v0.9.0-final`, and v0.9.0-era scripts,
  fixtures and patches (`*-v090*`, `../d2-v0.9.0-trace`) are historical
  evidence, not the current oracle. Move the pin only deliberately, updating
  the manifest, the trace checkout and every scoreboard baseline together.
- Implement the accepted plan in `docs/PLAN.md`; update `docs/STATUS.md` with
  actual evidence and outstanding work. Never equate inherited tests with full
  D2 compatibility.
- Pristine source is `../d2`, fixed by `reference-manifest.json`. Instrument only
  the manifest's trace checkout (`../d2-trace`, rebuilt from the pin plus the
  manifest's `trace_patches`); keep Rust implementation changes in this
  repository. Scripts take the pin, oracle binary and e2e case dump from
  `scripts/references.py`, never from hard-coded paths.
- Keep investigation scoped to this repository and its pinned D2 references;
  unrelated sibling projects and experiments are out of scope. Weftan is a
  finished, archived historical experiment and is irrelevant to this project:
  ignore it entirely. Compare d2-rs only against the pinned upstream Go D2,
  including its bundled TALA; never inspect or use Weftan as an implementation
  source, test corpus, or parity reference.
- **Choose work by the upstream scoreboard** (`docs/SCOREBOARD-WORKFLOW.md`).
  Run `scripts/e2e-scoreboard.py` at session start. Fix the root cause that
  breaks the most golden boards, and report a zero-regression delta for every
  change. Every investigation must name the failing boards it targets.
  Open-ended qualification of internal states (control matrices, mutation
  sweeps, poll-string audits) without a failing board is not progress. This
  applies to every agent, Codex included: earlier Codex-driven work spent
  weeks on narrow TALA compaction controls while 194 upstream boards silently
  failed, most of them to cheap exporter bugs.
- **Performance phase.** Once every upstream scoreboard matches (true since
  2026-10-26), `scripts/perf-bench.py` is the scoreboard. Every perf-bench
  case on which d2-rs is slower than Go on a clean host is a failing board.
  Each investigation names the perf-bench cases and the profile hotspot it
  targets, and reports before → after ratios with byte-identical outputs and
  an unchanged e2e scoreboard. "No failing golden board" is not a reason to
  stop while any engine's clean-host Rust/Go wall ratio exceeds 1.0.
- Start algorithm changes with static source comparison, then fix the earliest
  divergence. No fixture-specific outputs, silent test exclusions or lossy trace
  normalization. Record source mapping and regression evidence.
- Diagnostic argument construction must compile out unless `diagnostic-traces`
  is explicitly enabled. Optimized builds alone are not permission to trace.
- Use uv for Python and pnpm for JavaScript tooling. On macOS, assume zsh; never
  assign to special or read-only shell parameters (especially `status`). Use
  descriptive, task-specific variable names such as `http_status_code` or
  `command_exit_code`. If Git reports `.git/index.lock: Operation not permitted`,
  retry the command once.
- Go and JS engines are test oracles only. Native algorithm paths must be Rust.
- Tests should cover actual contracts. Preserve third-party and source notices.
- **Oracle artifacts are regenerated, never committed** (`docs/FIXTURES.md`).
  The repository stores the upstream pin, the patch series in
  `tools/patches/`, recipes in `tools/fixtures/`, hooks, generators and
  small hand-meaningful seed inputs. `scripts/regen-fixtures.py` rebuilds
  every Go/JS oracle output into the ignored, visible `oracle-cache/`
directory. Don't put space-consuming caches in hidden directories.
  Each recipe's cache key names only what it really depends on: upstream
  paths, module versions, toolchain, hook and patch hashes. Never commit
  regenerated output, compressed or not, and never hand-edit an expected
  value.
- Two test tiers. Plain `cargo test --workspace` is self-contained and needs
  no Go or cache. Tests that read regenerated artifacts compile only under
  each crate's `oracle-fixtures` feature. With the feature on, a missing
  artifact fails loudly with its regen command, never a skip. CI runs both
  tiers.
- Parity is per architecture. Go fuses multiply-add on arm64 but not on
  amd64, and d2-rs mirrors this with `cfg(target_arch = "aarch64")` paths.
  arm64 is the reference platform: qualify parity on arm64 hosts with
  `--goarch arm64` captures. Measure x86-64 parity with `--goarch amd64` on an
  x86-64 host. Never compare a host's Rust output against the other
  architecture's captures and call it drift (`docs/FIXTURES.md`).
- Moving the pin: update the manifest, rebase the patch series, regenerate,
  and run both tiers. A failing oracle test is upstream drift; fix the Rust
  code or the hook, never the artifact.
- Recipe new fixtures only for behavior reachable from D2 input or public
  APIs and not already covered by upstream goldens. Retire fixtures that pin
  unreachable internal states (adversarial callbacks, poll accounting) with a
  written reason; their historical data stays in the private
  `nelsonjchen/d2-rs-history` repository.
- Raster parity captures must require nonempty, complete output inventories and
  validate every board before claiming a match. Reuse `collect_png_outputs` and
  `compare_png_outputs` from `scripts/check-render-parity.py`, or test an
  equivalent gate explicitly. Missing artifacts, corrupt output and timeouts
  are failures. Run `scripts/test_render_*.py` when changing the raster harness;
  historical capture scripts are evidence, not a substitute for this gate.
- Clean up temporary files and disposable generated artifacts created during
  your work once they are no longer needed. Preserve source, deliverables,
  user data, pinned references, reproducible evidence and anything needed by
  running work. Keep cleanup scoped to this project's own artifacts; do not
  broadly delete unrelated files or caches. Record what was removed and the
  space recovered.
- Disk usage is a project-wide constraint, not merely a `target/` constraint.
  Include hidden directories, `.reference/`, `.git/`, dependency/toolchain
  copies, snapshots, traces and captures when measuring the checkout. Measure
  both allocated disk usage and logical/apparent size; compression and hard
  links must not conceal uncontrolled artifact growth in reports. On macOS,
  use `du -sk .` for allocated usage and `du -A -l -sk .` for apparent usage;
  the latter counts every retained path, including hard-linked duplicates.
- Default storage ceilings, unless the user explicitly changes them: **30 GiB
  allocated and 50 GiB apparent for the whole checkout**, **10 GiB allocated
  for `target/`**, and **15 GiB allocated / 30 GiB apparent for `.reference/`**.
  Maintain at least **20 GiB free on the volume**. Check usage at session start,
  before and after artifact-heavy runs, and before reporting completion. If
  already over a ceiling, prioritize scoped cleanup and do not start another
  artifact-heavy run until within budget. Do not describe a bounded `target/`
  as evidence that repository-wide storage is bounded. Existing overages remain
  violations until actually reduced; adding this rule does not resolve them.
  Treat any exceeded ceiling as a stop gate for parity development: clean up
  first and record a fresh whole-checkout measurement proving every ceiling
  passes before resuming. Moving artifacts elsewhere is not cleanup.
  An explicit user cleanup request also takes priority when already under these
  ceilings: remove verified obsolete artifacts, report measured savings, and
  finish the disk audit before returning to parity work. Do not launch new
  parity captures or builds during that cleanup.
- Keep one shared Cargo target and shared oracle/dependency caches. Do not
  use relative `GOMODCACHE`, `GOCACHE` or `CARGO_TARGET_DIR` values when a tool
  changes working directories: resolve them to absolute shared paths in this
  checkout. Do not recreate nested `.reference/go-mod` toolchain/dependency
  copies inside oracle modules; retire obsolete copies before new captures.
  Do not create per-checkpoint target directories, copy full dependency trees,
  retain repeated source archives, or snapshot the full checkout for every experiment.
  Store immutable source inputs once by content hash and reference them from
  manifests. This also applies to `compile-inputs/` and failed/retried
  writer-freeze attempts; those directories must not accumulate full source copies. Reuse the prescribed warm test executables; delete obsolete test
  executable variants after their jobs terminate. Never delete active artifacts.
- The canonical Go module cache is the absolute path
  `/Users/nelson/code/d2-rs/.reference/go-mod/pkg/mod`; do not repopulate the
  older `.reference/go-mod` module layout. Once a downloaded module archive is
  verified against every retained expanded member, its redundant ZIP may be
  retired with checksums and a replacement record. Keep required checksum metadata.
  Retain only the current required WASM build profile: after publishing and
  qualifying the release package, retire obsolete WASM diagnostic build outputs
  unless an active investigation requires them. Record the rebuild command;
  preserve the published package, pinned inputs and unresolved failure witnesses.
- Retention is bounded: keep the latest qualified checkpoint, current
  investigation, and compact regression evidence. Reproducible evidence means
  pinned inputs/source revisions, build/run scripts, checksums, result inventories
  and minimal source/native witnesses needed to reproduce findings; it does
  **not** require retaining every historical binary, full trace, source copy,
  successful replay output or rendered capture forever. Retire superseded bulk
  artifacts once that reproduction record is verified. Preserve unresolved
  failure witnesses and user deliverables. Record retired paths and their
  replacement reproduction records so historical ledgers do not falsely imply
  deleted artifacts are still available.
- Cleanup requests must end with an actual deletion ledger and a fresh disk
  audit, even when already below the ceilings. Remove idle Go action caches
  and superseded Cargo variants after checking active processes and retained
  corpus bindings. Remove verified temporary compression backups immediately
  after validation; do not accumulate cleanup backups as new permanent bulk.
- Do not carry every historical executable into cumulative corpus manifests.
  Source-freeze manifests and their content-addressed stores must exclude
  generated native/WASM executables and package build outputs. Bind current
  deliverables separately; retain historical hashes and rebuild instructions
  instead of freezing another binary with each source checkpoint.
  After a checkpoint is superseded, retain its inputs, frozen source, build/run
  instructions, expected outputs and hashes; retire its redundant executables.
  Keep an explicit retirement overlay for historical bindings, and use the
  verified remaining input/output closure as the next checkpoint's retention
  baseline. Check old `.reference/` binaries and Cargo variants during cleanup,
  even when the checkout is already below its ceilings.
- Give each capture run an explicit output budget before launching it. Defaults
  are **1 GiB total and 128 MiB per trace file**, with a streaming size/free-space
  guard. On exhaustion, stop the capture and record a budget-exceeded failure;
  narrow the investigation and rerun. Never count truncated or missing output
  as passing parity, and never expand retention indefinitely to avoid narrowing
  a diagnostic question. Compression/deduplication supplement bounded retention;
  they are not substitutes for it.
- Before retiring corpus artifacts, follow all retained input/output binding
  manifests, including paths used as JSON keys and generated-input folders such
  as `layout-inputs` and `engine-inputs`. Preserve that dependency closure.
  After cleanup, verify existence and original hashes of every input and
  expected output in the latest qualified cohorts, in addition to compiled
  source snapshots. A preserved result ledger alone does not prove a runnable
  corpus. Record any exact-hash reconstruction or reinstatement explicitly.
- Use the Applesauce skill for retained test artifacts on this Mac: completed
  traces, replay outputs, source/native test captures, and idle test executables
  that still need to be kept. Remove obsolete artifacts first; then prefer
  verified LZFSE transparent compression over leaving eligible retained files
  uncompressed. Follow the skill's inspection, backup, checksum/mode validation
  and rollback requirements. Never rewrite an active file or silently break
  existing hard links. Avoid recompressing already-compressed files/media.
  Compression must preserve test bytes and executability, and does not relax
  either the apparent-size budget or the retention rules above.
- End artifact-heavy work with a compact disk report: whole-checkout allocated
  and apparent sizes, `.reference/` and `target/` sizes, free space, and actual
  cleanup savings. Keep cleanup within this project; do not move the same bulk
  into sibling directories or `/tmp` to make the reported size smaller.
- External layout-plugin discovery and the `d2plugin-*` protocol are explicitly
  out of scope for this project. Do not add, debug, or count plugin support as a
  completion gate; current and future work targets the pinned D2's in-process
  engines and APIs (upstream master has removed `d2plugin` entirely). Existing plugin adapter code may remain as historical
  scaffolding until it is removed deliberately, but it is not a release
  requirement.

- Content-addressed retention objects are immutable. Replay/test writers must
  use fresh writable output paths; never write through a deduplicated symlink or
  hard link into the content store. Before reusing an output path, detach its
  alias and explicitly retire its previous binding. Preserve negative baselines.
  Verify latest aliases and historical retirement overlays separately at sealing;
  do not assume that every historical replay path still denotes its old bytes.
