// SPDX-License-Identifier: MPL-2.0
// Test-oracle mesh dump for d2-rs-isometric, copied only into the trace
// checkout's d2isometricimg package by scripts/isometric-scoreboard.py.
// D2RS_ISOMETRIC_MESH=<request.jsonl> runs TestD2rsMeshDump: each request is
// {"board": <board.exp.json>, "themeID": <int|null>, "out": <dir>} and writes
// <dir>/<board name>.mesh.json (or .err) for the e2e SVG configuration.
package d2isometricimg

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
)

// Optional call capture for the text tier; inert in every other mode.
var d2rsPrimitivesEnabled bool
var d2rsPrimitiveCalls []map[string]any

var d2rsTextEnabled bool
var d2rsTextCalls []map[string]any
var d2rsTextPainters map[*textPainter]int
var d2rsRichEnabled bool
var d2rsRichCalls []map[string]any
var d2rsRichPainters map[*richLabelPainter]int

type d2rsMeshVertex struct {
	P [3]float64 `json:"p"`
	N [3]float64 `json:"n"`
	U float64    `json:"u"`
	V float64    `json:"v"`
}

type d2rsMeshMaterial struct {
	Color           [4]uint8 `json:"color"`
	Roughness       float64  `json:"roughness"`
	Metalness       float64  `json:"metalness"`
	Emissive        [4]uint8 `json:"emissive"`
	Texture         []int    `json:"texture,omitempty"`
	Vector          int      `json:"vector,omitempty"`
	Unlit           bool     `json:"unlit,omitempty"`
	Multiply        bool     `json:"multiply,omitempty"`
	SVGContour      bool     `json:"svgContour,omitempty"`
	SVGSolidTexture bool     `json:"svgSolidTexture,omitempty"`
	Gradient        string   `json:"gradient,omitempty"`
}

type d2rsMeshTriangle struct {
	V               [3]d2rsMeshVertex `json:"v"`
	Material        int               `json:"material"`
	CastShadow      bool              `json:"castShadow,omitempty"`
	ShadowOpacity   *float64          `json:"shadowOpacity,omitempty"`
	ShadowFillAlpha *uint8            `json:"shadowFillAlpha,omitempty"`
	ShadowGround    *float64          `json:"shadowGround,omitempty"`
	DepthBias       float64           `json:"depthBias,omitempty"`
	NoDepthWrite    bool              `json:"noDepthWrite,omitempty"`
	OpacityGroup    int               `json:"opacityGroup,omitempty"`
	PaintOwner      int               `json:"paintOwner,omitempty"`
	CoverageOnly    bool              `json:"coverageOnly,omitempty"`
}

type d2rsMeshLink struct {
	Points  [][3]float64 `json:"points"`
	URL     string       `json:"url,omitempty"`
	Tooltip string       `json:"tooltip,omitempty"`
	Target  string       `json:"target,omitempty"`
}

type d2rsSurfaceDump struct {
	Document        any              `json:"document"`
	InkCoverage     *d2rsSurfaceDump `json:"inkCoverage,omitempty"`
	CoverageOpacity float64          `json:"coverageOpacity,omitempty"`
	CapBackground   *[4]uint8        `json:"capBackground,omitempty"`
	CapPaint        any              `json:"capPaint,omitempty"`
}

func d2rsSurface(surface *nativeVectorSurface) *d2rsSurfaceDump {
	if surface == nil {
		return nil
	}
	out := &d2rsSurfaceDump{Document: encDocument(surface.document), CoverageOpacity: surface.coverageOpacity, CapPaint: encPaint(surface.capPaint)}
	if surface.capBackground != nil {
		c := surface.capBackground
		out.CapBackground = &[4]uint8{c.R, c.G, c.B, c.A}
	}
	out.InkCoverage = d2rsSurface(surface.inkCoverage)
	return out
}

type d2rsMeshDump struct {
	Surfaces          []*d2rsSurfaceDump `json:"surfaces,omitempty"`
	BackgroundSurface *d2rsSurfaceDump   `json:"backgroundSurface,omitempty"`
	Background        [4]uint8           `json:"background"`
	BackgroundPaint   bool               `json:"backgroundPaint,omitempty"`
	Links             []d2rsMeshLink     `json:"links,omitempty"`
	Width             int                `json:"width"`
	Height            int                `json:"height"`
	Camera            map[string]any     `json:"camera"`
	Materials         []d2rsMeshMaterial `json:"materials"`
	Groups            []float64          `json:"groups"`
	Owners            []bool             `json:"owners"`
	Triangles         []d2rsMeshTriangle `json:"triangles"`
}

