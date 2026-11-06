// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
)

func d2rsImageOptions(t *testing.T, format exportExtension, scale float64) (d2isometricimg.Options, error) {
	options, err := isometricImageOptions(format, scale)
	message := ""
	if err != nil {
		message = err.Error()
	}
	input := map[string]any{"format": string(format), "scale_bits": fmt.Sprintf("%016x", math.Float64bits(scale))}
	result := map[string]any{"format": options.Format, "width": options.Width, "height": options.Height, "error": message}
	data, encodeErr := json.Marshal(map[string]any{"test": t.Name(), "call": d2rsModeCall, "input": input, "result": result})
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	path := filepath.Join(os.Getenv("D2RS_MODE_CAPTURE"), "calls", fmt.Sprintf("%03d.json", d2rsModeCall))
	d2rsModeCall++
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	return options, err
}

// Separate source-derived witnesses; these do not credit an original test.
func TestD2rsSourceImageAdmissionPriority(t *testing.T) {
	for _, test := range []struct {
		name string
		args []string
	}{
		{"scale", []string{"--scale=0"}},
		{"appendix", []string{"--force-appendix"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			dir := d2rsModeDir(t)
			path := filepath.Join(dir, "out.txt")
			sentinel := []byte("previous private output")
			if err := os.WriteFile(path, sentinel, 0600); err != nil {
				t.Fatal(err)
			}
			args := append(test.args, "out.txt")
			data, err := imageCLI(t, dir, "vars: {d2-config: {isometric: true}}\na", args...)
			if err == nil || !strings.Contains(err.Error(), "--isometric exports SVG, PNG, GIF, PDF or PPTX") {
				t.Fatalf("admission error: %v", err)
			}
			if len(data) != 0 {
				t.Fatal("failed export wrote stdout")
			}
			got, err := os.ReadFile(path)
			if err != nil || !bytes.Equal(got, sentinel) {
				t.Fatalf("clobbered output %q %v", got, err)
			}
		})
	}
}
