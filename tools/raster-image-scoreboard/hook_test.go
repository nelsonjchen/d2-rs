// SPDX-License-Identifier: MPL-2.0
package rasterimage

import (
	"bytes"
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
)

var d2rsImageMu sync.Mutex
var d2rsImageCounts = map[string]int{}

// Call the original API and return its original result to the original test.
// The extra encoding is exactly vector_surface_svg.go:rasterURL's boundary.
func d2rsDecodeFirst(t *testing.T, ctx context.Context, data []byte, format string) (image.Image, error) {
	result, err := DecodeFirst(ctx, data, format)
	root := os.Getenv("D2RS_IMAGE_CAPTURE")
	if root == "" {
		return result, err
	}
	d2rsImageMu.Lock()
	defer d2rsImageMu.Unlock()
	index := d2rsImageCounts[t.Name()]
	d2rsImageCounts[t.Name()]++
	directory := filepath.Join(root, "go", t.Name(), fmt.Sprint(index))
	if e := os.MkdirAll(directory, 0755); e != nil {
		t.Fatal(e)
	}
	write := func(name string, value []byte) {
		if e := os.WriteFile(filepath.Join(directory, name), value, 0644); e != nil {
			t.Fatal(e)
		}
	}
	request, e := json.Marshal(map[string]string{"test": t.Name(), "format": format})
	if e != nil {
		t.Fatal(e)
	}
	write("case.json", request)
	write("input.bin", data)
	if err != nil {
		write("expected.err", []byte(err.Error()))
	} else {
		var encoded bytes.Buffer
		if e := png.Encode(&encoded, result); e != nil {
			t.Fatal(e)
		}
		write("expected.png", encoded.Bytes())
	}
	return result, err
}

// One source/over pair isolates draw.Draw's NRGBA fast copy versus its
// RGBA64 premultiply/unpremultiply fallback on a partial animation frame.
func TestD2rsFirstFrameAlphaContracts(t *testing.T) {
	if os.Getenv("D2RS_IMAGE_CAPTURE") == "" {
		t.Skip("capture not requested")
	}
	poster := image.NewNRGBA(image.Rect(0, 0, 3, 2))
	frame := image.NewNRGBA(image.Rect(0, 0, 1, 1))
	frame.SetNRGBA(0, 0, color.NRGBA{R: 1, G: 2, B: 3, A: 1})
	for _, blend := range []byte{0, 1} {
		t.Run(fmt.Sprint(blend), func(t *testing.T) {
			chunks := parseTestPNGChunks(t, makeAPNG(t, poster, frame, image.Pt(2, 1)))
			chunks[pngTestChunkIndex(t, chunks, "fcTL", 0)].payload[25] = blend
			if _, err := d2rsDecodeFirst(t, context.Background(), encodeTestPNGChunks(chunks), "png"); err != nil {
				t.Fatal(err)
			}
		})
	}
}
