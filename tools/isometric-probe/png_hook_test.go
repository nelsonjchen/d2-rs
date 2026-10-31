// SPDX-License-Identifier: MPL-2.0
// Test-only ordinary PNG and raster-mesh capture; installed only in the trace checkout.
package d2isometricimg

import (
	"bufio"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2target"
	"os"
	"path/filepath"
	"testing"
)

func TestD2rsPNGCapture(t *testing.T) {
	input := os.Getenv("D2RS_ISOMETRIC_PNG")
	if input == "" {
		t.Skip("D2RS_ISOMETRIC_PNG not set")
	}
	f, err := os.Open(input)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		var req struct{ Board, Out, Assets string }
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			t.Fatal(err)
		}
		data, err := os.ReadFile(req.Board)
		if err != nil {
			t.Fatal(err)
		}
		var d d2target.Diagram
		if err = json.Unmarshal(data, &d); err != nil {
			t.Fatal(err)
		}
		for _, b := range d2rsPNGMeshBoards(&d) {
			if err := os.MkdirAll(req.Out, 0755); err != nil {
				t.Fatal(err)
			}
			o, err := normalize(&Options{Format: PNG, Width: 1200, Height: 1200, FitContent: true})
			if err != nil {
				t.Fatal(err)
			}
			png, err := Render(context.Background(), b.diagram, &o)
			if err != nil {
				t.Fatal(err)
			}
			if err = os.WriteFile(filepath.Join(req.Out, b.name+".png"), png, 0644); err != nil {
				t.Fatal(err)
			}
			scene, err := d2isometric.BuildScene(b.diagram, &o.Render)
			if err != nil {
				t.Fatal(err)
			}
			native, err := newNativeSceneWithOptions(context.Background(), scene, o.Width, o.Height, nil, nil, nativeSceneOptions{deferRaster: true, fitContent: true, outputDensity: sceneOutputDensity(scene, o.Width, o.Height, nil)})
			if err != nil {
				t.Fatal(err)
			}
			dump := d2rsPNGMesh(native)
			textures := make([]string, len(dump.Materials))
			seen := map[*Material]bool{}
			index := 0
			for _, triangle := range native.triangles {
				m := triangle.Material
				if m == nil || seen[m] {
					continue
				}
				seen[m] = true
				if m.Texture != nil {
					bounds := m.Texture.Bounds()
					pixels := make([]byte, 0, bounds.Dx()*bounds.Dy()*4)
					for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
						for x := bounds.Min.X; x < bounds.Max.X; x++ {
							r, g, b, a := m.Texture.At(x, y).RGBA()
							pixels = append(pixels, byte(r>>8), byte(g>>8), byte(b>>8), byte(a>>8))
						}
					}
					sum := sha256.Sum256(pixels)
					path := filepath.Join(req.Assets, fmt.Sprintf("%x.rgba", sum))
					if err = os.MkdirAll(req.Assets, 0755); err != nil {
						t.Fatal(err)
					}
					if _, err := os.Stat(path); os.IsNotExist(err) {
						if err = os.WriteFile(path, pixels, 0644); err != nil {
							t.Fatal(err)
						}
					}
					textures[index] = path
				}
				index++
			}
			record := struct {
				d2rsPNGMeshDump
				TexturePixels []string `json:"texturePixels"`
			}{dump, textures}
			data, err = json.Marshal(record)
			if err != nil {
				t.Fatal(err)
			}
			if err = os.WriteFile(filepath.Join(req.Out, b.name+".mesh.json"), data, 0644); err != nil {
				t.Fatal(err)
			}
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
}

type d2rsPNGMeshVertex struct {
	P [3]float64 `json:"p"`
	N [3]float64 `json:"n"`
	U float64    `json:"u"`
	V float64    `json:"v"`
}

type d2rsPNGMeshMaterial struct {
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

type d2rsPNGMeshTriangle struct {
	V               [3]d2rsPNGMeshVertex `json:"v"`
	Material        int                  `json:"material"`
	CastShadow      bool                 `json:"castShadow,omitempty"`
	ShadowOpacity   *float64             `json:"shadowOpacity,omitempty"`
	ShadowFillAlpha *uint8               `json:"shadowFillAlpha,omitempty"`
	ShadowGround    *float64             `json:"shadowGround,omitempty"`
	DepthBias       float64              `json:"depthBias,omitempty"`
	NoDepthWrite    bool                 `json:"noDepthWrite,omitempty"`
	OpacityGroup    int                  `json:"opacityGroup,omitempty"`
	PaintOwner      int                  `json:"paintOwner,omitempty"`
	CoverageOnly    bool                 `json:"coverageOnly,omitempty"`
}

type d2rsPNGMeshLink struct {
	Points  [][3]float64 `json:"points"`
	URL     string       `json:"url,omitempty"`
	Tooltip string       `json:"tooltip,omitempty"`
	Target  string       `json:"target,omitempty"`
}

type d2rsPNGMeshDump struct {
	Background [4]uint8              `json:"background"`
	Width      int                   `json:"width"`
	Height     int                   `json:"height"`
	Camera     map[string]any        `json:"camera"`
	Materials  []d2rsPNGMeshMaterial `json:"materials"`
	Groups     []float64             `json:"groups"`
	Owners     []bool                `json:"owners"`
	Triangles  []d2rsPNGMeshTriangle `json:"triangles"`
}

func d2rsPNGVec(v Vec) [3]float64 { return [3]float64{v.X, v.Y, v.Z} }
func d2rsPNGMesh(scene *nativeScene) d2rsPNGMeshDump {
	dump := d2rsPNGMeshDump{Width: scene.width, Height: scene.height, Camera: map[string]any{
		"right": d2rsPNGVec(scene.camera.right), "up": d2rsPNGVec(scene.camera.up), "direction": d2rsPNGVec(scene.camera.direction),
		"centerX": scene.camera.centerX, "centerY": scene.camera.centerY, "centerDepth": scene.camera.centerDepth,
		"scale": scene.camera.scale, "width": scene.camera.width, "height": scene.camera.height,
	}}
	dump.Background = [4]uint8{scene.background.R, scene.background.G, scene.background.B, scene.background.A}
	materials := map[*Material]int{}
	groups := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	for _, t := range scene.triangles {
		out := d2rsPNGMeshTriangle{CastShadow: t.CastShadow, ShadowOpacity: t.ShadowOpacity, ShadowFillAlpha: t.ShadowFillAlpha,
			ShadowGround: t.ShadowGround, DepthBias: t.DepthBias, NoDepthWrite: t.NoDepthWrite, CoverageOnly: t.svgCoverageOnly}
		for i, v := range t.V {
			out.V[i] = d2rsPNGMeshVertex{P: d2rsPNGVec(v.Position), N: d2rsPNGVec(v.Normal), U: v.U, V: v.V}
		}
		if t.Material != nil {
			index, ok := materials[t.Material]
			if !ok {
				m := t.Material
				entry := d2rsPNGMeshMaterial{Color: [4]uint8{m.Color.R, m.Color.G, m.Color.B, m.Color.A}, Roughness: m.Roughness, Metalness: m.Metalness,
					Emissive: [4]uint8{m.Emissive.R, m.Emissive.G, m.Emissive.B, m.Emissive.A}, Unlit: m.Unlit,
					Multiply: m.Multiply, SVGContour: m.svgContour, SVGSolidTexture: m.svgSolidTexture, Gradient: m.gradient}
				if m.Texture != nil {
					b := m.Texture.Bounds()
					entry.Texture = []int{b.Dx(), b.Dy()}
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

func d2rsPNGMeshBoards(diagram *d2target.Diagram) []struct {
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
