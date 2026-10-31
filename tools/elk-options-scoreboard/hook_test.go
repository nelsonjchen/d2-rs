package d2elklayout

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

var d2rsOptionCall int

func d2rsOptionCapture(t *testing.T, kind string, input any, result *elkOpts) *elkOpts {
	t.Helper()
	if directory := os.Getenv("D2RS_ELK_OPTIONS_CAPTURE"); directory != "" {
		data, err := json.Marshal(map[string]any{"test": t.Name(), "input": map[string]any{"kind": kind, "options": input}, "result": result})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(directory, fmt.Sprintf("%03d.json", d2rsOptionCall)), data, 0600); err != nil {
			t.Fatal(err)
		}
		d2rsOptionCall++
	}
	return result
}

func d2rsRootOptions(t *testing.T, input *ConfigurableOpts) *elkOpts {
	return d2rsOptionCapture(t, "root", input, newRootLayoutOptions(input))
}

func d2rsContainerOptions(t *testing.T, input *ConfigurableOpts) *elkOpts {
	return d2rsOptionCapture(t, "container", input, newContainerLayoutOptions(input))
}

func d2rsBareOptionsMarshal(t *testing.T, input *elkOpts) ([]byte, error) {
	kind := "leaf"
	var options any
	if input.PortSide != "" {
		kind = "port"
		options = map[string]any{"side": input.PortSide}
	}
	if input.InlineEdgeLabels {
		kind = "edge-label"
	}
	d2rsOptionCapture(t, kind, options, input)
	return json.Marshal(input)
}
