# Pinned D2 language translation

Evidence JSON cited below as `docs/*.json` is no longer in the tree. It was
retired at 9b706ac1 and in the 2026-10-31 second pass; `docs/RETIRED-EVIDENCE.md`
lists each file and how to read it back. Checkers now write fresh reports
under the gitignored `oracle-cache/reports/`.

Reference: D2 v0.9.0, commit
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`. Core parser productions and the
public byte/stream Parse boundaries have substantial exact-match evidence, but
that evidence is corpus-scoped; parser fuzz/resource-limit qualification,
compilation, and editing remain incomplete. Source-position primitives are
qualified separately by the existing 88-case boundary report.

The public parser wrapper matches **1,229/1,229** cases, streaming Parse
matches **3,742/3,742**, and the pinned v0.9.0 upstream `TestParse` inline
corpus matches **45/45** through the public API. The 45 inline results also
run in the native Rust test suite. Reports and reproducible checks are
`docs/public-parser-parity.json`, `docs/stream-parser-parity.json`, and
`docs/upstream-parser-parity.json`.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --release --example parse-oracle
uv run --no-project python scripts/check-upstream-parser-corpus.py
cargo test -p d2-rs-language
```

## Keyword rules

`d2ast/keywords.go` maps to `d2-rs-core::ast::keywords`. It provides the simple,
holder, composite, style and board keyword inventories; the complete reserved
membership query; ordered node-near, label/icon and tooltip vocabularies; fill
patterns and text transforms; and the label-position mapping. Composite names
include holders and boards after upstream initialization. Matching is exact;
compiler case handling is a separate, pending concern.

`docs/keyword-parity.json` compares 12 complete inventories with the actual
initialized Go globals. Sets are sorted because their iteration order is not
part of this inventory comparison; source arrays retain their exact order.
All groups and mappings match. The two Rust regressions cover category
boundaries, case sensitivity and position spellings that must be rejected.
This does not qualify parser/compiler behavior or map traversal in algorithms.

Reproduce:

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example keywords-oracle
uv run --no-project python scripts/check-keywords.py
```

The Go oracle remains development-only. Native keyword queries use Rust
constant tables and the shared `LabelPosition` enum, with no new dependency.

## Initial leaf nodes and shared errors

`ast/nodes.rs` translates Comment, BlockComment, Null, Suspension and Boolean,
plus `d2ast.Error`. Separate Node, MapNode, ArrayNode, Value and Scalar traits
preserve the permitted node categories. Type and scalar strings can be borrowed
or owned to accommodate future dynamically named block strings and interpolated
values. These leaves borrow static strings and have no children.

Position and Range serialization uses the upstream text representation. The
shared diagnostic serializes `range` and `errmsg`; display returns the message
alone. Suspension's scalar string is empty independently of its boolean value.

`docs/ast-leaf-parity.json` matches 16 direct Go cases covering all implemented
leaves, both boolean states, zero/nonzero ranges, Unicode/comma paths and shared
errors. It compares complete JSON values, type names, range display, scalar
strings and child counts. Rust empty child vectors model leaf traversal, not
a separately serialized Go nil slice. Deserialization, rational Number values,
string/interpolation nodes, composite AST nodes and parser behavior remain
unimplemented at that checkpoint.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example ast-leaves-oracle
uv run --no-project python scripts/check-ast-leaves.py
```

## Single-quoted and block strings

SingleQuotedString and BlockString now implement scalar and string operations.
`set_string` clears SingleQuotedString.raw, while BlockString retains its quote
delimiter and tag. Both report `is_unquoted == false`. BlockString's type name
is the tag followed by ` block string`, including the leading space for an empty
tag. Cloning these records preserves their values independently.

The expanded leaf oracle now matches 26 cases, including original/edited copies,
multiline Unicode replacement values, empty tags and complete serialized fields.
Unquoted/double-quoted interpolation nodes, rational numbers, composite nodes,
deserialization and parsing remain pending. Native scalar/type strings use
borrowed values where possible; block type names are constructed from the tag.

## Interpolation ownership audit

