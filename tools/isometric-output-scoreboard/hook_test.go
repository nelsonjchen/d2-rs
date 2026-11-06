// SPDX-License-Identifier: MPL-2.0
// Original-test observer installed only in the pinned trace checkout.
package d2isometricimg

import (
	"bytes"
	"context"
	"encoding/json"
	"image"
	"image/color"
	"os"
	"path/filepath"
	"testing"
	"time"
)

var outputRows []any

func d2rsGeometryStart(t *testing.T) {
	outputRows = []any{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]any{"name": t.Name(), "operations": outputRows})
		if err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func outputRecord(req, got any) {
	data, err := json.Marshal(map[string]any{"request": req, "got": got})
	if err != nil {
		panic(err)
	}
	outputRows = append(outputRows, json.RawMessage(data))
}
func outputError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func outputColors(p color.Palette) any {
	out := [][4]uint8{}
	for _, c := range p {
		v := color.NRGBAModel.Convert(c).(color.NRGBA)
		out = append(out, [4]uint8{v.R, v.G, v.B, v.A})
	}
	return out
}
func outputPalette(ctx context.Context, p color.Palette) ([]uint8, error) {
	v, err := paletteLookup(ctx, p)
	outputRecord(map[string]any{"op": "palette", "colors": outputColors(p), "canceled": ctx.Err() != nil}, map[string]any{"error": outputError(err), "lookup": v})
	return v, err
}
func outputPaletted(p *image.Paletted) any {
	if p == nil {
		return nil
	}
	return map[string]any{"rect": [4]int{p.Rect.Min.X, p.Rect.Min.Y, p.Rect.Max.X, p.Rect.Max.Y}, "stride": p.Stride, "pixels": p.Pix, "colors": outputColors(p.Palette)}
}
func outputIndex(ctx context.Context, frame image.Image, p color.Palette, lookup []uint8) (*image.Paletted, error) {
	f := frame.(*image.RGBA)
	req := map[string]any{"op": "index", "width": f.Rect.Dx(), "height": f.Rect.Dy(), "stride": f.Stride, "pixels": f.Pix, "colors": outputColors(p), "lookup": lookup, "canceled": ctx.Err() != nil}
	v, err := indexFrame(ctx, frame, p, lookup)
	outputRecord(req, map[string]any{"error": outputError(err), "image": outputPaletted(v)})
	return v, err
}
func outputDelta(before, after *image.Paletted) *image.Paletted {
	v := frameDelta(before, after)
	outputRecord(map[string]any{"op": "delta", "before": outputPaletted(before), "after": outputPaletted(after)}, outputPaletted(v))
	return v
}
func outputDelay(i int) int {
	v := frameDelay(i)
	outputRecord(map[string]any{"op": "delay", "index": i}, v)
	return v
}
func outputWrite(w *boundedWriter, p []byte) (int, error) {
	dst := w.w.(*bytes.Buffer)
	req := map[string]any{"op": "write", "remaining": w.remaining, "bytes": p, "destination": append([]byte{}, dst.Bytes()...), "canceled": w.ctx.Err() != nil}
	n, err := w.Write(p)
	outputRecord(req, map[string]any{"count": n, "error": outputError(err), "destination": append([]byte{}, dst.Bytes()...), "remaining": w.remaining})
	return n, err
}
func outputLoop(ctx context.Context, s *nativeScene, loop time.Duration) (*nativeScene, error) {
	// The original mask test has nil primitives/assets: ordinary d2scene JSON
	// preserves every field without an interface-value adapter.
	req := map[string]any{"op": "loop", "document": s.panels[0].document, "animated": s.panels[0].animated, "loop": int64(loop), "canceled": ctx.Err() != nil}
	data, err := json.Marshal(req)
	if err != nil {
		panic(err)
	}
	out, err := nativeGIFLoopScene(ctx, s, loop)
	got := map[string]any{"error": outputError(err), "source": s.panels[0].document}
	if out != nil {
		got["document"] = out.panels[0].document
		got["distinct_document"] = out.panels[0].document != s.panels[0].document
		got["distinct_root"] = out.panels[0].document.Root != s.panels[0].document.Root
		got["distinct_mask"] = out.panels[0].document.Root.Mask.Root != s.panels[0].document.Root.Mask.Root
	}
	outputRecord(json.RawMessage(data), got)
	return out, err
}
func outputExtent(e *projectedExtent) any {
	return map[string]any{"minX": e.minX, "minY": e.minY, "minZ": e.minZ, "maxX": e.maxX, "maxY": e.maxY, "maxZ": e.maxZ, "valid": e.valid}
}
func outputGround(e *projectedExtent, ts []Triangle, nodes []nativeAnimatedNode, scale float64, camera ...rasterCamera) {
	if len(nodes) != 0 || len(camera) != 0 {
		panic("unexpected original framing input")
	}
	triangles := []any{}
	for _, t := range ts {
		vertices := []Vec{}
		for _, v := range t.V {
			vertices = append(vertices, v.Position)
		}
		triangles = append(triangles, map[string]any{"vertices": vertices, "color": t.Material.Color, "cast": t.CastShadow, "opacity": t.ShadowOpacity})
	}
	req := map[string]any{"op": "ground", "extent": outputExtent(e), "triangles": triangles, "scale": scale}
	e.groundShadows(ts, nodes, scale, camera...)
	outputRecord(req, outputExtent(e))
}
