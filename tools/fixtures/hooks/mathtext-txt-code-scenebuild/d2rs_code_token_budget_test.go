// SPDX-License-Identifier: MPL-2.0
// Code token expansion under scene node budgets x context (recipe mathtext-txt-code-token-budget).
package d2scenebuild

import (
	"context"
	"encoding/json"
	"github.com/alecthomas/chroma/v2"
	"os"
	"strings"
	"testing"
	"time"
)

func TestD2RSCodeTokenBudget(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type Value struct {
		Unit   string `json:"unit"`
		Repeat int    `json:"repeat"`
	}
	type Request struct {
		Name           string  `json:"name"`
		MaxNodes       int     `json:"max_nodes"`
		Values         []Value `json:"values"`
		SequenceRepeat int     `json:"sequence_repeat"`
		Cancel         string  `json:"cancel"`
	}
	var requests []Request
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	var out []map[string]any
	for _, r := range requests {
		ctx, cancel := context.WithCancel(context.Background())
		if r.Cancel == "before" {
			cancel()
		}
		if r.Cancel == "deadline_before" {
			var c context.CancelFunc
			ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
			defer c()
		}
		budget := newCodeTokenBudget(r.MaxNodes)
		values := []string{}
		for _, v := range r.Values {
			values = append(values, strings.Repeat(v.Unit, v.Repeat))
		}
		count := len(values) * r.SequenceRepeat
		index := 0
		iterator := func() chroma.Token {
			if index == 1 && r.Cancel == "after_first" {
				cancel()
			}
			if index >= count {
				return chroma.EOF
			}
			value := values[index%len(values)]
			index++
			return chroma.Token{Type: chroma.Text, Value: value}
		}
		tokens, err := collectCodeTokens(ctx, iterator, &budget)
		message := ""
		if err != nil {
			message = err.Error()
		}
		out = append(out, map[string]any{"request": r, "error": message, "raw_tokens": budget.rawTokens, "expanded_pieces": budget.expandedPieces, "returned_tokens": len(tokens)})
		cancel()
	}
	data, err = json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