`docs/ast-interpolation-reference.json` captures 13 Go-only cases. No Rust
interpolation implementation is qualified by this report. Reproduce with
`uv run --no-project python scripts/capture-ast-interpolation.py` after rebuilding
the development oracle.

Observed contracts that determine the next representation work:

- Nil and empty interpolation slices serialize differently before coalescing.
- ScalarString returns only the first literal, not all literal segments.
- Coalesce concatenates consecutive literal-bearing boxes until the first box
  without a literal, then replaces the list with one literal, even when empty.
- SetString/Coalesce preserve UnquotedString.Pattern while discarding old
  interpolation/raw-spelling entries.
- A box can contain both String and Substitution. Its substitution is a child,
  and its string still participates in coalescing. An exclusive enum cannot
  represent this source state.
- Copy shares the interpolation slice backing storage. Mutating a copied
  element changes the original; SetString replaces only that copy's list.

The dependencies are recursive: interpolated strings contain substitution
boxes; substitutions contain nullable StringBoxes; StringBox can contain any
string node. StringBox.Unbox selects unquoted, double-quoted, single-quoted,
then block strings in that order when more than one field is present.
Before implementing these nodes, the Rust child-access interface must support
shared AST ownership without invalid borrowed references or silent deep-copy
semantics. This is a concrete remaining interface issue, not a passing test.

## Interpolation translation

`ast/interpolation.rs` now represents unquoted/double-quoted strings,
InterpolationBox, StringBox and substitution records. Optional shared slices
preserve nil/empty serialization. Their fixed-length headers share element
slots on copy, while SetString installs new storage. Literal strings and child
records also retain shared identity. This is not a general Go append/capacity
emulator; parser construction and other slice operations remain separate work.

AST traversal now returns owned `Child` handles and copies source ranges out
of shared records. It cannot leave references pointing into a released storage
borrow. StringBox selection and substitution traversal follow source precedence
and skip nil/empty boxes. Mixed literal/substitution boxes remain representable.

All 13 captured cases now match in `docs/ast-interpolation-parity.json`.
Three additional regressions check shared-slot edits versus list replacement,
child lifetime/identity after coalescing and parent drop, and nullable path
traversal/box precedence. The earlier reference-only report is retained as
historical evidence; it is superseded by the parity report for these cases.
Numbers, imports/key-path helpers, composite nodes, deserialization, parser
behavior and broader ownership/cycle handling remain pending.

```sh
cargo build -p d2-rs-lib --example ast-interpolation-oracle
uv run --no-project python scripts/check-ast-interpolation.py
```

## Raw string representation

`raw_string_box` implements d2ast.RawStringBox/RawString selection using the
native shared string records. It retains distinct key/value special-character
sets, reserved-value handling, quote/newline precedence and boundary whitespace
rules. The source's actual trailing-hyphen rule is preserved even though the
nearby source comment describes only doubled hyphens. Unicode whitespace uses
the pinned Go White_Space ranges rather than broader trimming conventions.

`docs/raw-string-parity.json` verifies 774 key/value inputs against complete
Go StringBox JSON: targeted punctuation/whitespace cases and deterministic
generated combinations. All match. Invalid UTF-8 source bytes, parsing and
formatted output are outside this comparison.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example raw-strings-oracle
uv run --no-project python scripts/check-raw-strings.py
```

## Key paths

KeyPath now supports construction from text and string-node scalar values,
child traversal, ID extraction, last-box access, copying, scalar equality and
glob inspection. Unlike interpolation Copy, KeyPath.Copy allocates new path
slots but retains box identities. Copying an empty path produces a nil path.
Glob queries inspect unquoted parsed patterns, including the exact double/triple
glob encodings, rather than literal asterisk text.

`docs/key-path-parity.json` matches 11 Go cases, including nil/empty paths,
re-quoting existing strings, parsed-pattern variants and copy mutations.
Two regressions distinguish copied slots from shared boxes and parsed globs
from quoted asterisks. Malformed pointer/panic behavior and nullable receiver
APIs are not qualified; Rust callers represent absent paths with Option.
Parser, import/edge construction and broader AST integration remain pending.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example key-paths-oracle
uv run --no-project python scripts/check-key-paths.py
```

