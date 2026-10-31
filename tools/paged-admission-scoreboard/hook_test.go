// SPDX-License-Identifier: MPL-2.0
// Observe the original calls without changing their assertions or inputs.
package d2cli

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
)

var d2rsPagedCall int

func d2rsPagedBudget(r *isometricPagedRenderer) []int64 {
	return []int64{r.remainingPixels, r.remainingEncoded, int64(r.remainingLinks), int64(r.remainingLinkBytes)}
}

func d2rsPagedRecord(t *testing.T, input, result any) {
	t.Helper()
	data, err := json.Marshal(map[string]any{"test": t.Name(), "call": d2rsPagedCall, "input": input, "result": result})
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(os.Getenv("D2RS_PAGED_CAPTURE"), fmt.Sprintf("%03d.json", d2rsPagedCall))
	d2rsPagedCall++
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsPagedError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}

func d2rsPagedAdmit(t *testing.T, r *isometricPagedRenderer, page *d2isometricimg.Page) error {
	t.Helper()
	input := map[string]any{"kind": "page", "budget": d2rsPagedBudget(r), "width": r.opts.Width, "height": r.opts.Height, "canceled": r.ctx.Err() != nil, "page": page}
	err := r.admitPage(page)
	d2rsPagedRecord(t, input, map[string]any{"error": d2rsPagedError(err), "budget": d2rsPagedBudget(r)})
	return err
}

func d2rsPagedWrite(t *testing.T, w *isometricDocumentWriter, p []byte) (int, error) {
	t.Helper()
	input := map[string]any{"kind": "write", "remaining": w.remaining, "canceled": w.ctx.Err() != nil, "data": p}
	n, err := w.Write(p)
	// The original test supplies a bytes.Buffer and asserts no bytes are written.
	d2rsPagedRecord(t, input, map[string]any{"error": d2rsPagedError(err), "remaining": w.remaining, "n": n, "output": base64.StdEncoding.EncodeToString(w.output.(*bytes.Buffer).Bytes())})
	return n, err
}
