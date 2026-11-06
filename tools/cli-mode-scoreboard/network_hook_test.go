// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/util-go/cmdlog"
	"github.com/d2lang/util-go/xmain"
)

// Preserve the direct parser's returned error and the original errors.As
// assertion. Observe its CLI terminal projection for subprocess comparison.
func d2rsNetworkDefault(t *testing.T, ms *xmain.State) (bool, error) {
	value, err := privateNetworkEnvDefault(ms)
	dir := d2rsModeDir(t)
	var terminal bytes.Buffer
	if err != nil {
		logger := cmdlog.New(ms.Env, &terminal)
		logger.Error.Print(err.Error())
		logger.Error.Print("Run with --help to see usage.")
	}
	input := map[string]any{"directory": dir, "args": []string{"--help"}, "env": ms.Env.Environ(), "canceled": false, "files": d2rsModeFiles(t, dir)}
	result := map[string]any{"error": terminal.String(), "stdout": "", "files": d2rsModeFiles(t, dir)}
	data, encodeErr := json.Marshal(map[string]any{"test": t.Name(), "call": d2rsModeCall, "input": input, "result": result})
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	path := filepath.Join(os.Getenv("D2RS_MODE_CAPTURE"), "calls", fmt.Sprintf("%03d.json", d2rsModeCall))
	d2rsModeCall++
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	return value, err
}