## Edges, indices and traversal

`ast/edges.rs` represents edge endpoints/arrows and nullable integer/glob
indices. Edge equality compares endpoint scalar paths and arrow strings in
source order, ignoring ranges. EdgeIndex equality deliberately uses shared
integer identity rather than numeric equality, preserving the upstream TODO's
current behavior. Null integer pointers compare equal only when glob flags also
agree. Source-order child traversal skips absent endpoints.

`walk` visits nodes in preorder and prunes only the subtree whose callback
returns false. The oracle report `docs/ast-edge-parity.json` matches complete
edge/index JSON, ordinary and pruned traversal, range-insensitive edge equality,
arrow changes and all 49 pairwise comparisons among seven index records.
Malformed pointer/panic behavior and deep/cyclic traversal are not qualified;
parser/compiler integration remains pending.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example ast-edges-oracle
uv run --no-project python scripts/check-ast-edges.py
```

## Import records and paths

`ast/imports.rs` adds import records, ordered child traversal, imported-identifier
extraction, PathWithPre/Dir and substitution IDA. The first import path component
names the file; IDA returns the subsequent components. A missing path yields an
empty PathWithPre even with a prefix, but is invalid for import IDA.

Path reduction follows Go's lexical slash semantics on every host. Later
absolute-looking components do not replace the prefix, backslashes are ordinary
characters, rooted paths discard excess parent segments and empty directory
results become `.`. `docs/import-path-parity.json` matches six direct source
cases including full import JSON and child/identifier counts. The compiler
import path stack, cycles, in-memory byte cache and read counts match 237
operations in `docs/import-loading-parity.json`; 96 import-context cases match
clone isolation and reference remapping. The end-to-end compiler matrix matches
347/357 imported compilations and all 3,472 successful ordered trace events.
Its five source panic cases and two AST-projection panics are recorded
separately, and three missing-key diagnostics include Go's unstable pointer
addresses. A nested-panic regression now restores the Rust parent import stack
before resuming the panic, matching Go's deferred cleanup. Native OS file-open
errors, Windows absolute-path behavior and broader filesystem/provider
qualification remain pending.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example import-paths-tala-oracle
uv run --no-project python scripts/check-import-paths.py
```

## Number records

`ast/number.rs` translates `d2ast.Number` with nullable shared exact rational
storage. Raw source spelling remains independent, including when it disagrees
with the stored numeric value. JSON emits null or Go-compatible rational text:
reduced fractions, normalized signs and integers without a `/1` suffix.
No float conversion is used. Native arithmetic uses `num-bigint` and
`num-rational`; dependency notices are included in each crate.

`docs/ast-number-parity.json` compares 41 direct Go cases, including nil, zero,
negative denominators, reducible fractions and 200-digit integers. All match.
This qualifies record serialization and scalar spelling, not Go Rat.SetString
or D2 numeric parsing; those remain to be translated.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example ast-numbers-oracle
uv run --no-project python scripts/check-ast-numbers.py
```

## Composite AST records

`ast/composite.rs` adds Array, Map, Key, MapNodeBox, ArrayNodeBox, ValueBox and
ScalarBox. Box fields remain independent nullable pointers, preserving upstream
selection order even when several fields are present. ArrayNodeBox deliberately
has no suspension field, matching upstream. Node lists preserve nil versus
empty storage and shared element slots. Key children preserve the order of key
path, non-null edges, index, edge key, primary scalar and value.

The 222 cases in `docs/ast-composite-parity.json` qualify empty, singleton,
pairwise and all-present box fields, scalar/string projections, nested JSON and
walk order, board/filter inspection and scalar replacement on array values.
Board/filter inspection looks directly at MapKey even when Comment wins Unbox.
No JSON serialization normalization is applied beyond decoding object key order.

AST deserialization and parser/compiler integration remain pending. SetScalar's other value branches
are translated from source but still need direct oracle qualification.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example ast-composite-oracle
uv run --no-project python scripts/check-ast-composite.py
```

