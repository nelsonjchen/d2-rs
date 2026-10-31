// SPDX-License-Identifier: MPL-2.0
// Oracle for github.com/rivo/uniseg (pinned by go.mod), which D2 uses for
// line breaking, grapheme clusters and string width. Run by
// tools/fixtures/mathtext-txt.toml:
//
//	mathtext-uniseg-oracle INPUT.json > OUTPUT.json
//
// INPUT.json is a JSON list of strings (scripts/fixture-gen-uniseg.py).
package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/rivo/uniseg"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: mathtext-uniseg-oracle INPUT.json")
		os.Exit(2)
	}
	data, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var inputs []string
	if err := json.Unmarshal(data, &inputs); err != nil {
		panic(err)
	}
	rows := []map[string]any{}
	for _, input := range inputs {
		segments := []map[string]any{}
		line := [][2]any{}
		remaining, state, offset := input, -1, 0
		for remaining != "" {
			segment, rest, must, next := uniseg.FirstLineSegmentInString(remaining, state)
			offset += len(segment)
			line = append(line, [2]any{offset, must})
			segments = append(segments, map[string]any{"segment": segment, "rest": rest, "must_break": must, "new_state": next})
			remaining, state = rest, next
		}
		graphemes := uniseg.NewGraphemes(input)
		ends := []int{}
		for graphemes.Next() {
			_, end := graphemes.Positions()
			ends = append(ends, end)
		}
		rows = append(rows, map[string]any{"string_width": uniseg.StringWidth(input), "input": input, "line_boundaries": line, "grapheme_boundaries": ends, "source_segments": segments})
	}
	out, err := json.Marshal(rows)
	if err != nil {
		panic(err)
	}
	if _, err := os.Stdout.Write(append(out, '\n')); err != nil {
		panic(err)
	}
}