func d2rsVec(v Vec) [3]float64 { return [3]float64{v.X, v.Y, v.Z} }

func d2rsMesh(scene *nativeScene, full bool) d2rsMeshDump {
	dump := d2rsMeshDump{Width: scene.width, Height: scene.height, Camera: map[string]any{
		"right": d2rsVec(scene.camera.right), "up": d2rsVec(scene.camera.up), "direction": d2rsVec(scene.camera.direction),
		"centerX": scene.camera.centerX, "centerY": scene.camera.centerY, "centerDepth": scene.camera.centerDepth,
		"scale": scene.camera.scale, "width": scene.camera.width, "height": scene.camera.height,
	}}
	dump.Background = [4]uint8{scene.background.R, scene.background.G, scene.background.B, scene.background.A}
	dump.BackgroundPaint = scene.backgroundPaint != nil
	if full {
		if scene.backgroundPaint != nil {
			dump.BackgroundSurface = d2rsSurface(&nativeVectorSurface{document: nativePaintDocument(scene.backgroundPaint)})
		}
	}
	for _, link := range scene.links {
		out := d2rsMeshLink{URL: link.region.URL, Tooltip: link.region.Tooltip, Target: link.region.Target, Points: [][3]float64{}}
		for _, p := range link.points {
			out.Points = append(out.Points, d2rsVec(p))
		}
		dump.Links = append(dump.Links, out)
	}
	vectors := map[*nativeVectorSurface]int{}
	materials := map[*Material]int{}
	groups := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	for _, t := range scene.triangles {
		out := d2rsMeshTriangle{CastShadow: t.CastShadow, ShadowOpacity: t.ShadowOpacity, ShadowFillAlpha: t.ShadowFillAlpha,
			ShadowGround: t.ShadowGround, DepthBias: t.DepthBias, NoDepthWrite: t.NoDepthWrite, CoverageOnly: t.svgCoverageOnly}
		for i, v := range t.V {
			out.V[i] = d2rsMeshVertex{P: d2rsVec(v.Position), N: d2rsVec(v.Normal), U: v.U, V: v.V}
		}
		if t.Material != nil {
			index, ok := materials[t.Material]
			if !ok {
				m := t.Material
				entry := d2rsMeshMaterial{Color: [4]uint8{m.Color.R, m.Color.G, m.Color.B, m.Color.A}, Roughness: m.Roughness, Metalness: m.Metalness,
					Emissive: [4]uint8{m.Emissive.R, m.Emissive.G, m.Emissive.B, m.Emissive.A}, Unlit: m.Unlit,
					Multiply: m.Multiply, SVGContour: m.svgContour, SVGSolidTexture: m.svgSolidTexture, Gradient: m.gradient}
				if m.Texture != nil {
					b := m.Texture.Bounds()
					entry.Texture = []int{b.Dx(), b.Dy()}
				}
				if m.Vector != nil {
					if _, ok := vectors[m.Vector]; !ok {
						vectors[m.Vector] = len(vectors) + 1
						if full {
							dump.Surfaces = append(dump.Surfaces, d2rsSurface(m.Vector))
						}
					}
					entry.Vector = vectors[m.Vector]
				}
				index = len(dump.Materials) + 1
				materials[t.Material] = index
				dump.Materials = append(dump.Materials, entry)
			}
			out.Material = index
		}
		if t.OpacityGroup != nil {
			index, ok := groups[t.OpacityGroup]
			if !ok {
				index = len(dump.Groups) + 1
				groups[t.OpacityGroup] = index
				dump.Groups = append(dump.Groups, t.OpacityGroup.Opacity)
			}
			out.OpacityGroup = index
		}
		if t.PaintOwner != nil {
			index, ok := owners[t.PaintOwner]
			if !ok {
				index = len(owners) + 1
				owners[t.PaintOwner] = index
				dump.Owners = append(dump.Owners, t.PaintOwner.Opaque)
			}
			out.PaintOwner = index
		}
		dump.Triangles = append(dump.Triangles, out)
	}
	return dump
}