## Map insertion and key helpers

`ast/key_helpers.rs` translates ordinary and oracle key comparisons, glob
queries and filter support. Ordinary equality preserves upstream's asymmetric
primary-scalar check and pointer-based edge-index comparison. Oracle equality
omits primary scalars and edge contents, while nested map keys still use ordinary
equality. Neither comparison checks ranges or array contents. These are upstream
helper semantics, not whole-AST equivalence suitable for parser acceptance.

Map insertion selects the last cursor with matching node identity; absent
cursors append. Nil cursors match empty boxes. A fresh slice detaches old map
copies while retaining shared node pointers.

`docs/ast-key-helpers-parity.json` matches six insertion cases (before/after, nil,
duplicate and equal-content/different-identity cursors), 225 ordered comparisons
for each equality method, and 20 glob/location cases. Deeper recursive structures,
malformed pointers, additional filter combinations and nil-receiver qualification
remain open.

```sh
uv run --no-project python scripts/build-oracle.py
cargo build -p d2-rs-lib --example ast-key-helpers-oracle
uv run --no-project python scripts/check-ast-key-helpers.py
```

## Parser cursor and single-quoted production

The new `d2-rs-language` package introduces the rune-reader boundary, input
cursor, ordered errors and `parseSingleQuotedString` translation. Readahead
uses a Rust deque; lookahead position, EOF rewind, commits and reader-error
positions retain the upstream algorithm. Invalid UTF-8 consumes one invalid
byte per replacement rune, as Go ReadRune does. Position advancement counts
the replacement rune's encoding width, rather than the invalid input width.

`docs/single-quoted-parser-parity.json` qualifies 150 direct production cases:
15 byte strings in both position modes with ordinary input and four injected
read-failure positions. It checks exact partial records, ordered errors, remaining
input and final positions. Cases include doubled quotes, backslashes, escaped
newlines, CRLF, missing terminators, Unicode, malformed UTF-8 and long input.

The helper build copies verified `d2parser/parse.go` into `.reference/` and changes
only its package name to permit direct private-production calls. The tracked
harness and original source hashes are recorded alongside the oracle binary.
Go remains a development oracle; the Rust production has no Go dependency.

This subsection isolates one production and is not a general Parse/ParseValue
qualification. The public wrapper and BOM behavior are covered by the exact
match corpora above; broader malformed-input, fuzz, and cursor-operation
qualification remain pending.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example single-quoted-oracle
uv run --no-project python scripts/check-single-quoted.py
```

## Line comments and whitespace scanning

`parseComment`, `parseCommentLine`, `peekNotSpace` and `readNotSpace` are now
translated. Adjacent comment lines merge across one newline, while two newlines
end the production. Only the first ASCII space after each hash is removed; tabs
and additional spaces remain. Whitespace classification uses Go's complete
White Space set explicitly. EOF rewinds pending whitespace and I/O failures
retain their original diagnostic ordering.

`docs/comment-parser-parity.json` matches 132 Go cases across both position
modes, including merged and separated comments, CRLF, Unicode whitespace,
malformed UTF-8, long input and injected read failures. Both captured parser
reports (282 cases) also run under `cargo test` without invoking Go.

Block-string translation is next. Static analysis found that upstream
`splitLeadingIndent` counts whitespace runes but slices the string at that count
as a byte offset. Its intermediate strings can therefore contain invalid UTF-8
for non-ASCII indentation. The Rust translation must preserve those byte
operations rather than replace them with character slicing. No block-string
parity is claimed yet.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example single-quoted-oracle
uv run --no-project python scripts/check-comment-parser.py
cargo test -p d2-rs-language
```

## Byte-exact indentation

`parser/indent.rs` now translates `splitLeadingIndent`, `trimIndent`,
`trimCommonIndent` and `trimSpaceAfterLastNewline` on byte buffers. It preserves
the rune-count/byte-offset behavior identified above, tab expansion to two
spaces, exact-equality stopping limits, whitespace-only lines and trailing
newlines. No invalid UTF-8 is repaired or discarded by these helpers.

