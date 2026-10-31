// SPDX-License-Identifier: MPL-2.0
// JSON encoding of d2scene values matching crates/d2-rs-scene's serde model:
// encoding/json defaults for plain structs, and interface values tagged by
// their Go type name ({"Rect": {...}}). Pointer variants contentEncode like values.
package d2isometricimg

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

type contentObject = map[string]any

// contentAssetStore holds content-addressed font bytes referenced by DataFile.
var contentAssetStore string

func contentEncPaint(p d2scene.Paint) any {
	switch v := p.(type) {
	case nil:
		return nil
	case d2scene.SolidPaint:
		return contentObject{"SolidPaint": contentObject{"Color": v.Color}}
	case *d2scene.SolidPaint:
		return contentEncPaint(*v)
	case d2scene.LinearGradient:
		return contentObject{"LinearGradient": v}
	case *d2scene.LinearGradient:
		return contentEncPaint(*v)
	case d2scene.RadialGradient:
		return contentObject{"RadialGradient": v}
	case *d2scene.RadialGradient:
		return contentEncPaint(*v)
	case d2scene.PatternPaint:
		return contentObject{"PatternPaint": contentObject{"Tile": v.Tile, "Root": contentEncNode(v.Root), "Units": v.Units, "Transform": v.Transform}}
	case *d2scene.PatternPaint:
		return contentEncPaint(*v)
	}
	panic(fmt.Sprintf("unsupported paint %T", p))
}

func contentEncStroke(s *d2scene.Stroke) any {
	if s == nil {
		return nil
	}
	return contentObject{"Paint": contentEncPaint(s.Paint), "Width": s.Width, "Cap": s.Cap, "Join": s.Join,
		"MiterLimit": s.MiterLimit, "Dashes": s.Dashes, "DashOffset": s.DashOffset}
}

func contentEncPath(p d2scene.Path) contentObject {
	return contentObject{"Commands": p.Commands, "FillRule": p.FillRule, "Fill": contentEncPaint(p.Fill), "Stroke": contentEncStroke(p.Stroke)}
}

func contentEncPrimitive(p d2scene.Primitive) any {
	switch v := p.(type) {
	case nil:
		return nil
	case d2scene.Rect:
		return contentObject{"Rect": contentObject{"Box": v.Box, "RadiusX": v.RadiusX, "RadiusY": v.RadiusY, "Fill": contentEncPaint(v.Fill), "Stroke": contentEncStroke(v.Stroke)}}
	case *d2scene.Rect:
		return contentEncPrimitive(*v)
	case d2scene.Ellipse:
		return contentObject{"Ellipse": contentObject{"Center": v.Center, "RadiusX": v.RadiusX, "RadiusY": v.RadiusY, "Fill": contentEncPaint(v.Fill), "Stroke": contentEncStroke(v.Stroke)}}
	case *d2scene.Ellipse:
		return contentEncPrimitive(*v)
	case d2scene.Path:
		return contentObject{"Path": contentEncPath(v)}
	case *d2scene.Path:
		return contentEncPrimitive(*v)
	case d2scene.TextRun:
		return contentObject{"TextRun": contentObject{"Text": v.Text, "Origin": v.Origin, "Anchor": v.Anchor, "Font": v.Font,
			"Fallbacks": v.Fallbacks, "Fill": contentEncPaint(v.Fill), "Stroke": contentEncStroke(v.Stroke), "Underline": v.Underline,
			"Strike": v.Strike, "Glyphs": v.Glyphs, "Ink": v.Ink}}
	case *d2scene.TextRun:
		return contentEncPrimitive(*v)
	case d2scene.Image:
		return contentObject{"Image": v}
	case *d2scene.Image:
		return contentEncPrimitive(*v)
	}
	panic(fmt.Sprintf("unsupported primitive %T", p))
}

func contentEncFilter(f d2scene.Filter) any {
	switch v := f.(type) {
	case d2scene.GaussianBlur:
		return contentObject{"GaussianBlur": v}
	case *d2scene.GaussianBlur:
		return contentEncFilter(*v)
	case d2scene.DropShadow:
		return contentObject{"DropShadow": v}
	case *d2scene.DropShadow:
		return contentEncFilter(*v)
	}
	panic(fmt.Sprintf("unsupported filter %T", f))
}

func contentEncNode(n *d2scene.Node) any {
	if n == nil {
		return nil
	}
	out := contentObject{"ID": n.ID, "Classes": n.Classes, "Transform": n.Transform, "Opacity": n.Opacity, "Blend": n.Blend,
		"Primitive": contentEncPrimitive(n.Primitive), "Animations": n.Animations}
	if n.Clip != nil {
		out["Clip"] = contentObject{"Path": contentEncPath(n.Clip.Path), "Transform": n.Clip.Transform}
	} else {
		out["Clip"] = nil
	}
	if n.Mask != nil {
		out["Mask"] = contentObject{"Type": n.Mask.Type, "Root": contentEncNode(n.Mask.Root), "Transform": n.Mask.Transform}
	} else {
		out["Mask"] = nil
	}
	if n.Filters != nil {
		filters := make([]any, len(n.Filters))
		for i, f := range n.Filters {
			filters[i] = contentEncFilter(f)
		}
		out["Filters"] = filters
	} else {
		out["Filters"] = nil
	}
	if n.Children != nil {
		children := make([]any, len(n.Children))
		for i, c := range n.Children {
			children[i] = contentEncNode(c)
		}
		out["Children"] = children
	} else {
		out["Children"] = nil
	}
	return out
}

func contentEncAsset(a d2scene.Asset) any {
	switch v := a.(type) {
	case d2scene.FontAsset:
		// Fonts repeat across every document: keep one content-addressed copy.
		sum := sha256.Sum256(v.Data)
		name := hex.EncodeToString(sum[:])
		path := filepath.Join(contentAssetStore, name)
		if _, err := os.Stat(path); err != nil {
			if err := os.MkdirAll(contentAssetStore, 0o755); err != nil {
				panic(err)
			}
			if err := os.WriteFile(path, v.Data, 0o644); err != nil {
				panic(err)
			}
		}
		return contentObject{"FontAsset": contentObject{"MIMEType": v.MIMEType, "DataFile": path, "FaceIndex": v.FaceIndex}}
	case *d2scene.FontAsset:
		return contentEncAsset(*v)
	case d2scene.RasterAsset:
		return contentObject{"RasterAsset": v}
	case *d2scene.RasterAsset:
		return contentEncAsset(*v)
	case d2scene.VectorAsset:
		return contentObject{"VectorAsset": contentObject{"ViewBox": v.ViewBox, "Root": contentEncNode(v.Root)}}
	case *d2scene.VectorAsset:
		return contentEncAsset(*v)
	}
	panic(fmt.Sprintf("unsupported asset %T", a))
}

func contentEncDocument(d *d2scene.Document) any {
	assets := contentObject{}
	for id, a := range d.Assets {
		assets[string(id)] = contentEncAsset(a)
	}
	return contentObject{"ViewBox": d.ViewBox, "LogicalWidth": d.LogicalWidth, "LogicalHeight": d.LogicalHeight,
		"ViewportFit": d.ViewportFit, "ViewportAlign": d.ViewportAlign, "Root": contentEncNode(d.Root),
		"Assets": assets, "Links": d.Links}
}
