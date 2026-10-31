<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

# Original-Go Equidistance prelude witness

The natural three-node control executes the actual original D2 v0.9.0 wrappers. Two 100x101 members at (1000,1000)/(1120,1000) have identical outgoing edges to a 100x100 peer at (1800,1000). Actual AddClusters intrinsically produces Row 220x101 with padding 20. Actual OptimizeClusters computes cell 220 and commits a real Column 100x222 flip. Actual Equidistance then recomputes cell 222 before its first work. No arrangement, vessel dimensions or cell value is forced.

Five original-Go controls and ten observer off/on runs passed: natural stale220-to222, already-current222, one-node100-to20 after width change, unchanged100 and empty0-to10. A raw-state hook immediately after the actual Equidistance computation leaves cached costs and entry counts unchanged; geometry, cell, costs, cache entry counts, changed gate and errors match exactly across observer modes. This proof is Go-only. The natural graph has no further Equidistance move, isolating the wrapper prelude.

The executed unit-test binary embeds Go 1.27.0. Its virtual overlay uses a fresh verified copy of 4,495 pinned pristine source files and adds only the raw-state hook and unique test. Removing the hook restores the exact original pipeline source. The original checkout remains pristine.

Native source remains unchanged. Private proposals extract the actual production Equidistance wrapper, share it with five prepared cell tests, and add the missing single computation only in a separate AFTER patch. Expected BEFORE failures are the natural220-to222, one-node100-to20 and empty0-to10 controls; neither proposal has been installed or run. Native tests call real automatic grouping and the existing production OptimizeClusters wrapper. If that preparation fails, preserve the earlier failure instead of forcing cluster geometry. No native cache/geometry/trace, authored full-NodePlacement, seed-race or full TALA/D2 parity claim follows.
