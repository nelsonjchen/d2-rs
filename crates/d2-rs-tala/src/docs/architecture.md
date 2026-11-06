# Engine architecture

TALA is a staged layout engine. It deliberately does not optimize one global
formula from input to output. Each stage owns a narrower decision once the
facts it needs are available.

## Workspace boundaries

```text
D2 integration
   │ serialized graph JSON (historical adapter only)
   ▼
d2-rs-lib              lossless serialized-graph adapter
   │
   ▼
d2-rs-tala             in-process layout and routing
```

The core crate does not know D2's complete document schema. The adapter owns
shape, label, icon, fixed-position, and serialization conversion. External
plugin process discovery is outside scope. This separation lets Rust
applications use [`crate::Graph`] directly without constructing serialized D2
JSON.

> **Working mental model:** recognize structure, place boxes, route paths,
> place typography, then select a complete candidate.

The diagram below groups the production pipeline into its major information
boundaries. Arrows carry owned geometry forward; later stages do not mutate the
caller's [`crate::Graph`].
