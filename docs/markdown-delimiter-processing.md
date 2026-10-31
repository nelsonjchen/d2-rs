# Delimiter processing translation contract

Pinned source: Goldmark v1.8.5 `parser/delimiter.go` and
`parser/parser.go`, reached through D2 v0.9.0. Scanning and the scalar
consumption methods and delimiter context push/remove/clear operations are
translated. The processing loop is translated for the emphasis and
strikethrough processors used by D2; full inline parser dispatch remains pending.

`CalcComsumption` (upstream spelling) uses **original** lengths for its modulo
three rule and **remaining** lengths to choose one or two characters. The
modulo restriction applies when the opener can also close or the closer can
also open, and only when the closer's original length is not divisible by
three. `ConsumeCharacters` changes remaining length and segment stop, retains
original length, and clears ForceNewline through `WithStop`.

The oracle now compares 5,060 combinations of original/remaining lengths,
opening/closing flags and consumption counts, before and after mutation.
Negative and over-consuming counts are direct method boundary observations,
not claims that ordinary Markdown parsing reaches those states.

The node arena now provides stable identities and qualified AST child mutations.
Context integration now retains two distinct structures:

- AST parent/previous-sibling/next-sibling links.
- Delimiter previous/next links plus context first/last pointers.

`ProcessDelimiters` searches potential openers through delimiter links. A
successful match consumes both ends, creates the processor's node, sets its
position from the opener, moves the AST siblings between the pair into that
node, then inserts it after the opener. It removes intervening delimiters from
the context and removes exhausted endpoints. A partially consumed closer is
revisited rather than advanced immediately.

`RemoveDelimiter` repairs delimiter links and context endpoints, then clears
the removed node's delimiter links. Nonempty remnants are converted through
`MergeOrReplaceTextSegment`; exhausted nodes are removed from the AST. The
already translated text merging rules therefore affect this processing step.

Two source details must remain explicit in the implementation:

- When a bottom boundary is supplied, initial closer selection walks backward
  through **AST siblings**, inspecting which siblings are delimiters.
- `ClearDelimiters` starts from the context's last delimiter, but continues
  through each node's **previous AST sibling**, saving it before removal.

A single vector of delimiter records cannot stand in for both structures once
nodes have moved into an emphasis node. Qualification must inspect resulting
AST shape, surviving node identities, both link sets, context endpoints and
bottom-boundary behavior. Existing owned inline snapshots remain useful as
oracle output, but do not establish identity or linked-mutation parity.

The 756-case context corpus captures stable allocation identities for text
remnants through an observer-only NewTextSegment hook. It compares both link
structures and stored counts, including nested parents and bottom boundaries.
See `markdown-context-parity.json`; the matching loop adds 1,295 further cases for a total of 2,051 context
sequences. Comparisons capture state after each requested operation, not a full
trace of every internal loop operation. The oracle processors reproduce the
pinned emphasis/strikethrough callbacks and call the original constructors.
Directly supplied delimiter records include method boundary cases; they are not
all claimed reachable through ordinary Markdown syntax.