`docs/indent-parser-parity.json` compares 456 cases as hex bytes against the
private Go production helper. All match. The cases cover ASCII and Unicode
whitespace, tabs, limits from -1 through 8, malformed UTF-8 and multiple lines.
The captured cases also run in native tests. `Utf8Reader` now shares its rune
decoder with these helpers; existing production regressions check that change.

The current AST string fields use Rust String. Block strings require a byte
representation that preserves upstream's potentially invalid contents through
subsequent operations; repairing only their JSON output would not establish
semantic parity. The block-string integration is described in the following section.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example indent-oracle
uv run --no-project python scripts/check-indent-parser.py
cargo test -p d2-rs-language
```

## Block-string production and byte storage

`parser/block.rs` now translates block strings. `BlockString.value` uses
`ByteString`, retaining invalid UTF-8 through indentation and scalar comparisons.
`Scalar.scalar_bytes` and box byte accessors provide semantic values;
`scalar_string` remains text presentation and can replace invalid bytes. Key
equality and key-path equality now use bytes. Further byte-preserving construction
and identifier/import helpers remain necessary before full language parity.

The production preserves upstream's delimiter quirks: removing the last rune's
width from the front of the quote, and using the remaining byte length as a rune
lookahead count. It restores depth on all ordinary exits and keeps partial
records and ordered errors. Unicode letter/digit classification comes from all
code points of Go 1.27.0's Unicode 17.0.0 tables; the generated source and oracle
hashes are recorded in `docs/parser-unicode-reference.json`.

All 408 cases in `docs/block-parser-parity.json` match both raw scalar hex and
JSON, including Unicode indentation, malformed UTF-8, symbolic/mixed-width
delimiters, nesting depths and read failures. Native fixture tests include these
cases. This production corpus does not by itself establish complete file-parser
or helper-wrapper parity; see the public-boundary qualification above.

```sh
uv run --no-project python scripts/build-parser-oracle.py
uv run --no-project python scripts/generate-parser-unicode.py
cargo build -p d2-rs-language --example single-quoted-oracle
uv run --no-project python scripts/check-block-parser.py
```

## Block comments

`parseBlockComment` and the cursor's `peekn` operation are translated. Block
comments retain raw bytes after indentation, restore depth on ordinary exits,
and preserve upstream's partial closing-quote/error behavior. The Go harness
consumes the opening triple quote before invoking the production, as the parent
parser does.

`docs/block-comment-parser-parity.json` matches 312 cases: both position modes,
two initial depths, six read-failure settings, empty/unfinished comments, partial
closing quotes, tabs, CRLF, Unicode indentation, invalid UTF-8 and long content.
Raw value hex is compared in addition to JSON, and captured cases run in native
tests. This brings captured string/comment productions to 1,002 cases, plus
456 indentation-helper cases.

The next dependency group is recursive: parseKey calls parseString; unquoted
and double-quoted strings call parseSubstitution; substitutions call parseKey.
These must be integrated together, preserving prefix peeks, pattern fragments,
raw spelling and partial results. No fallback string parser has been added.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example single-quoted-oracle
uv run --no-project python scripts/check-block-comment-parser.py
```

## Recursive keys, strings and substitutions

`parser/strings.rs` translates parseKey, parseString, parseUnquotedString,
parseDoubleQuotedString, parseSubstitution and decodeEscape together. This
preserves their recursive calls, four import-prefix peeks, nil/empty values,
518-byte key checks, raw spelling, source ranges and partial recovery. Ordinary
and edge-group contexts remain distinct.

A combined pattern/substitution witness (`*${a}é*`) exposed a Rust panic where
Go instead returns an invalid-byte pattern fragment. Pattern storage now uses
ByteString and slicing uses byte offsets. The comparator checks raw pattern
hex as well as JSON. This preserves the source behavior rather than repairing
the fragment or resetting its index. A deterministic 702-input differential
matrix then found 326 cases where Go v0.9.0 panics at `parse.go:1208` while
slicing a pattern suffix after a substitution; Rust panics at the corresponding
suffix slice, with no Rust-only panics or output mismatches. These are recorded
as upstream malformed-input limitations, not successful parses. The minimized
witness `a: x*${x}a*` runs in the native test suite. Reproduce the matrix with:

