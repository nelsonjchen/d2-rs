// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"
	"os"
)

type processor struct{ Allowed string }

func (p processor) IsDelimiter(b byte) bool {
	for i := range len(p.Allowed) {
		if p.Allowed[i] == b {
			return true
		}
	}
	return false
}
func (p processor) CanOpenCloser(a, b *parser.Delimiter) bool { return a.Char == b.Char }
func (p processor) OnMatch(n int) ast.Node                    { return ast.NewEmphasis(n) }

type request struct {
	Line    string
	Before  int32
	Minimum int
	Allowed string
}

type delimiterSpec struct {
	CanOpen        bool         `json:"can_open"`
	CanClose       bool         `json:"can_close"`
	Length         int          `json:"length"`
	OriginalLength int          `json:"original_length"`
	Char           byte         `json:"char"`
	Segment        text.Segment `json:"segment"`
}

func (s delimiterSpec) make() *parser.Delimiter {
	d := parser.NewDelimiter(s.CanOpen, s.CanClose, s.Length, s.Char, processor{"*_"})
	d.OriginalLength = s.OriginalLength
	d.Segment = s.Segment
	return d
}
func spec(d *parser.Delimiter) delimiterSpec {
	return delimiterSpec{d.CanOpen, d.CanClose, d.Length, d.OriginalLength, d.Char, d.Segment}
}
func consumption() {
	var requests []struct {
		Opener delimiterSpec
		Closer delimiterSpec
		Count  int
	}
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, q := range requests {
		opener, closer := q.Opener.make(), q.Closer.make()
		before := opener.CalcComsumption(closer)
		opener.ConsumeCharacters(q.Count)
		closer.ConsumeCharacters(q.Count)
		results = append(results, map[string]any{"before": before, "after": opener.CalcComsumption(closer), "opener": spec(opener), "closer": spec(closer)})
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
func main() {
	if len(os.Args) > 1 && os.Args[1] == "consume" {
		consumption()
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "ranges" {
		ranges := make([][3]int, 0)
		start, previous := 0, 0
		for i := 0; i <= 0x110000; i++ {
			flags := 0
			if i < 0x110000 {
				if util.IsPunctRune(rune(i)) {
					flags |= 1
				}
				if util.IsSpaceRune(rune(i)) {
					flags |= 2
				}
			}
			if flags != previous {
				if previous != 0 {
					ranges = append(ranges, [3]int{start, i - 1, previous})
				}
				start = i
				previous = flags
			}
		}
		if err := json.NewEncoder(os.Stdout).Encode(ranges); err != nil {
			panic(err)
		}
		return
	}
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, q := range requests {
		line, err := hex.DecodeString(q.Line)
		if err != nil {
			panic(err)
		}
		d := parser.ScanDelimiter(line, rune(q.Before), q.Minimum, processor{q.Allowed})
		if d == nil {
			results = append(results, nil)
		} else {
			results = append(results, map[string]any{"can_open": d.CanOpen, "can_close": d.CanClose, "length": d.Length, "original_length": d.OriginalLength, "char": d.Char, "segment": d.Segment})
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
