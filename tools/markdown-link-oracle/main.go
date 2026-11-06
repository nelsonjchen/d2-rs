// SPDX-License-Identifier: MPL-2.0
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"
	"os"
	"strings"
)

type request struct {
	Source     string
	Segments   []text.Segment
	Kind       string
	Alignments []int
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
	if q.Kind == "setextlife" || q.Kind == "setextempty" || q.Kind == "setextnext" || q.Kind == "setextnextempty" {return parser.OracleSetextLifecycle(source,q.Segments,q.Kind)}
	if q.Kind == "atx" {return parser.OracleATX(source,q.Segments[0])}
	if q.Kind == "setextbar" { return parser.OracleSetext(source) }
	if q.Kind == "listitem" { return parser.OracleListItem(source) }
	if q.Kind == "fenceblock" { return parser.OracleFenceBlock(source,q.Segments) }
	if q.Kind == "codeblock" { return parser.OracleCodeBlock(source,q.Segments) }
	if q.Kind == "blockquote" || q.Kind == "thematic" { return parser.OracleSimpleBlock(source,q.Segments[0],q.Kind) }
	if q.Kind == "htmlblock" { return parser.OracleHTMLBlock(source,q.Segments) }
	if q.Kind == "tableparagraph" || q.Kind == "tableinline" { return extension.OracleTableParagraph(source, q.Segments, q.Kind == "tableinline") }
	if strings.HasPrefix(q.Kind, "table") {
		return extension.OracleTable(source, q.Segments[0], q.Alignments, q.Kind)
	}
	if q.Kind == "refparagraph" || q.Kind == "refparagraphmutate" {
		return parser.OracleReferenceParagraph(source, q.Segments, q.Kind == "refparagraphmutate")
	}
	if strings.HasPrefix(q.Kind, "inline") {
		var extra parser.InlineParser
		if q.Kind == "inlinestrike" {
			extra = extension.NewStrikethroughParser()
		}
		return parser.OracleInlineBlock(source, q.Segments, q.Kind == "inlineescaped", q.Kind == "inlineraw", extra)
	}
	if q.Kind == "tagreadopen" || q.Kind == "tagreadclose" {
		return parser.OracleTagMatch(source, q.Segments, q.Kind == "tagreadclose")
	}
	if q.Kind == "tagprogram" {
		return parser.OracleTagProgram(len(source) > 0)
	}
	if q.Kind == "tagopen" || q.Kind == "tagclose" {
		return parser.OracleTagPrefix(source, q.Kind == "tagclose")
	}
	if q.Kind == "autoindex" {
		return []int{util.FindEmailIndex(source), util.FindURLIndex(source)}
	}
	if q.Kind == "parse" {
		return parser.OracleParseLinks(source, q.Segments)
	}
	if q.Kind == "openclose" {
		return parser.OracleLinkOpenClose(source, q.Segments)
	}
	if q.Kind == "bottoms" {
		return parser.OracleLinkBottoms(source)
	}
	if q.Kind == "labels" {
		return parser.OracleLabelStates(q.Segments, source)
	}
	if q.Kind == "registry" {
		pc := parser.NewContext()
		labels := bytes.Split(source, []byte{10})
		refs := []parser.Reference{}
		keys := [][]byte{}
		for i, label := range labels {
			refs = append(refs, parser.NewReference(label, []byte(fmt.Sprint(i)), nil))
			keys = append(keys, append([]byte(nil), label...), []byte(util.ToLinkReference(label)))
		}
		snapshot := func() any {
			values := []any{}
			for _, key := range keys {
				ref, ok := pc.Reference(string(key))
				if !ok {
					values = append(values, nil)
					continue
				}
				id := -1
				for i, v := range refs {
					if v == ref {
						id = i
						break
					}
				}
				values = append(values, map[string]any{"id": id, "label": hex.EncodeToString(ref.Label()), "destination": hex.EncodeToString(ref.Destination())})
			}
			return values
		}
		rounds := []any{}
		for _, ref := range refs {
			pc.AddReference(ref)
			rounds = append(rounds, snapshot())
		}
		for _, label := range labels {
			if len(label) > 0 {
				label[0] ^= 32
			}
		}
		rounds = append(rounds, snapshot())
		return rounds
	}
	if q.Kind == "normalize" {
		return hex.EncodeToString([]byte(util.ToLinkReference(source)))
	}
	segments := text.NewSegments()
	segments.AppendAll(q.Segments)
	r := text.NewBlockReader(source, segments)
	if q.Kind == "paragraph" || q.Kind == "paragraphclear" {
		p := parser.NewParagraphParser()
		pc := parser.NewContext()
		root := ast.NewDocument()
		node, state := p.Open(root, r, pc)
		if node != nil {
			root.AppendChild(root, node)
		}
		snap := func(state parser.State) any {
			var lines any
			attached := false
			posValue := -1
			if node != nil {
				lines = append([]text.Segment{}, node.Lines().Sliced(0, node.Lines().Len())...)
				attached = node.Parent() != nil
				posValue = node.Pos()
			}
			line, pos := r.Position()
			return map[string]any{"state": int(state), "lines": lines, "attached": attached, "node_pos": posValue, "count": root.ChildCount(), "line": line, "position": pos, "source": hex.EncodeToString(source)}
		}
		rounds := []any{snap(state)}
		if node != nil {
			for {
				r.AdvanceLine()
				state = p.Continue(node, r, pc)
				rounds = append(rounds, snap(state))
				if state&parser.Close != 0 {
					break
				}
			}
			if q.Kind == "paragraphclear" {
				node.Lines().Clear()
			}
			p.Close(node, r, pc)
			rounds = append(rounds, snap(0))
		}
		return rounds
	}

	if q.Kind == "rawdelimited" || q.Kind == "rawtag" {
		n := parser.NewRawHTMLParser().Parse(ast.NewParagraph(), r, parser.NewContext())
		var value any
		if n != nil {
			v := n.(*ast.RawHTML)
			value = v.Segments.Sliced(0, v.Segments.Len())
		}
		line, pos := r.Position()
		return map[string]any{"segments": value, "line": line, "position": pos, "source": hex.EncodeToString(source)}
	}
	if q.Kind == "extract" {
		root := ast.NewDocument()
		paragraph := ast.NewParagraph()
		paragraph.SetLines(segments)
		paragraph.SetBlankPreviousLines(true)
		root.AppendChild(root, paragraph)
		parser.LinkReferenceParagraphTransformer.Transform(paragraph, text.NewReader(source), parser.NewContext())
		refs := []any{}
		for n := root.FirstChild(); n != nil; n = n.NextSibling() {
			if ref, ok := n.(*ast.LinkReferenceDefinition); ok {
				var title any
				if ref.Title != nil {
					title = hex.EncodeToString(ref.Title)
				}
				refs = append(refs, map[string]any{"label": hex.EncodeToString(ref.Label), "destination": hex.EncodeToString(ref.Destination), "title": title, "lines": ref.Lines().Sliced(0, ref.Lines().Len()), "blank": ref.HasBlankPreviousLines()})
			}
		}
		remaining := []text.Segment{}
		remaining = append(remaining, paragraph.Lines().Sliced(0, paragraph.Lines().Len())...)
		return map[string]any{"references": refs, "remaining": remaining, "removed": paragraph.Parent() == nil, "source": hex.EncodeToString(source)}
	}
	if q.Kind == "reference" {
		ref, start, end := parser.OracleReference(r)
		var data any
		if ref != nil {
			var title any
			if ref.Title != nil {
				title = hex.EncodeToString(ref.Title)
			}
			data = map[string]any{"label": hex.EncodeToString(ref.Label), "destination": hex.EncodeToString(ref.Destination), "title": title, "first_segment": ref.Lines().At(0)}
		}
		line, pos := r.Position()
		return map[string]any{"reference": data, "start": start, "end": end, "line": line, "position": pos, "source": hex.EncodeToString(source), "peek": r.Peek()}
	}
	value, ok := parser.OracleLinkValue(r, q.Kind == "title")
	var data any
	if value != nil {
		data = hex.EncodeToString(value)
	}
	line, pos := r.Position()
	return map[string]any{"value": data, "ok": ok, "line": line, "position": pos, "source": hex.EncodeToString(source), "peek": r.Peek()}
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
