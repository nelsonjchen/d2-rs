// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/json"
	"os"

	"github.com/d2lang/d2/lib/textmeasure"
)

type request struct {
	Text      string
	Variables map[string]string
	Repeats   int
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, r := range requests {
		runs := make([]any, 0)
		for i := 0; i < max(1, r.Repeats); i++ {
			output, events := textmeasure.OracleMarkdownSubstitutions(r.Text, r.Variables)
			runs = append(runs, map[string]any{"output": output, "events": events})
		}
		results = append(results, runs)
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
