// SPDX-License-Identifier: MPL-2.0
// Test-only capture boundary; installed in the pinned trace checkout.
package d2raster

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

var d2rsCaptureLock sync.Mutex
var d2rsCaptureCounts = map[string]int{}

func d2rsRasterRender(t *testing.T, ctx context.Context, document *d2scene.Document, options FrameOptions) (*image.NRGBA, error) {
	root := os.Getenv("D2RS_RASTER_API_CAPTURE")
	if root == "" {
		return Render(ctx, document, options)
	}
	// Serialize before calling Render so input mutations cannot change the witness.
	d2rsCaptureLock.Lock()
	name := t.Name()
	index := d2rsCaptureCounts[name]
	d2rsCaptureCounts[name]++
	dir := filepath.Join(root, "go", name, fmt.Sprintf("%03d", index))
	if err := os.MkdirAll(dir, 0o755); err != nil {
		panic(err)
	}
	assetStore = filepath.Join(root, "assets")
	write := func(path string, value any) {
		data, err := json.Marshal(value)
		if err != nil {
			panic(err)
		}
		if err := os.WriteFile(filepath.Join(dir, path), data, 0o644); err != nil {
			panic(err)
		}
	}
	write("board.scene.json", encDocument(document))
	data, err := json.Marshal(options)
	if err != nil {
		panic(err)
	}
	var fields map[string]any
	if err := json.Unmarshal(data, &fields); err != nil {
		panic(err)
	}
	if options.Background != nil {
		fields["Background"] = color.NRGBAModel.Convert(options.Background)
	}
	write("board.options.json", fields)
	write("case.json", map[string]any{"test": name, "index": index})
	d2rsCaptureLock.Unlock()
	frame, renderErr := Render(ctx, document, options)
	if renderErr != nil {
		if err := os.WriteFile(filepath.Join(dir, "board.err"), []byte(renderErr.Error()), 0o644); err != nil {
			panic(err)
		}
	} else {
		file, err := os.Create(filepath.Join(dir, "board.png"))
		if err != nil {
			panic(err)
		}
		if err := png.Encode(file, frame); err != nil {
			panic(err)
		}
		if err := file.Close(); err != nil {
			panic(err)
		}
	}
	return frame, renderErr
}
