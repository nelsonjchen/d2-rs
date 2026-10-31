
## Field ownership

The adapter updates geometry that belongs to layout:

- object `box.TopLeft` coordinates and fitted dimensions;
- edge `route` point arrays and the non-curve marker;
- supported node and edge label positions, sizes, and percentages;
- geometry derived from labels, icons, shapes, tables, and modifiers.

Other members remain in the original JSON tree. This includes application data
and newer protocol fields unknown to this crate.

## Stable identity

Serialized objects carry a local `id` as well as their absolute `AbsID`.
D2 object `AbsID` strings are decoded into a table from external identity to
[`d2_tala_rs::NodeId`]. `ChildrenArray` establishes container ownership. Edges are
kept in serialized order and map to stable [`d2_tala_rs::EdgeId`] values.

```text
"AbsID": "service.api" ──► NodeId(4)
"ChildrenArray": ["service.api"]
                           └──► Node.parent = Some(service NodeId)
"Src": "service.api", "Dst": "db"
                           └──► Edge { source: NodeId(4), target: db_id }
```

Typed record errors are reported before layout. Raw graph topology and identity
validation have separate checks; their malformed-input behavior is not yet
fully matched to Go.

## Numeric serialization

D2's Go protocol has observable floating-point formatting behavior. The
adapter preserves signed zero and uses Go-compatible floating-point formatting.
Record defaults, field omission, and complete object and edge inventories are
checked independently of number formatting.

## Source-only arrows

D2 can represent an edge with only a source arrow. Standalone routing reverses
the internal point list when required so the serialized route and label
percentage remain consistent with D2's visual direction. Label positions use
[`d2_tala_rs::LabelPosition::mirrored`] during that transformation.
