# Provenance and distribution

This doc states what d2-rs is ported from, against which upstream revision,
and what any distribution of it must carry.

## What d2-rs ports

d2-rs is a Rust translation of upstream Go D2 (Copyright 2022 Terrastruct
Inc., MPL-2.0) at the commit pinned in `reference-manifest.json`: `master` at
6a7b48378 (2026-10-11). That includes TALA, the layout engine bundled in D2
under `d2layouts/d2talalayout`, which upstream ships under D2's MPL-2.0
license (merged in d2lang/d2#2882). Translations follow the pinned commit
and the Go module versions in the manifest's `dependencies`.

Beyond D2 itself, d2-rs ports the Go standard-library code and Go libraries
that D2's output depends on (`strconv`, `sort`, `compress/flate`, `goldmark`,
`chroma`, `mathjax-go`, `elk-go`, `dagro`, `rough-go` and others; the full
list is in the README). Each keeps its upstream license notice in
`THIRD_PARTY_NOTICES.md`. Go and JS builds of D2 are test oracles only: no Go
code is linked into d2-rs.

## Copied assets

Some files are copied unchanged from the pinned D2, for example the SVG
renderer's pattern assets from `d2renderers/internal/patternassets/`:

```text
crates/d2-rs-render/assets/grain.png        7aec37844fa033cbdb586efaa1955dc6f43d56707c0b1bf91e8b5e32386d8d69
crates/d2-rs-render/assets/paper.txt.br     fe047f4104975b17b033988e49981c42a5aca928da39d149e35f4e31c8fb248a
crates/d2-rs-render/assets/streak_path.txt 7b4001e7d043ac395abd015ce0bc2bbe018725a634f6e352b1eb2ff60cc99dea
```

`crates/d2-rs-render/assets/paper.svg` (sha256 `98b3c110…53527`) is the
deterministic decompression of `paper.txt.br`. Other copied D2 assets, such
as the PPTX template and the watch-mode client, are listed in
`THIRD_PARTY_NOTICES.md`.

## Notices

Keep upstream author notices and license terms when moving code, fonts,
fixtures or generated tables. Inherited notices stay in force for inherited
code, including Go's BSD license and the Apache-licensed Fibonacci heap.
Before replacing an implementation and removing its notice, check where the
replacement came from.

## Distribution

Every distributed crate, binary archive and WASM package must include:

- the MPL-2.0 text (`LICENSE`);
- `THIRD_PARTY_NOTICES.md`, and the Cargo dependency notices that
  `node scripts/sync-licenses.mjs` generates into `DEPENDENCY_NOTICES.md`
  (not committed);
- the exact source revision it was built from.

The corresponding source is this repository at that revision; it contains
every modification needed to build the artifact. Upstream references are
not stored here but are acquired reproducibly from the manifest by
`scripts/references.py` (`docs/REFERENCES.md`), so they cannot hide
modifications. See `SOURCE.md`.
