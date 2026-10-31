// SPDX-License-Identifier: MPL-2.0
// Raw JSON oracle for the pinned d2target model (recipe d2-target-json-probe,
// formerly tools/d2-target-json-probe/main.go). Overlaid into d2target; inert
// unless run by scripts/regen-fixtures.py. Each committed raw input is decoded
// with encoding/json, re-encoded, serialized with Diagram.Bytes and hashed.
package d2target

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"testing"
)

type d2rsJSONInput struct {
	Name      string `json:"name"`
	Raw       string `json:"raw"`
	RawBase64 string `json:"raw_base64"`
}

type d2rsJSONResult struct {
	Name      string `json:"name"`
	Outcome   string `json:"outcome"`
	Canonical string `json:"canonical,omitempty"`
	HashInput string `json:"hash_input,omitempty"`
	Hash      string `json:"hash,omitempty"`
	Error     string `json:"error,omitempty"`
}

func d2rsDecodeTarget(in d2rsJSONInput) (out d2rsJSONResult) {
	out.Name = in.Name
	out.Outcome = "decode-error"
	defer func() {
		if e := recover(); e != nil {
			out.Outcome = "source-panic"
			out.Error = fmt.Sprint(e)
		}
	}()
	var diagram *Diagram
	raw := []byte(in.Raw)
	if in.RawBase64 != "" {
		raw, _ = base64.StdEncoding.DecodeString(in.RawBase64)
	}
	if e := json.Unmarshal(raw, &diagram); e != nil {
		out.Error = e.Error()
		return
	}
	if diagram == nil {
		out.Outcome = "nil"
		return
	}
	out.Outcome = "decoded"
	b, e := json.Marshal(diagram)
	if e != nil {
		out.Outcome = "encode-error"
		out.Error = e.Error()
		return
	}
	out.Canonical = string(b)
	b, e = diagram.Bytes()
	if e != nil {
		out.Outcome = "hash-error"
		out.Error = e.Error()
		return
	}
	out.HashInput = base64.StdEncoding.EncodeToString(b)
	out.Hash, e = diagram.HashID(nil)
	if e != nil {
		out.Outcome = "hash-error"
		out.Error = e.Error()
	}
	return
}

func TestD2RSCaptureTargetJSON(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	b, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs []d2rsJSONInput
	if err := json.Unmarshal(b, &inputs); err != nil {
		t.Fatal(err)
	}
	rows := make([]d2rsJSONResult, 0, len(inputs))
	for _, in := range inputs {
		rows = append(rows, d2rsDecodeTarget(in))
	}
	b, err = json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, append(b, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
