// SPDX-License-Identifier: MPL-2.0
// internal/rasterimage DecodeFirst followed by png.Encode, as the d2isometric
// and raster PNG adapters use it (recipes mathtext-raster-static-png and
// mathtext-raster-apng-alpha). Adapted from tools/isometric-probe/
// png_contract_test.go and tools/raster-image-scoreboard/hook_test.go.
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
	"testing"
)

func d2rsPNGOutput(t *testing.T) string {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	return out
}

func d2rsDecodeEncode(t *testing.T, input []byte) []byte {
	decoded, err := DecodeFirst(context.Background(), input, "png")
	if err != nil {
		t.Fatal(err)
	}
	var expected bytes.Buffer
	if err := png.Encode(&expected, decoded); err != nil {
		t.Fatal(err)
	}
	return expected.Bytes()
}

func d2rsWritePNGRecords(t *testing.T, out string, records any) {
	data, err := json.MarshalIndent(records, "", " ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// Static paletted (2, 4, 16, 17 colours), NRGBA (opaque, translucent) and
// Gray16 PNGs, encoded with BestSpeed.
func TestD2RSStaticPNG(t *testing.T) {
	out := d2rsPNGOutput(t)
	type witness struct {
		Name            string
		Input, Expected []byte
	}
	records := []witness{}
	add := func(name string, img image.Image) {
		var input bytes.Buffer
		if err := (&png.Encoder{CompressionLevel: png.BestSpeed}).Encode(&input, img); err != nil {
			t.Fatal(err)
		}
		records = append(records, witness{name, input.Bytes(), d2rsDecodeEncode(t, input.Bytes())})
	}
	for _, count := range []int{2, 4, 16, 17} {
		palette := make(color.Palette, count)
		for i := range palette {
			palette[i] = color.NRGBA{uint8(i * 13), uint8(i * 7), uint8(255 - i), 255}
		}
		palette[count-1] = color.NRGBA{70, 90, 110, 80}
		p := image.NewPaletted(image.Rect(0, 0, 7, 3), palette)
		for i := range p.Pix {
			p.Pix[i] = uint8(i % count)
		}
		add(string(rune('a'+count)), p)
	}
	for _, alpha := range []uint8{255, 80} {
		p := image.NewNRGBA(image.Rect(0, 0, 7, 3))
		for y := 0; y < 3; y++ {
			for x := 0; x < 7; x++ {
				p.SetNRGBA(x, y, color.NRGBA{uint8(x * 20), uint8(y * 30), 123, alpha})
			}
		}
		add(string(rune(alpha)), p)
	}
	gray := image.NewGray16(image.Rect(0, 0, 7, 3))
	for y := 0; y < 3; y++ {
		for x := 0; x < 7; x++ {
			gray.SetGray16(x, y, color.Gray16{uint16(x*9000 + y)})
		}
	}
	add("gray16", gray)
	d2rsWritePNGRecords(t, out, records)
}

// One source/over pair isolates draw.Draw's NRGBA fast copy versus its
// RGBA64 premultiply/unpremultiply fallback on a partial animation frame.
func TestD2RSAPNGAlpha(t *testing.T) {
	out := d2rsPNGOutput(t)
	type record struct {
		Name     string `json:"name"`
		Input    []byte `json:"input"`
		Expected []byte `json:"expected"`
	}
	records := []record{}
	poster := image.NewNRGBA(image.Rect(0, 0, 3, 2))
	frame := image.NewNRGBA(image.Rect(0, 0, 1, 1))
	frame.SetNRGBA(0, 0, color.NRGBA{R: 1, G: 2, B: 3, A: 1})
	for _, blend := range []byte{0, 1} {
		chunks := parseTestPNGChunks(t, makeAPNG(t, poster, frame, image.Pt(2, 1)))
		chunks[pngTestChunkIndex(t, chunks, "fcTL", 0)].payload[25] = blend
		input := encodeTestPNGChunks(chunks)
		records = append(records, record{fmt.Sprintf("TestD2rsFirstFrameAlphaContracts/%d/0", blend), input, d2rsDecodeEncode(t, input)})
	}
	d2rsWritePNGRecords(t, out, records)
}
