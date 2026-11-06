# Source

This is Rustybuzz 0.20.1, copied from its pinned Cargo registry package and
modified so that text shaping matches `go-text/typesetting` v0.3.4, the
shaper upstream D2 uses.

- **Licenses:** Rustybuzz and HarfBuzz remain under the MIT license in
  `LICENSE`. Code that follows go-text's Go source keeps its copyright
  (2021, the go-text authors) and its Unlicense OR BSD-3-Clause notice in
  `GO_TEXT_LICENSE`; go-text's HarfBuzz port notice is in
  `GO_HARFBUZZ_LICENSE`.
- **Changes from Rustybuzz:** GPOS pair adjustment, coverage digests and
  Unicode property tables (Unicode 17.0.0, script tags, combining classes,
  mirroring, composition exclusions) follow go-text's behaviour, including
  its quirks, so shaped output matches Go byte for byte. The `shape`
  example builds on platforms where `c_char` is unsigned.
