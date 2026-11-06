// SPDX-License-Identifier: MPL-2.0
// JSON encoding of d2scene values matching crates/d2-rs-scene's serde model:
// encoding/json defaults for plain structs, and interface values tagged by
// their Go type name ({"Rect": {...}}). Pointer variants encode like values.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

type object = map[string]any

// assetStore holds content-addressed font bytes referenced by DataFile.
var assetStore string

func encPaint(p d2scene.Paint) any {
	switch v := p.(type) {
	case nil:
		return nil
	case d2scene.SolidPaint:
		return object{"SolidPaint": object{"Color": v.Color}}
	case *d2scene.SolidPaint:
		return encPaint(*v)
	case d2scene.LinearGradient:
		return object{"LinearGradient": v}
	case *d2scene.LinearGradient:
		return encPaint(*v)
	case d2scene.RadialGradient:
		return object{"RadialGradient": v}
	case *d2scene.RadialGradient:
		return encPaint(*v)
	case d2scene.PatternPaint:
		return object{"PatternPaint": object{"Tile": v.Tile, "Root": encNode(v.Root), "Units": v.Units, "Transform": v.Transform}}
	case *d2scene.PatternPaint:
		return encPaint(*v)
	}
	panic(fmt.Sprintf("unsupported paint %T", p))
}

func encStroke(s *d2scene.Stroke) any {
	if s == nil {
		return nil
	}
	return object{"Paint": encPaint(s.Paint), "Width": s.Width, "Cap": s.Cap, "Join": s.Join,
		"MiterLimit": s.MiterLimit, "Dashes": s.Dashes, "DashOffset": s.DashOffset}
}

func encPath(p d2scene.Path) object {
	return object{"Commands": p.Commands, "FillRule": p.FillRule, "Fill": encPaint(p.Fill), "Stroke": encStroke(p.Stroke)}
}

func encPrimitive(p d2scene.Primitive) any {
	switch v := p.(type) {
	case nil:
		return nil
	case d2scene.Rect:
		return object{"Rect": object{"Box": v.Box, "RadiusX": v.RadiusX, "RadiusY": v.RadiusY, "Fill": encPaint(v.Fill), "Stroke": encStroke(v.Stroke)}}
	case *d2scene.Rect:
		return encPrimitive(*v)
	case d2scene.Ellipse:
		return object{"Ellipse": object{"Center": v.Center, "RadiusX": v.RadiusX, "RadiusY": v.RadiusY, "Fill": encPaint(v.Fill), "Stroke": encStroke(v.Stroke)}}
	case *d2scene.Ellipse:
		return encPrimitive(*v)
	case d2scene.Path:
		return object{"Path": encPath(v)}
	case *d2scene.Path:
		return encPrimitive(*v)
	case d2scene.TextRun:
		return object{"TextRun": object{"Text": v.Text, "Origin": v.Origin, "Anchor": v.Anchor, "Font": v.Font,
			"Fallbacks": v.Fallbacks, "Fill": encPaint(v.Fill), "Stroke": encStroke(v.Stroke), "Underline": v.Underline,
			"Strike": v.Strike, "Glyphs": v.Glyphs, "Ink": v.Ink}}
	case *d2scene.TextRun:
		return encPrimitive(*v)
	case d2scene.Image:
		return object{"Image": v}
	case *d2scene.Image:
		return encPrimitive(*v)
	}
	panic(fmt.Sprintf("unsupported primitive %T", p))
}

func encFilter(f d2scene.Filter) any {
	switch v := f.(type) {
	case d2scene.GaussianBlur:
		return object{"GaussianBlur": v}
	case *d2scene.GaussianBlur:
		return encFilter(*v)
	case d2scene.DropShadow:
		return object{"DropShadow": v}
	case *d2scene.DropShadow:
		return encFilter(*v)
	}
	panic(fmt.Sprintf("unsupported filter %T", f))
}

func encNode(n *d2scene.Node) any {
	if n == nil {
		return nil
	}
	out := object{"ID": n.ID, "Classes": n.Classes, "Transform": n.Transform, "Opacity": n.Opacity, "Blend": n.Blend,
		"Primitive": encPrimitive(n.Primitive), "Animations": n.Animations}
	if n.Clip != nil {
		out["Clip"] = object{"Path": encPath(n.Clip.Path), "Transform": n.Clip.Transform}
	} else {
		out["Clip"] = nil
	}
	if n.Mask != nil {
		out["Mask"] = object{"Type": n.Mask.Type, "Root": encNode(n.Mask.Root), "Transform": n.Mask.Transform}
	} else {
		out["Mask"] = nil
	}
	if n.Filters != nil {
		filters := make([]any, len(n.Filters))
		for i, f := range n.Filters {
			filters[i] = encFilter(f)
		}
		out["Filters"] = filters
	} else {
		out["Filters"] = nil
	}
	if n.Children != nil {
		children := make([]any, len(n.Children))
		for i, c := range n.Children {
			children[i] = encNode(c)
		}
		out["Children"] = children
	} else {
		out["Children"] = nil
	}
	return out
}

func encAsset(a d2scene.Asset) any {
	switch v := a.(type) {
	case d2scene.FontAsset:
		// Fonts repeat across every document: keep one content-addressed copy.
		sum := sha256.Sum256(v.Data)
		name := hex.EncodeToString(sum[:])
		path := filepath.Join(assetStore, name)
		if _, err := os.Stat(path); err != nil {
			if err := os.MkdirAll(assetStore, 0o755); err != nil {
				panic(err)
			}
			if err := os.WriteFile(path, v.Data, 0o644); err != nil {
				panic(err)
			}
		}
		return object{"FontAsset": object{"MIMEType": v.MIMEType, "DataFile": path, "FaceIndex": v.FaceIndex}}
	case *d2scene.FontAsset:
		return encAsset(*v)
	case d2scene.RasterAsset:
		return object{"RasterAsset": v}
	case *d2scene.RasterAsset:
		return encAsset(*v)
	case d2scene.VectorAsset:
		return object{"VectorAsset": object{"ViewBox": v.ViewBox, "Root": encNode(v.Root)}}
	case *d2scene.VectorAsset:
		return encAsset(*v)
	}
	panic(fmt.Sprintf("unsupported asset %T", a))
}

func encDocument(d *d2scene.Document) any {
	assets := object{}
	for id, a := range d.Assets {
		assets[string(id)] = encAsset(a)
	}
	return object{"ViewBox": d.ViewBox, "LogicalWidth": d.LogicalWidth, "LogicalHeight": d.LogicalHeight,
		"ViewportFit": d.ViewportFit, "ViewportAlign": d.ViewportAlign, "Root": encNode(d.Root),
		"Assets": assets, "Links": d.Links}
}
