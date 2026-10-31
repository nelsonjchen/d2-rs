// SPDX-License-Identifier: MPL-2.0
package urlenc

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

func d2rsURLCapture(t *testing.T, op, input string, call int, result string, err error) {
	if dir := os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"); dir != "" {
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		name := fmt.Sprintf("%s#%d", t.Name(), call)
		row := map[string]interface{}{"name": name, "test": t.Name(), "request": map[string]interface{}{"op": op, "input": input}, "got": map[string]interface{}{"result": result, "error": errorText}}
		data, err := json.Marshal(row)
		if err != nil {
			t.Fatal(err)
		}
		id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
		if err := os.WriteFile(filepath.Join(dir, id+".json"), data, 0644); err != nil {
			t.Fatal(err)
		}
	}
}

func d2rsURLEncode(t *testing.T, raw string) (string, error) {
	result, err := Encode(raw)
	d2rsURLCapture(t, "encode", raw, 0, result, err)
	return result, err
}

func d2rsURLDecode(t *testing.T, encoded string) (string, error) {
	result, err := Decode(encoded)
	d2rsURLCapture(t, "decode", encoded, 1, result, err)
	return result, err
}
