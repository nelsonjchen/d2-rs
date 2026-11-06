// SPDX-License-Identifier: MPL-2.0
// Original test observers; installed only in the pinned trace checkout.
package d2isometricimg

import "github.com/d2lang/d2/d2renderers/d2isometric"

func hierObserveBoards(bs []d2isometric.Board, ns map[string]*d2isometric.Node) []d2isometric.Board {
	got := hierarchyPresentationBoards(bs, ns)
	solidRecord(map[string]any{"op": "boards", "boards": bs, "nodes": ns}, map[string]any{"boards": got, "source": bs, "nodes": ns})
	return got
}
func hierObserveRenderNodes(ns []d2isometric.Node, supports ...map[string]float64) []d2isometric.Node {
	got := hierarchyRenderNodes(ns, supports...)
	solidRecord(map[string]any{"op": "render-nodes", "nodes": ns, "supports": supports}, map[string]any{"nodes": got, "source": ns, "supports": supports})
	return got
}
func hierObserveTier(depth int) float64 {
	got := hierarchyTierHeight(depth)
	solidRecord(map[string]any{"op": "tier", "depth": depth}, got)
	return got
}
func hierObserveSupports(bs []d2isometric.Board) map[string]float64 {
	got := hierarchySupportOffsets(bs)
	solidRecord(map[string]any{"op": "supports", "boards": bs}, got)
	return got
}
func hierObserveInk(b d2isometric.Board, bs map[string]d2isometric.Board, ns map[string]*d2isometric.Node, tints map[string]string, background string) string {
	got := hierarchyHeaderInk(b, bs, ns, tints, background)
	solidRecord(map[string]any{"op": "header-ink", "board": b, "boards": bs, "nodes": ns, "tints": tints, "background": background}, got)
	return got
}
func hierObserveSpacer(n d2isometric.Node) bool {
	got := hierarchySpacer(n)
	solidRecord(map[string]any{"op": "spacer", "node": n}, got)
	return got
}
func hierObserveRelief(n d2isometric.Node) float64 {
	got := hierarchyNodeRelief(n)
	solidRecord(map[string]any{"op": "relief", "node": n}, got)
	return got
}
func hierObservePlace(op string, s labelSurface, b d2isometric.Board, n d2isometric.Node, ns []d2isometric.Node, scale float64, bs ...[]d2isometric.Board) (labelSurface, bool) {
	got, fits := hierarchyBoardHeaderPlacement(s, b, n, ns, scale, bs...)
	var fit any
	if op == "placement" {
		fit = fits
	}
	solidRecord(map[string]any{"op": op, "surface": solidSurface(s), "board": b, "owner": n, "nodes": ns, "scale": scale, "boards": bs}, map[string]any{"surface": solidSurface(got), "fits": fit, "board": b, "owner": n, "nodes": ns})
	return got, fits
}
func hierObservePlacement(s labelSurface, b d2isometric.Board, n d2isometric.Node, ns []d2isometric.Node, scale float64, bs ...[]d2isometric.Board) (labelSurface, bool) {
	return hierObservePlace("placement", s, b, n, ns, scale, bs...)
}
func hierObserveHeader(s labelSurface, b d2isometric.Board, n d2isometric.Node, ns []d2isometric.Node, scale float64, bs ...[]d2isometric.Board) labelSurface {
	got, _ := hierObservePlace("header", s, b, n, ns, scale, bs...)
	return got
}
func hierRich(b *meshBuilder) any {
	p := b.rich
	if p == nil {
		return nil
	}
	return map[string]any{"count": p.count, "used": p.used, "pixels": p.pixels, "bytes": p.sourceBytes, "rows": p.rows, "primary": p.primary, "mono": p.mono, "theme": p.themeID, "overrides": p.themeOverrides}
}
func hierState(b *meshBuilder, raw bool) any {
	return map[string]any{"state": shapeState(b, raw), "rich": hierRich(b), "supports": b.hierarchySupports, "drop": b.nodeSupportDrop}
}
func hierRequest(b *meshBuilder, op string) map[string]any {
	r := shapeRequest(b, op)
	r["rich"], r["supports"], r["drop"] = hierRich(b), b.hierarchySupports, b.nodeSupportDrop
	r["retainVector"] = nativeVectorEnabled(b.ctx)
	return r
}
func hierObserveNode(b *meshBuilder, op string, n d2isometric.Node, tint string) {
	r := hierRequest(b, op)
	r["node"], r["tint"] = n, tint
	if op == "node" {
		b.node(n, tint)
	} else {
		b.hierarchyNode(n, tint)
	}
	solidRecord(r, map[string]any{"builder": hierState(b, false), "node": n})
}
func hierObserveBoard(b *meshBuilder, board d2isometric.Board, n *d2isometric.Node, tint string, opacity float64) {
	r := hierRequest(b, "board")
	r["board"], r["node"], r["tint"], r["opacity"] = board, n, tint, opacity
	b.hierarchyBoard(board, n, tint, opacity)
	solidRecord(r, hierState(b, false))
}
func hierCamera(c rasterCamera) any {
	return map[string]any{"right": c.right, "up": c.up, "direction": c.direction, "centerX": c.centerX, "centerY": c.centerY, "centerDepth": c.centerDepth, "scale": c.scale, "width": c.width, "height": c.height}
}
func hierObserveLighting(c rasterCamera, t Triangle) ([3]float64, [3]float64) {
	a, b := svgTextureLighting(c, t)
	solidRecord(map[string]any{"op": "texture-lighting", "camera": hierCamera(c), "mesh": solidMesh([]Triangle{t}, true)}, map[string]any{"slope": a, "intercept": b})
	return a, b
}
func hierPaint(p rasterPaint) any {
	if p.texture != nil {
		panic("textured lighting input needs a pixel bridge")
	}
	return map[string]any{"base": p.base, "emission": p.emission, "roughness": p.roughness, "metalness": p.metalness, "unlit": p.unlit, "alpha": p.alpha, "multiply": p.multiply}
}
func hierVertex(v svgLightingVertex) any {
	return map[string]any{"point": []float64{v.point.x, v.point.y, v.point.z}, "position": v.position, "normal": v.normal}
}
func hierVertices(v [3]svgLightingVertex) any {
	return []any{hierVertex(v[0]), hierVertex(v[1]), hierVertex(v[2])}
}
func hierGradient(g svgLightingGradient) any {
	stops := []any{}
	for _, s := range g.stops {
		stops = append(stops, map[string]any{"offset": s.offset, "color": s.color})
	}
	return map[string]any{"start": []float64{g.start.x, g.start.y, g.start.z}, "end": []float64{g.end.x, g.end.y, g.end.z}, "values": g.values, "valid": g.valid, "stops": stops}
}
func hierObserveGradient(r *Raster, p rasterPaint, v [3]svgLightingVertex) (svgLightingGradient, float64) {
	g, e := svgFitLightingGradient(r, p, v)
	solidRecord(map[string]any{"op": "gradient", "camera": hierCamera(r.camera), "paint": hierPaint(p), "vertices": hierVertices(v)}, map[string]any{"gradient": hierGradient(g), "error": e})
	return g, e
}
func hierObservePatches(r *Raster, p rasterPaint, v [3]svgLightingVertex) []svgLightingPatch {
	ps := svgLightingPatches(r, p, v)
	out := []any{}
	for _, x := range ps {
		out = append(out, map[string]any{"vertices": hierVertices(x.vertices), "gradient": hierGradient(x.gradient)})
	}
	solidRecord(map[string]any{"op": "patches", "camera": hierCamera(r.camera), "paint": hierPaint(p), "vertices": hierVertices(v)}, out)
	return ps
}
func hierObserveCaster(t Triangle) uint8 {
	got := svgCasterOpacity(t)
	solidRecord(map[string]any{"op": "caster", "mesh": solidMesh([]Triangle{t}, true)}, got)
	return got
}
func hierObserveGroundCaster(t Triangle, g, r float64, c rasterCamera, l Vec) []svgPoint {
	got := svgGroundCaster(t, g, r, c, l)
	ps := []any{}
	for _, p := range got {
		ps = append(ps, []float64{p.x, p.y, p.z})
	}
	solidRecord(map[string]any{"op": "ground-caster", "mesh": solidMesh([]Triangle{t}, true), "ground": g, "receiver": r, "camera": hierCamera(c), "light": l}, ps)
	return got
}
func hierObserveShadow(w *nativeSVGWriter, s *nativeScene) {
	req := map[string]any{"op": "shadow", "camera": hierCamera(s.camera), "mesh": solidMesh(s.triangles, true), "width": s.width, "height": s.height, "prefix": w.buf.String()}
	writeSVGGroundShadow(w, s)
	err := ""
	if w.err != nil {
		err = w.err.Error()
	}
	solidRecord(req, map[string]any{"svg": w.buf.String(), "error": err})
}