func d2rsMeshBoards(diagram *d2target.Diagram) []struct {
	name    string
	diagram *d2target.Diagram
} {
	var out []struct {
		name    string
		diagram *d2target.Diagram
	}
	var walk func(*d2target.Diagram, string)
	walk = func(d *d2target.Diagram, name string) {
		if !d.IsFolderOnly {
			out = append(out, struct {
				name    string
				diagram *d2target.Diagram
			}{name, d})
		}
		for i, children := range [][]*d2target.Diagram{d.Layers, d.Scenarios, d.Steps} {
			kind := []string{"layers", "scenarios", "steps"}[i]
			for j, child := range children {
				walk(child, fmt.Sprintf("%s.%s.%d", name, kind, j))
			}
		}
	}
	walk(diagram, "isometric")
	return out
}

type d2rsVisibilityFace struct {
	Points  [][3]float64 `json:"points"`
	Opaque  bool         `json:"opaque,omitempty"`
	Group   int          `json:"group,omitempty"`
	Owner   int          `json:"owner,omitempty"`
	Contour bool         `json:"contour,omitempty"`
	Order   int          `json:"order"`
}

type d2rsVisibilityDump struct {
	Faces   []d2rsVisibilityFace `json:"faces"`
	Visible [][][][3]float64     `json:"visible"`
	Error   string               `json:"error,omitempty"`
}

// d2rsVisibility repeats nativeSceneSVG's face construction (svg.go) and
// records svgVisibleFaces' input and output.
func d2rsVisibility(scene *nativeScene) d2rsVisibilityDump {
	camera := scene.camera
	faces := make([]svgVisibilityFace, len(scene.triangles))
	for i, t := range scene.triangles {
		f := &faces[i]
		f.order, f.group = i, t.OpacityGroup
		f.owner = t.PaintOwner
		m := t.Material
		f.contour = m != nil && m.svgContour
		paint := rasterMaterial(m)
		if paint.alpha == 0 {
			continue
		}
		if paint.alpha < .999 || paint.multiply {
			n := nadd(nadd(t.V[0].Normal, t.V[1].Normal), t.V[2].Normal)
			if ndot(n, n) > 1e-20 && ndot(n, camera.direction) <= 0 {
				continue
			}
		}
		f.opaque = !t.NoDepthWrite && paint.alpha == 1 && (paint.texture == nil || m.svgSolidTexture) && !paint.multiply
		for _, v := range t.V {
			p := camera.project(v.Position)
			f.points = append(f.points, svgProjectedPoint(p.x, p.y, p.z+t.DepthBias))
		}
	}
	groups := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	dump := d2rsVisibilityDump{}
	for _, f := range faces {
		out := d2rsVisibilityFace{Opaque: f.opaque, Contour: f.contour, Order: f.order, Points: [][3]float64{}}
		if f.group != nil {
			if _, ok := groups[f.group]; !ok {
				groups[f.group] = len(groups) + 1
			}
			out.Group = groups[f.group]
		}
		if f.owner != nil {
			if _, ok := owners[f.owner]; !ok {
				owners[f.owner] = len(owners) + 1
			}
			out.Owner = owners[f.owner]
		}
		for _, p := range f.points {
			out.Points = append(out.Points, [3]float64{p.x, p.y, p.z})
		}
		dump.Faces = append(dump.Faces, out)
	}
	visible, err := svgVisibleFaces(context.Background(), faces)
	if err != nil {
		dump.Error = err.Error()
		return dump
	}
	for _, fragments := range visible {
		out := [][][3]float64{}
		for _, fragment := range fragments {
			points := [][3]float64{}
			for _, p := range fragment {
				points = append(points, [3]float64{p.x, p.y, p.z})
			}
			out = append(out, points)
		}
		dump.Visible = append(dump.Visible, out)
	}
	return dump
}

