// SPDX-License-Identifier: MPL-2.0
// Original-test observer installed only in the pinned instrumentation checkout.
package d2isometricimg

import (
	"context"
	"image"
	"net/url"
	"os"
	"path/filepath"

	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/imageasset"
)

var artworkPainters = map[*surfaceIconPainter]int{}
var artworkResolvers = map[*imageasset.Resolver]any{}
var artworkTextures = map[*image.RGBA]int{}
var artworkTargets = map[*d2isometric.Scene]*d2target.Diagram{}

func artworkError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func artworkResolver(o imageasset.Options) (*imageasset.Resolver, error) {
	r, e := imageasset.New(o)
	if e == nil {
		root, ok := o.LocalFiles.RootPath()
		if !ok {
			panic("expected explicit rooted policy")
		}
		files := map[string][]byte{}
		entries, e := os.ReadDir(root)
		if e != nil {
			panic(e)
		}
		for _, entry := range entries {
			if entry.IsDir() {
				panic("unexpected nested asset directory")
			}
			b, e := os.ReadFile(filepath.Join(root, entry.Name()))
			if e != nil {
				panic(e)
			}
			files[entry.Name()] = b
		}
		artworkResolvers[r] = map[string]any{"root": root, "limits": o.Limits, "files": files}
	}
	return r, e
}
func artworkNewIcon(ctx context.Context, count int, assets *d2scenebuild.AssetOptions) (*surfaceIconPainter, error) {
	var supplied any
	if assets != nil {
		var ok bool
		supplied, ok = artworkResolvers[assets.Resolver]
		if !ok {
			panic("unobserved resolver")
		}
	}
	p, e := newSurfaceIconPainter(ctx, count, assets)
	id := 0
	if p != nil {
		id = len(artworkPainters) + 1
		artworkPainters[p] = id
		if assets == nil {
			artworkResolvers[p.resolver] = map[string]any{"owner": id}
		}
	}
	solidRecord(map[string]any{"op": "icon_new", "id": id, "count": count, "assets": supplied}, map[string]any{"error": artworkError(e), "nil": p == nil})
	return p, e
}
func artworkTexture(p *surfaceIconPainter, u *url.URL, w, d float64, radius ...float64) (*image.RGBA, error) {
	id, ok := artworkPainters[p]
	if !ok {
		panic("unobserved painter")
	}
	req := map[string]any{"op": "icon", "id": id, "source": u, "width": w, "depth": d, "radius": radius}
	if u.Scheme == "" {
		data, err := os.ReadFile(u.Path)
		if err != nil {
			panic(err)
		}
		req["local_file"] = data
	}
	t, e := p.texture(u, w, d, radius...)
	tid := 0
	if t != nil {
		var ok bool
		tid, ok = artworkTextures[t]
		if !ok {
			tid = len(artworkTextures) + 1
			artworkTextures[t] = tid
		}
	}
	solidRecord(req, map[string]any{"error": artworkError(e), "texture": solidTexture(t, false), "id": tid})
	return t, e
}
func artworkScene(d *d2target.Diagram, o *d2isometric.RenderOpts) (*d2isometric.Scene, error) {
	s, e := d2isometric.BuildScene(d, o)
	if e == nil {
		artworkTargets[s] = d
	}
	return s, e
}
func artworkCapture(ctx context.Context, s *d2isometric.Scene, w, h int, assets *d2scenebuild.AssetOptions, fonts *d2scenebuild.FontFallbackOptions, o nativeSceneOptions) (*nativeScene, error) {
	if assets != nil || fonts != nil || !o.deferRaster || !o.vector {
		panic("unexpected capture options")
	}
	d, ok := artworkTargets[s]
	if !ok {
		panic("unobserved source scene")
	}
	n, e := newNativeSceneWithOptions(ctx, s, w, h, assets, fonts, o)
	out := map[string]any{"error": artworkError(e), "scene": s, "target": d, "mesh": nil, "surface_ids": nil, "surfaces": nil}
	if n != nil {
		ids := map[*nativeVectorSurface]int{}
		bindings := []int{}
		fragments := []any{}
		for _, t := range n.triangles {
			id := 0
			if t.Material != nil && t.Material.Vector != nil {
				surface := t.Material.Vector
				var ok bool
				id, ok = ids[surface]
				if !ok {
					id = len(ids) + 1
					ids[surface] = id
					fragment, err := nativeSurfaceSVG(ctx, surface, "fixture")
					fragments = append(fragments, map[string]any{"svg": fragment, "error": artworkError(err)})
				}
			}
			bindings = append(bindings, id)
		}
		out["mesh"] = solidMesh(n.triangles, false)
		out["surface_ids"] = bindings
		out["surfaces"] = fragments
	}
	solidRecord(map[string]any{"op": "capture", "target": d, "width": w, "height": h}, out)
	return n, e
}
