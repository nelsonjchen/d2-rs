# Markdown substitution source contract

Evidence JSON cited below as `docs/*.json` is no longer in the tree. It was
retired at 9b706ac1 and in the 2026-10-31 second pass; `docs/RETIRED-EVIDENCE.md`
lists each file and how to read it back. Checkers now write fresh reports
under the gitignored `oracle-cache/reports/`.

Target: D2 v0.9.0, `d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`,
Go 1.27.0, Goldmark v1.8.5. This is investigation evidence, not implemented
native Markdown compatibility.

`d2ir/compile.go` handles every BlockString through the Markdown substitution
helper, regardless of its language tag. `collectVariables` visits fields in
their stored order. Scalar fields contribute their scalar spelling. Map fields
contribute both qualified descendants and unqualified descendants. A field with
both a primary and a map takes the primary branch. Later assignments overwrite
earlier names, including assignments made by later variable scopes.

`lib/textmeasure/substitutions.go` parses the original bytes with the shared
Goldmark parser configured in `markdown.go`: default parsers plus Strikethrough
and Table extensions. Its walk skips code blocks, fenced blocks and code spans.
Only `ast.Text` segments are eligible. Each segment is transformed independently,
then changed segments are spliced into the original source in descending start
offset order. This preserves Markdown markup and uses byte offsets, not character
or UTF-16 indices. Segment boundaries matter even when rendered text is identical.

For each text segment, `replaceVariables` ranges over a Go map and applies
`strings.ReplaceAll` once per variable. Replacement values can introduce another
variable; the resulting string depends on actual traversal order. There is no
recursive fixed-point expansion. With `a = "${b}"` and `b = "DONE"`, the source
`${a} ${b}` produced both `${b} DONE` and `DONE DONE` in repeated calls. The
instrumented function and the unmodified function both exhibit the alternatives.
A deterministic sorted replacement pass would choose one possible result but
would not establish upstream execution parity.

The 26 reference requests cover plain/Unicode text, code spans and blocks,
emphasis and strikethrough, links and images, inline and block HTML, comments,
escapes, hard/soft line breaks, headings, quotes, lists, tables, qualified and
missing variables, and replacement chaining. Notable observations:

- Link/image labels are substituted; destinations and titles are preserved.
- Inline HTML surrounding text permits substitution; HTML blocks and comments
  do not expose eligible text segments.
- A backslash before `${x}` does not prevent replacement in the observed case.
- Code remains untouched, including code inside table cells.

Run `uv run python scripts/check-markdown-substitution-reference.py` to rebuild
both development oracles. The script verifies the pristine checkout before and
after, records complete source/harness/overlay/binary checksums, and retains text
segments, actual map traversal and every observed output in
`markdown-substitution-reference.json`. Twenty-five independent-replacement
cases agree between instrumented and original functions. The chained case is
checked against the source-derived possible outcomes; this is neither statistical
distribution qualification nor run-for-run trace parity.

Next implementation dependency: translate the reachable Goldmark block/inline
segmentation behavior and variable collection before qualifying substituted
block strings against the native compiler. The source path now uses the
translated AST for a visible plain-text projection during measurement and
semantic SVG rendering; styled HTML output, substitution parity, and the rest
of Goldmark's corpus remain separate work. Qualify traversal-sensitive cases
explicitly; do not normalize their outputs into a successful parity result.
