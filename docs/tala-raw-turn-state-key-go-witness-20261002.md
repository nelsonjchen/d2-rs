<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

Original Go's actual private `placementEdgeLengthState` hashes raw Turn
independently of EdgeLength cache occupancy. It never materializes a cost.
Four controls, each with repeated raw observation disabled/enabled, pass all
eight runs against pristine D2 v0.9.0 with pinned Go 1.27.0.

The actual engine-stage control calls `pipeline.gapNormalizationStage` on two
fixed 100×100 nodes at (1000,1000) and (2000,1000). It returns raw Turn 0 while
retaining two EdgeLength cache entries. The actual key changes after an
explicit `Graph.TurnCost()` reader produces 112.5; cache occupancy remains 2.
Fresh graphs prove the complementary case: an actual 112.5 turn cache is
included in the key even while the EdgeLength map is empty. Neither key
construction nor its actual cache lookup changes raw costs, map size or
geometry, including two additional repeated reads per observer-on checkpoint.

| Actual control | Before key | After actual TurnCost reader | Map entries before/after reader |
|---|---|---|---|
| NormalizeGaps→explicit reader | `0x4b325f7b9076c29b` | `0xf7da59555443cec7` | 2 / 2 |
| NormalizeGaps→actual EdgeLength score→reader | `0x4b325f7b9076c29b` | `0xf7da59555443cec7` | 2 / 3 |
| Fresh, actual turn already cached | `0xa82b96a70ccd1842` | `0xa82b96a70ccd1842` | 0 / 0 |
| Fresh, turn not yet cached | `0x3d10d8e538de630e` | `0xa82b96a70ccd1842` | 0 / 0 |

In the second control, the intervening actual EdgeLength call constructs its
key from raw Turn 0, computes 1800 (`0x409c200000000000`), materializes Turn 112.5
while scoring and stores the result under the preceding raw-zero key. This
creates the third entry. In the explicit-reader control, the raw-zero key
misses; the post-reader key already hits an 1800 entry from the actual stage.
The final actual score is 1800 in all controls. These observations record the
real key, getter, scorer and cache lookups rather than a reimplemented hash or
manual cache population.

Golden key inputs are complete and captured from live source objects:

- Graph.Nodes order is [1,2], Graph.Edges order is [0], and each node's actual
  Node.Edges order is [0]. The source constructors retain edge ID 0; no identity
  is normalized or replaced.
- CellSize is 100, bits `0x4059000000000000`. Both widths/heights use those same
  bits. Node1 is at (1000,1000), both coordinates `0x408f400000000000`; node2 is
  at (2000,1000), X `0x409f400000000000`, Y `0x408f400000000000`.
- Options are IncludeNodeSizes=true, EnforceMinimumGap=false,
  PenalizeDirection=false, EdgeAbductions empty, giving the original flags
  byte 1.
- Turn 112.5 has bits `0x405c200000000000`. NonCenterPort is 0 in every key. Fresh
  controls have Crossing 0; stage controls have actual cached Crossing bits
  `0x4058e219652bd3c3`. The source key writes CellSize, Turn, NonCenterPort and
  Crossing in that order before node/incident-edge records.

`.reference/tala-turn-key-witness-20261002/golden-raw-turn-key-inputs.json`
contains the full eight before/after records, decimal and hexadecimal u64
keys, exact geometry/cost bits, incident orders and actual lookup results.
FixedTopLeft and Graph.Edges are provided as context; the original key consumes
the node/incident inventory rather than using Graph.Edges or cache occupancy
as hash input. This supports a native raw-key contract test with reconstructed
source identities and explicitly restored raw costs, without claiming native
stage cache population. The native test build's restored-score cache bypass
cannot establish production cache occupancy.

Original source is commit `d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`; all 4,495
files are frozen and rechecked. The implementation and pipeline are unchanged.
A tiny virtual placementcost bridge calls its unchanged private function;
the caller test lives in engine because placement imports placementcost and
an internal placementcost test cannot import the real stage without a cycle.
Both additions exist only in this exclusive overlay.

The actual retained Go build freezes 322 exact linker archives and 4,058
source/assembly/embed input records across 324 listed packages. 321 archives
match the package export objects byte-for-byte. The generated actual test-main
archive is bound directly to its recorded compiler command, generated source,
import configuration and pinned compiler; the Go-list export has a different
build context despite identical generated source. The initial strict
export-only closure audit is preserved, along with the initial passing
runtime capture before incident-edge observations were added. External macOS
SDK/system libraries are identified by the build environment/logs and are not
copied as source.

The [qualification manifest](tala-raw-turn-state-key-go-witness-20261002.json)
records all source, object, toolchain, runtime and golden-input hashes. This
qualifies only the original-Go raw turn-key/getter/cache contract. No Rust
implementation was edited or executed, and crossing/non-center-port lazy
policy, complete pipeline trace and full parity remain outside this evidence.
