// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/yuin/goldmark/text"
	"os"
)

type request struct {
	Source  string
	Segment text.Segment
	Widths  []int
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, r := range requests {
		source, err := hex.DecodeString(r.Source)
		if err != nil {
			panic(err)
		}
		s := r.Segment
		widths := make([]text.Segment, 0, len(r.Widths))
		for _, w := range r.Widths {
			widths = append(widths, s.TrimLeftSpaceWidth(w, source))
		}
		other := text.NewSegmentPadding(s.Stop, s.Stop, 0)
		results = append(results, map[string]any{
			"value": hex.EncodeToString(s.Value(source)), "len": s.Len(), "empty": s.IsEmpty(),
			"left": s.TrimLeftSpace(source), "right": s.TrimRightSpace(source), "widths": widths,
			"start": s.WithStart(s.Stop), "stop": s.WithStop(s.Start), "between": s.Between(other),
			"concat": hex.EncodeToString(s.ConcatPadding([]byte("prefix"))),
		})
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
