// SPDX-License-Identifier: MPL-2.0
// Test-only wrappers: all original calls and assertions remain active.
package d2svg

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
	"github.com/d2lang/d2/d2target"
)

var d2rsCounts = map[string]int{}

func d2rsRecord(t *testing.T, d *d2target.Diagram, opts *RenderOpts, native *d2isometricimg.Options, api string, out any, err error) {
	t.Helper()
	root := os.Getenv("D2RS_SVG_ADMISSION_DUMP")
	if root == "" {
		return
	}
	// JSON cannot represent Inf. Preserve its exact IEEE input separately,
	// restoring it in the native runner before the typed API is called.
	copyOpts := *opts
	var scaleBits *uint64
	if opts.Scale != nil {
		bits := math.Float64bits(*opts.Scale)
		scaleBits = &bits
		copyOpts.Scale = nil
	}
	request := map[string]any{"diagram": d, "opts": &copyOpts, "scaleBits": scaleBits, "native": native, "api": api}
	message := ""
	if err != nil {
		message = err.Error()
	}
	name := fmt.Sprintf("%s#%d", t.Name(), d2rsCounts[t.Name()])
	d2rsCounts[t.Name()]++
	row := map[string]any{"name": name, "test": t.Name(), "request": request, "got": map[string]any{"output": out, "error": message}}
	data, e := json.Marshal(row)
	if e != nil {
		t.Fatal(e)
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if e := os.WriteFile(filepath.Join(root, key+".json"), data, 0600); e != nil {
		t.Fatal(e)
	}
}
func d2rsRender(t *testing.T, d *d2target.Diagram, opts *RenderOpts) ([]byte, error) {
	out, err := Render(d, opts)
	d2rsRecord(t, d, opts, nil, "Render", out, err)
	return out, err
}
func d2rsRenderMultiboard(t *testing.T, d *d2target.Diagram, opts *RenderOpts) ([][]byte, error) {
	out, err := RenderMultiboard(d, opts)
	d2rsRecord(t, d, opts, nil, "RenderMultiboard", out, err)
	return out, err
}
func d2rsRenderIsometric(t *testing.T, ctx context.Context, d *d2target.Diagram, opts *RenderOpts, native *d2isometricimg.Options) ([]byte, error) {
	out, err := RenderIsometric(ctx, d, opts, native)
	d2rsRecord(t, d, opts, native, "RenderIsometric", out, err)
	return out, err
}
