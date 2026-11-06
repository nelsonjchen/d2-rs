// SPDX-License-Identifier: MPL-2.0
// Env-gated observation of original public Markdown substitution calls.
package textmeasure

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

var d2rsMarkdownMu sync.Mutex
var d2rsMarkdownCalls = map[string]int{}

func d2rsMarkdownReplace(t *testing.T, ctx context.Context, source string, vars map[string]string, limit int64) (string, int64, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return ReplaceSubstitutionsMarkdownBounded(ctx, source, vars, limit)
	}
	policy := map[string]interface{}{"kind": "background"}
	if c, ok := ctx.(*markdownCancelAfterErrContext); ok {
		policy = map[string]interface{}{"kind": "cancel_after_checks", "cancel_at": c.cancelAt, "initial_calls": c.calls}
	} else if ctx != nil {
		if deadline, ok := ctx.Deadline(); ok {
			policy = map[string]interface{}{"kind": "deadline", "remaining_ns": time.Until(deadline).Nanoseconds()}
		}
	}
	result, used, err := ReplaceSubstitutionsMarkdownBounded(ctx, source, vars, limit)
	errorText, errorKind := "", ""
	if err != nil {
		errorText = err.Error()
	}
	if errors.Is(err, context.Canceled) {
		errorKind = "canceled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		errorKind = "deadline"
	}
	d2rsMarkdownMu.Lock()
	defer d2rsMarkdownMu.Unlock()
	call := d2rsMarkdownCalls[t.Name()]
	d2rsMarkdownCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	row := map[string]interface{}{"name": name, "test": t.Name(),
		"request": map[string]interface{}{"source": source, "variables": vars, "limit": limit, "context_policy": policy},
		"got":     map[string]interface{}{"result": result, "used": used, "error": errorText, "error_kind": errorKind}}
	data, marshalErr := json.Marshal(row)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return result, used, err
}
