// SPDX-License-Identifier: MPL-2.0
// Go 1.27 encoder oracle for crates/d2-rs-export/src/go_png.rs: image/png
// BestSpeed bytes and the fpdf v0.12.0-equivalent RGB/alpha plane zlib
// streams for six deterministic RGBA images. Standard library only, so the
// output depends on the Go toolchain alone, not the D2 pin. Harvested from
// .reference/codex-runs/witness-inputs/run34-encoders.go (its PPTX ZIP half
// is not needed: zip-encoding-go127.zip stays a small committed template).
// Run via scripts/regen-fixtures.py (recipe renderexport-png-encoding).
package main

import (
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"encoding/json"
	"image"
	"image/png"
	"io"
	"os"
	"runtime"
)

type pngCase struct {
	Name     string `json:"name"`
	Width    int    `json:"width"`
	Height   int    `json:"height"`
	RGBA     []byte `json:"rgba"`
	PNG      []byte `json:"png"`
	PDFColor []byte `json:"pdf_color"`
	PDFAlpha []byte `json:"pdf_alpha,omitempty"`
}

func main() {
	if runtime.Version() != "go1.27.0" {
		panic("requires pinned go1.27.0; got " + runtime.Version())
	}
	var cases []pngCase
	for _, spec := range []struct {
		name          string
		w, h          int
		opaque, noise bool
	}{
		{"transparent-small", 2, 2, false, false}, {"opaque-small", 3, 3, true, false},
		{"opaque-pattern", 137, 113, true, false}, {"alpha-pattern", 137, 113, false, false},
		{"opaque-multiple-idat", 129, 191, true, true}, {"alpha-multiple-idat", 129, 191, false, true},
	} {
		img := image.NewNRGBA(image.Rect(0, 0, spec.w, spec.h))
		state := uint32(0x12345678)
		for i := range img.Pix {
			state = state*1664525 + 1013904223
			v := byte((i/4 + i/spec.w) % 256)
			if spec.noise {
				v = byte(state >> 24)
			}
			img.Pix[i] = v
		}
		if spec.opaque {
			for i := 3; i < len(img.Pix); i += 4 {
				img.Pix[i] = 255
			}
		}
		var out bytes.Buffer
		if err := (&png.Encoder{CompressionLevel: png.BestSpeed}).Encode(&out, img); err != nil {
			panic(err)
		}
		encoded := out.Bytes()
		var idats []byte
		for pos := 8; pos < len(encoded); {
			n := int(binary.BigEndian.Uint32(encoded[pos : pos+4]))
			if string(encoded[pos+4:pos+8]) == "IDAT" {
				idats = append(idats, encoded[pos+8:pos+8+n]...)
			}
			pos += 12 + n
		}
		colorData := idats
		var alphaData []byte
		if !spec.opaque {
			reader, err := zlib.NewReader(bytes.NewReader(idats))
			if err != nil {
				panic(err)
			}
			rows, err := io.ReadAll(reader)
			if err != nil {
				panic(err)
			}
			reader.Close()
			var colorRows, alphaRows []byte
			for y := 0; y < spec.h; y++ {
				row := rows[y*(1+spec.w*4) : (y+1)*(1+spec.w*4)]
				colorRows = append(colorRows, row[0])
				alphaRows = append(alphaRows, row[0])
				for x := 0; x < spec.w; x++ {
					colorRows = append(colorRows, row[1+x*4:1+x*4+3]...)
					alphaRows = append(alphaRows, row[1+x*4+3])
				}
			}
			compress := func(data []byte) []byte {
				var b bytes.Buffer
				w, err := zlib.NewWriterLevel(&b, zlib.BestSpeed)
				if err != nil {
					panic(err)
				}
				if _, err := w.Write(data); err != nil {
					panic(err)
				}
				if err := w.Close(); err != nil {
					panic(err)
				}
				return b.Bytes()
			}
			colorData = compress(colorRows)
			alphaData = compress(alphaRows)
		}
		cases = append(cases, pngCase{spec.name, spec.w, spec.h, img.Pix, encoded, colorData, alphaData})
	}
	out, err := json.Marshal(cases)
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(os.Args[1], out, 0o644); err != nil {
		panic(err)
	}
}
