# Provenance and distribution

The initial Rust layout implementation and measured-graph adapter were copied
from archived Weftan at the commit recorded in `reference-manifest.json`.
Weftan originated in reconstructed TALA implementation work using release
artifacts, disassembly and debugger evidence, then compared against OSS TALA.
This history is not a clean-room claim or a claim that the initial translation
originated from OSS source.

New translations use D2 v0.9.0 at the pinned commit and its pinned dependencies.
The source/translation ledger records which parts have been ported and tested.
Both reference checkouts live beside this repository under `~/code`; release
assets and dependency copies live in ignored `.reference/` storage.

Preserve upstream author notices and license terms when moving code, fonts,
fixtures or generated tables. The inherited third-party notices remain in force
for inherited code, including Go BSD and the Apache-licensed heap implementation.
Replacing an implementation requires reviewing its source provenance before
removing any notice. Dependency notices must accompany their eventual ports.

Every distributed crate, binary archive and WASM package must include the MPL
text, applicable notices and instructions identifying the exact corresponding
source revision. Until a public source URL exists, do not publish packages.
The source distribution must contain all modifications needed to build the
distributed artifact; ignored references are reproducibly acquired through the
manifest and reference script and cannot hide modifications.
