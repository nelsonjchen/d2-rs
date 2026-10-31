# Retired oracle fixtures: math, text, core, editor, layout, vendor

Fixtures removed without a recipe, per AGENTS.md ("Retire fixtures that pin
unreachable internal states ... with a written reason"). Their data remains
at base commit `9b706ac1` and in the private `nelsonjchen/d2-rs-history`
repository (`git show 9b706ac1:<path>`).

## `crates/d2-rs-core/tests/jpeg-cancel-go-v090.json` (10.3 MB)

- Test: `jpeg_first::tests::source_cancel_and_deadline_sweep_preserves_every_poll_read_and_color`
  (removed with its Spy/project helpers).
- Reason: it cancels D2's `internal/rasterimage` first-frame JPEG decode at
  every poll index of a context-wrapped reader and pins the per-poll `Polls`
  and `Events` log. That is internal poll accounting, not reachable from D2
  input or a public API. Cancellation outcomes stay covered by the
  cancelled/deadline modes of the jpeg-orientation and image-config recipes;
  decoded pixels by `jpeg-frame-go127`.

## `crates/d2-rs-core/tests/jpeg-entropy-go127.json` (2.9 MB)

- Tests: `jpeg_entropy::tests::source_tables_preserve_partial_reads_and_error_precedence`,
  `jpeg_entropy::tests::source_symbol_eof_marker_lookahead_and_undo_states`.
- Reason: they pin Go 1.27 standard-library `image/jpeg` private state: the
  Huffman lookup-table layout and the bit/byte reader's accumulator, mask,
  bit count, buffer indices and unread count after each call. No D2 input or
  public API observes these. DHT parsing, Huffman decoding and their errors
  stay covered end to end by the `jpeg-frame-go127` full decodes
  (`tools/fixtures/mathtext-img-stdlib.toml`).

## `crates/d2-rs-math/tests/fixtures/svg-viewport-v090.json` (0.8 MB)

- Tests: none (it had no Rust reader).
- Reason: superseded. It held the v0.9.0 viewport/aspect outputs for the same
  2,280 inputs as `svg-viewport-master.json`, and was only the input list of
  `tools/scenebuild-svgimport-scoreboard/viewport.go` (also removed). The
  `mathtext-img-svg-viewport` recipe now generates those inputs and the master
  outputs.

## Second pass (files under 256 KB)

Every other small oracle file of the area became a recipe output
(`tools/fixtures/mathtext-{latex,svg,img,txt,core}.toml`). Three were deleted
without a recipe of their own:

### `crates/d2-rs-editor/tests/fixtures/rename-v090.json` (13 KB) and `delete-set-v090.json` (102 KB)

- Tests: `tests/rename.rs::upstream_rename_text_and_errors`,
  `tests/delete_set.rs::upstream_{delete,set}_text_and_errors`.
- Reason: duplicates. Both held the live TestRename / TestDelete / TestSet
  requests recorded from upstream `d2oracle/edit_test.go`, which the existing
  `mathtext-txt-editor-graphs` recipe already records (same request fields,
  plus the upstream graph golden) in `editor-graphs-v090.json`. The two tests
  now read that artifact and filter by `request.operation`, keeping their 25
  and 122 + 102 case-count assertions.

### `crates/d2-rs-editor/tests/fixtures/move-reference-v090.json` (40 KB)

- Tests: none (orphan; no Rust or script reader).
- Reason: its 92 reference-move probes are the "92 reference probes" block of
  `move-transplant-v090.json`, which the `mathtext-txt-editor-move-transplant`
  recipe regenerates and `tests/move_references.rs` reads.

### Reader and input notes

- `padded-source-v090.json` and `commands-source-v090.json` are now plain
  d2latex hook rows (with `svg`). `padded.rs` compares the Go root's
  `width`/`height`/`viewBox`/`style` attributes itself instead of a
  pre-extracted `root` map; the `commands` test classifies a name as
  recognized unless its Go render's only `data-mjx-error` is
  `Undefined control sequence \<name>`, the rule of
  `scripts/generate-math-command-registry.py`.
- `chroma-plaintext-extension-source-v090.json` is produced by
  `tools/mathtext-chroma-oracle tokens`, so rows carry `styles` instead of the
  unread `lexer` field.
- `go-float-v127.json`: the 55 syntax controls are the v0.9.0 ones; the 1,000
  random hexadecimal/decimal floats are redrawn from a fixed-seed PRNG of the
  same shape (`scripts/fixture-gen-go-float.py`), so their values differ.
- `update-import-v090.json`: the test no longer asserts the v0.9.0
  `upstream_commit` metadata string; it checks every recorded case.
- `master-markdown-substitutions.json` (determinism): the hook's
  `time.Until(deadline)` is recorded rounded to whole seconds, the configured
  `context.WithTimeout` of the upstream tests, so two runs give identical bytes.
