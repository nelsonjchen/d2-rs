# Identity regression inputs

Source-inspired cases for pinned D2 v0.9.0. `collisions.d2` combines the
object collision names from `d2talalayout/adapter_test.go` and edge collision
names from `d2talalayout/transaction_test.go`. `nested.d2` exercises upstream
`d2graph.Edge.AbsID` common ancestry and quoted dotted object IDs. These are
new D2 inputs, not copied golden results. Differential reports determine parity.
