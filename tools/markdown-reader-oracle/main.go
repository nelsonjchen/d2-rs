// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"os"
)

type operation struct {
	Kind    string
	N       int
	Padding int
	Segment text.Segment
	Opener  byte
	Closer  byte
	Options text.FindClosureOptions
}
type request struct {
	Source     string
	Block      bool
	Segments   []text.Segment
	PeekLine   bool
	Operations []operation
}

func inlineNode(n ast.Node) any {
	if t, ok := n.(*ast.Text); ok {
		return map[string]any{"kind": "Text", "segment": t.Segment, "raw": t.IsRaw()}
	}
	if _, ok := n.(*ast.CodeSpan); ok {
		children := make([]any, 0)
		for c := n.FirstChild(); c != nil; c = c.NextSibling() {
			children = append(children, inlineNode(c))
		}
		return map[string]any{"kind": "CodeSpan", "children": children}
	}
	panic("unexpected inline node")
}
func evaluate(q request) (result any) {
	defer func() {
		if recover() != nil {
			result = map[string]any{"panic": true}
		}
	}()
	source, err := hex.DecodeString(q.Source)
	if err != nil {
		panic(err)
	}
	var r text.Reader = text.NewReader(source)
	if q.Block {
		segments := text.NewSegments()
		segments.AppendAll(q.Segments)
		r = text.NewBlockReader(source, segments)
	}
	rows := make([]any, 0)
	savedLine, savedPos := r.Position()
	snapshot := func(value any) {
		var lineValue any
		if q.PeekLine {
			line, _ := r.PeekLine()
			if line != nil {
				lineValue = hex.EncodeToString(line)
			}
		}
		line, pos := r.Position()
		rows = append(rows, map[string]any{"source": hex.EncodeToString(r.Source()), "value": value, "line": line, "position": pos, "peek": r.Peek(), "text": lineValue, "offset": r.LineOffset(), "preceding": r.PrecendingCharacter()})
	}
	snapshot(nil)
	for _, op := range q.Operations {
		var value any
		switch op.Kind {
		case "code-span":
			value = inlineNode(parser.NewCodeSpanParser().Parse(ast.NewParagraph(), r, parser.NewContext()))
		case "closure":
			segments, ok := r.FindClosure(op.Opener, op.Closer, op.Options)
			var values any
			if segments != nil {
				values = segments.Sliced(0, segments.Len())
			}
			value = map[string]any{"segments": values, "ok": ok}
		case "value":
			value = hex.EncodeToString(r.Value(op.Segment))
		case "advance":
			r.Advance(op.N)
		case "padding":
			r.SetPadding(op.Padding)
		case "advance-padding":
			r.AdvanceAndSetPadding(op.N, op.Padding)
		case "line":
			r.AdvanceLine()
		case "eol":
			r.AdvanceToEOL()
		case "reset":
			r.ResetPosition()
		case "save":
			savedLine, savedPos = r.Position()
		case "restore":
			r.SetPosition(savedLine, savedPos)
		case "rune":
			rn, size, err := r.ReadRune()
			value = map[string]any{"rune": rn, "size": size, "eof": err != nil}
		case "spaces":
			seg, n, ok := r.SkipSpaces()
			value = map[string]any{"segment": seg, "n": n, "ok": ok}
		case "blank":
			seg, n, ok := r.SkipBlankLines()
			value = map[string]any{"segment": seg, "n": n, "ok": ok}
		default:
			panic("unknown operation")
		}
		snapshot(value)
	}
	return rows
}
func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, q := range requests {
		results = append(results, evaluate(q))
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
