# Raster artifact gate qualification — 2026-10-05

The earlier absent-PNG false pass belonged to historical ad hoc captures. Those
scripts and their reports remain unchanged as evidence; the current broad runner
already rejects absent output. The durable raster checker had only two tests for
pixel-difference reporting and accepted some identically malformed PNGs.

`scripts/check-render-parity.py` now exposes a reusable complete-output gate:

- Successful runs require a real single PNG or a nonempty board directory.
- All relative board names, dimensions and complete decoded RGB pixels match.
- Missing, extra or renamed boards, ambiguous singleton-plus-board output, and
  unexpected non-PNG artifacts fail.
- Decoding validates chunk boundaries/order/CRC, IEND, zlib completeness and
  exact scanline lengths. Unsupported RGB transparency is rejected explicitly.
- Nonzero exits, timeouts and execution errors fail even if partial output exists.
- Both CLI renderers write into fresh, isolated output directories, supporting
  actual multiboard output. An empty comparison suite cannot pass.

Validation: 24 focused tests pass (22 new artifact/CLI contracts and the two
existing pixel-difference tests). They use tiny PNGs, temporary directories and
mocked renderer processes while exercising the real decoder, output collection,
comparison, JSON reporting and command exit status. No D2 build or full parity
suite was run for this harness change.

The corruption regression fails against the original checker: two PNGs without
IEND were reported as matched. Deliberately restoring empty-inventory success
also fails the missing-output test. Mutating either horizontal predictor to zero
fails the hand-encoded 2x2 filter test, including the upper-left Paeth branch.

GitHub CI now discovers `test_render_*.py` in a separate lightweight job. Its
remote execution is not claimed here. AGENTS.md requires future raster captures
to reuse this gate or explicitly test an equivalent one. This checkpoint changes
verification infrastructure; it does not increase any recorded D2 parity count.
