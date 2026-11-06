// SPDX-License-Identifier: MPL-2.0
// Installed only in the pinned trace checkout. Original assertions stay active.
package d2isometricimg

import "github.com/d2lang/d2/d2renderers/d2scene"

func pageStrokedPaths(root *d2scene.Node) []*d2scene.Stroke {
	paths := []*d2scene.Stroke{}
	var visit func(*d2scene.Node)
	visit = func(n *d2scene.Node) {
		if n == nil {
			return
		}
		if p, ok := n.Primitive.(d2scene.Path); ok && p.Stroke != nil {
			paths = append(paths, p.Stroke)
		}
		for _, child := range n.Children {
			visit(child)
		}
	}
	visit(root)
	return paths
}

func pageObserveRemoveGlyph(root *d2scene.Node) {
	request := map[string]any{"op": "remove-page-glyph", "root": contentEncNode(root)}
	before := pageStrokedPaths(root)
	nativeRemovePageGlyph(root)
	after := pageStrokedPaths(root)
	solidRecord(request, map[string]any{"root": contentEncNode(root),
		"outer_preserved": len(before) > 0 && len(after) > 0 && before[0] == after[0]})
}