```sh
cargo build -p d2-rs-language --release --example parse-oracle
uv run --no-project python scripts/check-parser-pattern-index.py
```

A bounded public-Parse depth sweep now compares nested arrays, maps and
substitutions from depth 1 through 512 in isolated pinned Go and optimized Rust
processes. All 30 cases match with no timeout; this does not establish a fixed
resource limit. Broader fuzz/property testing and explicit resource-limit
qualification remain pending. See `docs/parser-depth-parity.json`.

All 1,966 cases in `docs/string-parser-parity.json` match: key/string/unquoted/
double/substitution entry points, both position modes, in-key and value cases,
ordinary and edge-group contexts, read failures, invalid UTF-8, spread
substitutions and the combined regression. These cases also run in native tests.
The public ParseKey/ParseValue helpers are separately covered by the
1,229-case wrapper corpus, and this parser's container, edge, numeric, and
recovery paths are covered by the file-map corpus below. Broader fuzz and
resource-limit qualification remain pending.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example strings-oracle
uv run --no-project python scripts/check-string-parser.py
```

## Import and numeric productions

Import parsing now preserves relative prefixes, spread source ranges, quoted
path components, partial keys and upstream's unquoted `.d2` component removal.
All 256 cases in `docs/import-parser-parity.json` match, including read failures
and both position modes. Import resolution, filesystems and cycles belong to
later compiler integration and are not implemented by this production.

`parser/numeric.rs` translates the reachable Go 1.27.0 math/big string scanner
and rational construction rules. It accepts base prefixes, decimal/radix points,
fractions, separators and decimal/binary exponents without float conversion. It
preserves differing rules for floating mantissas versus integer fractions,
unsigned denominators, checked exponent parsing, zero special cases and Go's
exponent magnitude limits. Rejected numeric text returns None for later value
dispatch to keep as a string. Mutation of an existing Go Rat receiver is outside
this parser helper's API.

All 977 cases in `docs/numeric-parser-parity.json` match acceptance and exact
reduced values. Both numeric and import cases run under native workspace tests.
Broader scanner fuzzing, accepted exponent-limit stress cases and full value
dispatch remain pending. Go BSD attribution for the numeric translation is
packaged in every crate.

```sh
uv run --no-project python scripts/build-parser-oracle.py
cargo build -p d2-rs-language --example numeric-oracle --example strings-oracle
uv run --no-project python scripts/check-numeric-parser.py
uv run --no-project python scripts/check-import-parser.py
```

## File-map, containers, values and edge keys

`parser/containers.rs` and `parser/edges.rs` connect the existing productions
into the native file-map parser. They translate maps, arrays, special nodes,
value dispatch, filter prefixes, primary values, edge chains/groups, edge indices
and recovery. Values use exact rational parsing and Go-compatible folding for
the reserved ASCII keywords, including the long-s variant. Unicode digit and
printability tables are generated from the pinned Go toolchain.

The translation preserves source quirks: array range ends follow reader position;
array boxes omit suspension values; invalid/overflowing edge indices use the
ignored-error Atoi result; group flags reset as upstream does; and malformed
input retains partial ASTs and ordered diagnostics.

All 754 cases in `docs/container-parser-parity.json` match the private Go
productions. This includes 696 constructed cases across entry points and read
failures, plus 29 real D2 fixtures in both position modes. Copied upstream
fixtures are checked against their manifest hashes and pristine source bytes.
Captured results also run as native tests.

This qualifies file-map parsing through the rune-reader boundary. The public
wrapper, injected ParseError behavior, and the 45 inline cases from the pinned
upstream parser unit test are qualified separately above. Broader fuzzing and
panic/resource-limit qualification remain. Compilation and rendering are
separate unfinished stages.

```sh
uv run --no-project python scripts/build-parser-oracle.py
uv run --no-project python scripts/generate-parser-unicode.py
cargo build -p d2-rs-language --example containers-oracle
uv run --no-project python scripts/check-container-parser.py
```