func TestD2rsMeshDump(t *testing.T) {
	requests := os.Getenv("D2RS_ISOMETRIC_MESH")
	if requests == "" {
		t.Skip("D2RS_ISOMETRIC_MESH not set")
	}
	file, err := os.Open(requests)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 1<<20), 1<<26)
	for scanner.Scan() {
		var req struct {
			Board        string                `json:"board"`
			ThemeID      *int64                `json:"themeID"`
			Out          string                `json:"out"`
			Mode         string                `json:"mode"`
			Assets       string                `json:"assets"`
			Stream       bool                  `json:"stream"`
			AssetSources map[string]assetInput `json:"assetSources"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			t.Fatal(err)
		}
		assetStore = req.Assets
		data, err := os.ReadFile(req.Board)
		if err != nil {
			t.Fatal(err)
		}
		var diagram d2target.Diagram
		if err := json.Unmarshal(data, &diagram); err != nil {
			t.Fatal(err)
		}
		if err := os.MkdirAll(req.Out, 0o755); err != nil {
			t.Fatal(err)
		}
		var overrides *d2target.ThemeOverrides
		themeID := req.ThemeID
		if diagram.Config != nil {
			overrides = diagram.Config.ThemeOverrides
			if themeID == nil {
				themeID = diagram.Config.ThemeID
			}
		}
		for _, b := range d2rsMeshBoards(&diagram) {
			d2rsPrimitivesEnabled = req.Mode == "primitives"
			d2rsPrimitiveCalls = []map[string]any{}
			d2rsRichEnabled = req.Mode == "rich"
			d2rsRichCalls = []map[string]any{}
			d2rsRichPainters = nil
			d2rsTextEnabled = req.Mode == "text"
			d2rsTextCalls = []map[string]any{}
			d2rsTextPainters = nil
			payload, err := func() ([]byte, error) {
				o, err := normalize(&Options{Render: d2isometric.RenderOpts{ThemeID: themeID, ThemeOverrides: overrides}, Assets: assetOptions(req.AssetSources),
					Format: SVG, Width: 1200, Height: 1200, FitContent: true})
				if err != nil {
					return nil, err
				}
				ctx := nativeVectorContext(context.Background())
				o.LinkBudget = &d2scenebuild.LinkBudget{MaxRegions: 4096, MaxStringBytes: 1 << 20}
				scene, err := d2isometric.BuildScene(b.diagram, &o.Render)
				if err != nil {
					return nil, err
				}
				native, err := newNativeSceneWithOptions(ctx, scene, o.Width, o.Height, o.Assets, o.Fonts, nativeSceneOptions{
					deferRaster: true, vector: true, fitContent: o.FitContent, camera: o.camera,
					outputDensity: sceneOutputDensity(scene, o.Width, o.Height, o.camera), links: o.LinkBudget,
				})
				if err != nil {
					return nil, err
				}
				if req.Mode == "visibility" {
					return json.Marshal(d2rsVisibility(native))
				}
				var payload []byte
				if req.Mode == "primitives" {
					payload, err = json.Marshal(d2rsPrimitiveCalls)
				} else if req.Mode == "rich" {
					payload, err = json.Marshal(d2rsRichCalls)
				} else if req.Mode == "text" {
					payload, err = json.Marshal(d2rsTextCalls)
				} else {
					payload, err = json.Marshal(d2rsMesh(native, req.Mode == "surfaces"))
				}
				if err != nil {
					return nil, err
				}
				svg, err := nativeSceneSVG(ctx, native)
				if err != nil {
					return nil, err
				}
				oraclePath := filepath.Join(strings.Replace(req.Out, "/mesh/", "/go/", 1), b.name+".svg")
				if err := os.MkdirAll(filepath.Dir(oraclePath), 0o755); err != nil {
					return nil, err
				}
				if err := os.WriteFile(oraclePath, svg, 0o644); err != nil {
					return nil, err
				}
				return payload, nil
			}()
			path := filepath.Join(req.Out, b.name)
			if err != nil {
				path, payload = path+".err", []byte(err.Error())
			} else if req.Mode == "visibility" {
				path += ".visibility.json"
			} else {
				path += ".mesh.json"
			}
			if err := os.WriteFile(path+".partial", payload, 0o644); err != nil {
				t.Fatal(err)
			}
			if err := os.Rename(path+".partial", path); err != nil {
				t.Fatal(err)
			}
			if req.Stream {
				deadline := time.Now().Add(11 * time.Minute)
				for {
					if _, err := os.Stat(path + ".consumed"); err == nil {
						os.Remove(path + ".consumed")
						break
					}
					if time.Now().After(deadline) {
						t.Fatal("mesh consumer timed out")
					}
					time.Sleep(10 * time.Millisecond)
				}
			}
		}
	}
}
