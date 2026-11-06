# Upstream-derived test inputs

The 27 upstream end-to-end D2 inputs that used to be copied here
(`tests/upstream/d2`, with their checksum inventory `manifest.json`) are no
longer committed. They were byte-identical to `e2etests/testdata/files` at
the pin, so generators read them from the pinned tree instead:
`scripts/upstream_inputs.py` lists them and resolves them against the
fixture runner's patched upstream tree (`$D2RS_UPSTREAM`) or the pristine
checkout named by `reference-manifest.json`. A test that needs one of them
reads a recipe copy (for example recipe `lang-upstream-e2e-input`, see
docs/FIXTURES.md). `scripts/audit-engine-fixtures.py` enumerates the pinned
TALA internal testdata directly. The data of the removed copies remains at
base commit `9b706ac1` (docs/fixture-retirements/langlibelk.md).

`tala-fuzz-expanded/` holds 24 locally chosen `FuzzAutolayout` tuples; they
are hand-written test inputs, not upstream corpus entries.

The boundary harness takes any input directory, for example:

```sh
uv run --no-project python scripts/check-boundaries.py --layout-only --fixtures-dir ../d2/e2etests/testdata/files --output oracle-cache/reports/upstream-layout-baseline.json
```
