// SPDX-License-Identifier: MPL-2.0
package textmeasure

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

var d2rsSparseVariables map[string]string

func d2rsSparseNew(t *testing.T, ctx context.Context, vars map[string]string) (*markdownSubstitutionMatcher, error) {
	d2rsSparseVariables = vars
	return newMarkdownSubstitutionMatcher(ctx, vars)
}

func d2rsSparseAllocs(t *testing.T, matcher *markdownSubstitutionMatcher, input string, runs int, f func()) float64 {
	allocations := testing.AllocsPerRun(runs, f)
	if dir := os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"); dir != "" {
		sparse := true
		for _, state := range matcher.states {
			if state.terminal && state.next != nil {
				sparse = false
			}
		}
		got, used, err := matcher.replaceBounded(context.Background(), input, 32)
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		name := t.Name() + "#0"
		row := map[string]interface{}{"name": name, "test": t.Name(),
			"request":      map[string]interface{}{"op": "sparse", "variables": d2rsSparseVariables, "input": input, "runs": runs, "limit": 32},
			"got":          map[string]interface{}{"result": got, "used": used, "error": errorText, "terminal_storage_sparse": sparse, "allocations_within_limit": allocations <= 1},
			"observations": map[string]interface{}{"allocations_per_run": allocations}}
		data, err := json.Marshal(row)
		if err != nil {
			t.Fatal(err)
		}
		id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
		if err := os.WriteFile(filepath.Join(dir, id+".json"), data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	return allocations
}
