// SPDX-License-Identifier: MPL-2.0
// Finite regression witnesses for canonicalGradientStopPosition's exponent
// boundaries and the radial counterpart of the original constructed-ID test.
package main

import (
	"encoding/json"
	"github.com/d2lang/d2/lib/color"
	"os"
)

func main() {
	var requests []struct {
		Op       string         `json:"op"`
		CSS      string         `json:"css"`
		Gradient color.Gradient `json:"gradient"`
	}
	f, err := os.Open(os.Args[1])
	if err != nil {
		panic(err)
	}
	if err := json.NewDecoder(f).Decode(&requests); err != nil {
		panic(err)
	}
	var rows []any
	for _, r := range requests {
		var got any
		rawError := ""
		switch r.Op {
		case "parse":
			g, err := color.ParseGradient(r.CSS)
			if err != nil {
				rawError = err.Error()
			}
			got = map[string]any{"gradient": g, "error": err != nil}
		case "svg":
			got = color.GradientToSVG(r.Gradient)
		default:
			panic(r.Op)
		}
		rows = append(rows, map[string]any{"request": r, "got": got, "raw_error": rawError})
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
