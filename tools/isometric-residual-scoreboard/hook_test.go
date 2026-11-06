// SPDX-License-Identifier: MPL-2.0
// Observers run only in the pinned trace checkout; original assertions stay active.
package d2isometricimg

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
	"image"
	"math"
	"os"
	"path/filepath"
	"testing"
)

var residualTargets = map[*d2isometric.Scene]*d2target.Diagram{}
var residualOptions = map[*d2isometric.Scene]*d2isometric.RenderOpts{}
var residualScenes = map[*nativeScene]map[string]any{}
var residualPlans = map[*Raster][]Triangle{}

func residualScene(build func(*d2target.Diagram, *d2isometric.RenderOpts) (*d2isometric.Scene, error), d *d2target.Diagram, o *d2isometric.RenderOpts) (*d2isometric.Scene, error) {
	s, e := build(d, o)
	if e == nil {
		residualTargets[s] = d
		residualOptions[s] = o
	}
	return s, e
}
func residualPackets(ps []packetRoute) any {
	out := []any{}
	for _, p := range ps {
		out = append(out, map[string]any{"points": p.points, "lengths": p.lengths, "material": solidMesh([]Triangle{{Material: p.material}}, false), "forward": p.forward, "reverse": p.reverse})
	}
	return out
}
func residualEdges(b *meshBuilder, es []d2isometric.Edge, p *routeCaptionPlacer, scenes ...*d2isometric.Scene) []packetRoute {
	if len(p.rects) != 0 {
		panic("unexpected initial edge fixture")
	}
	req := shapeRequest(b, "edges")
	req["edges"], req["scenes"], req["icons"] = es, scenes, b.icons != nil
	if b.text != nil {
		if b.text.used != 0 {
			panic("unexpected used text painter")
		}
		req["text_count"] = b.text.count
	}
	ps := b.edges(es, p, scenes...)
	solidRecord(req, map[string]any{"state": shapeState(b, false), "packets": residualPackets(ps), "edges": es, "links": compositionLinks(b)})
	return ps
}
func residualNative(ctx context.Context, s *d2isometric.Scene, w, h int, assets *d2scenebuild.AssetOptions, fonts *d2scenebuild.FontFallbackOptions, o nativeSceneOptions) (*nativeScene, error) {
	if assets != nil || fonts != nil || o.camera != nil || o.links != nil {
		panic("unexpected native fixture")
	}
	d, ok := residualTargets[s]
	if !ok {
		panic("unobserved source")
	}
	req := map[string]any{"op": "native", "target": d, "render": residualOptions[s], "width": w, "height": h, "fit": o.fitContent, "defer": o.deferRaster, "vector": o.vector, "density": o.outputDensity, "retain_vectors": nativeVectorEnabled(ctx)}
	n, e := newNativeSceneWithOptions(ctx, s, w, h, assets, fonts, o)
	out := map[string]any{"error": rasterError(e), "scene": s, "target": d, "mesh": nil, "camera": nil, "packets": nil, "background": nil}
	if n != nil {
		out["mesh"], out["camera"], out["packets"], out["background"] = solidMesh(n.triangles, false), rasterCameraInput(&n.camera), residualPackets(n.packets), n.background
		residualScenes[n] = req
	}
	solidRecord(req, out)
	return n, e
}
func residualNativeDefault(ctx context.Context, s *d2isometric.Scene, w, h int) (*nativeScene, error) {
	return residualNative(ctx, s, w, h, nil, nil, nativeSceneOptions{})
}
func residualNativeFrame(n *nativeScene, ctx context.Context, at float64, traffic bool, packet ...float64) (*image.RGBA, error) {
	req := map[string]any{}
	for k, v := range residualScenes[n] {
		req[k] = v
	}
	req["op"], req["at"], req["traffic"], req["packet"] = "native-frame", at, traffic, packet
	p, e := n.Frame(ctx, at, traffic, packet...)
	solidRecord(req, map[string]any{"error": rasterError(e), "frame": rasterImageOutput(p)})
	return p, e
}
func residualFrameRaster(n *nativeScene, ctx context.Context, at float64, traffic bool, packet ...float64) (*Raster, error) {
	req := map[string]any{}
	for k, v := range residualScenes[n] {
		req[k] = v
	}
	req["op"], req["at"], req["traffic"], req["packet"] = "native-frame", at, traffic, packet
	r, e := n.frameRaster(ctx, at, traffic)
	var p *image.RGBA
	if r != nil {
		p = r.output
	}
	solidRecord(req, map[string]any{"error": rasterError(e), "frame": rasterImageOutput(p)})
	return r, e
}
func residualShadow(w *rasterWork, ts []Triangle) (rasterShadow, error) {
	req := map[string]any{"op": "shadow", "triangles": rasterTrianglesInput(ts), "remaining": w.remaining, "canceled": rasterContextInput(w.ctx)}
	s, e := rasterBuildShadow(w, ts)
	var out any
	if e == nil {
		depth := make([]byte, len(s.depth)*4)
		for i, n := range s.depth {
			binary.LittleEndian.PutUint32(depth[4*i:], math.Float32bits(n))
		}
		out = map[string]any{"camera": rasterCameraInput(&s.camera), "depth": fmt.Sprintf("%x", sha256.Sum256(depth)), "opacity": fmt.Sprintf("%x", sha256.Sum256(s.opacity)), "samples": len(s.depth), "bias": s.bias}
	}
	solidRecord(req, map[string]any{"error": rasterError(e), "remaining": w.remaining, "shadow": out})
	return s, e
}
func residualValidate(ctx context.Context, ts []Triangle) error {
	req := map[string]any{"op": "validate", "triangles": rasterTrianglesInput(ts), "canceled": rasterContextInput(ctx)}
	e := rasterValidate(ctx, ts)
	solidRecord(req, map[string]any{"error": rasterError(e)})
	return e
}
func residualPrepare(r *Raster, ts []Triangle) rasterDrawPlan {
	residualPlans[r] = ts
	return r.rasterPrepare(ts)
}
func residualDraw(r *Raster, w *rasterWork, p *image.RGBA, depth []float32, plan rasterDrawPlan, dirty []bool) error {
	if dirty != nil {
		panic("unexpected dirty fixture")
	}
	for _, n := range p.Pix {
		if n != 0 {
			panic("nonzero destination")
		}
	}
	for _, n := range depth {
		if n != 0 {
			panic("nonzero depth")
		}
	}
	req := map[string]any{"op": "draw", "triangles": rasterTrianglesInput(residualPlans[r]), "camera": rasterCameraInput(&r.camera), "remaining": w.remaining, "canceled": rasterContextInput(w.ctx), "width": p.Rect.Dx(), "height": p.Rect.Dy()}
	e := r.drawPrepared(w, p, depth, plan, dirty)
	solidRecord(req, map[string]any{"error": rasterError(e), "frame": rasterImageOutput(p)})
	return e
}

// Focused earliest-divergence witness for the two solid-contact caption failures.
var residualDiagnostic int

func (b *meshBuilder) routeCaptionKnockout(first int, s labelSurface) {
	req := shapeRequest(b, "knockout")
	req["first"], req["surface"] = first, solidSurface(s)
	b.d2rsOriginalCaptionKnockout(first, s)
	if !residualContactDiagnostic {
		return
	}
	data, e := json.Marshal(map[string]any{"request": req, "got": shapeState(b, false)})
	if e != nil {
		panic(e)
	}
	folder := filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "diagnostic")
	if e = os.MkdirAll(folder, 0700); e != nil {
		panic(e)
	}
	residualDiagnostic++
	if e = os.WriteFile(filepath.Join(folder, fmt.Sprintf("%03d.json", residualDiagnostic)), data, 0600); e != nil {
		panic(e)
	}
}

var residualContactDiagnostic bool

func residualContactTest(t *testing.T, n d2isometric.Node) {
	residualContactDiagnostic = true
	defer func() { residualContactDiagnostic = false }()
	testSolidContactArrowsTrafficAndMainCaption(t, n)
}
