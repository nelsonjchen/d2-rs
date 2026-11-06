// SPDX-License-Identifier: MPL-2.0
// Observe original calls only in the pinned instrumentation checkout.
package d2isometricimg

import (
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scene"
)

var panelSessions = map[*meshBuilder]int{}
var panelTextSessions = map[*textPainter]int{}
var panelIconSessions = map[*surfaceIconPainter]int{}

func panelState(b *meshBuilder) any {
	panels := []any{}
	for _, p := range b.panels {
		panels = append(panels, map[string]any{"document": contentEncDocument(p.document), "width": p.width, "height": p.height, "first": p.first, "last": p.last, "animated": p.animated})
	}
	if len(b.links) != 0 {
		panic("unexpected link input")
	}
	return map[string]any{"mesh": solidMesh(b.triangles, false), "panels": panels, "error": solidErr(b), "pixels": b.facePixels}
}
func panelObserve(b *meshBuilder, doc *d2scene.Document, area labelSurface, planned int) error {
	id, ok := panelSessions[b]
	if !ok {
		id = len(panelSessions) + 1
		panelSessions[b] = id
	}
	request := map[string]any{"op": "panel", "id": id, "density": b.outputDensity, "document": contentEncDocument(doc), "area": solidSurface(area), "planned": planned}
	err := b.sourcePanel(doc, area, planned)
	message := ""
	if err != nil {
		message = err.Error()
	}
	solidRecord(request, map[string]any{"error": message, "state": panelState(b), "source": contentEncDocument(doc)})
	return err
}
func d2rsPanelObserveNode(b *meshBuilder, op string, n d2isometric.Node, tint string, relief ...float64) {
	request := map[string]any{"op": op, "node": n, "tint": tint, "relief": relief, "state": solidState(b, true), "text": nil, "icons": nil}
	if b.text != nil {
		id, ok := panelTextSessions[b.text]
		if !ok {
			id = len(panelTextSessions) + 1
			panelTextSessions[b.text] = id
		}
		if b.text.primary != "" || b.text.mono != "" || b.text.fallbackFonts != nil {
			panic("unexpected configured font input")
		}
		request["text"] = map[string]any{"id": id, "count": b.text.count}
	}
	if b.icons != nil {
		id, ok := panelIconSessions[b.icons]
		if !ok {
			id = len(panelIconSessions) + 1
			panelIconSessions[b.icons] = id
		}
		request["icons"] = map[string]any{"id": id, "count": b.icons.count}
	}
	if op == "hierarchy" {
		b.hierarchyNode(n, tint)
	} else {
		b.node(n, tint, relief...)
	}
	solidRecord(request, map[string]any{"state": solidState(b, false), "node": n})
}
