// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): internal/fontface shaping of bundled fonts
// and sfnt cmap selection. Harvested from the v0.9.0-era probes
// d2fonts/{grapheme,color}_shaping_probe_test.go, d2fonts/cmap_contract_probe_test.go
// and ascii_shaping_probe.go.overlay; the request grids are the probes' own
// (fonts x sizes x texts). Retargeted to the manifest pin. Inert unless run by
// scripts/regen-fixtures.py.
package d2fonts

import (
	"context"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/internal/fontface"
	"golang.org/x/image/font/sfnt"
	"golang.org/x/image/math/fixed"
)

func d2rsShapeFace(t *testing.T, name string) fontface.ShapeFace {
	data, err := os.ReadFile("ttf/" + name)
	if name == "NotoColorEmoji" {
		data, err = bundledNotoColorEmoji()
	}
	if err != nil {
		t.Fatal(err)
	}
	face, err := fontface.ParseFace(data, 0)
	if err != nil {
		t.Fatal(err)
	}
	return fontface.ShapeFace{ID: name, Face: face}
}

func TestD2RSFontShaping(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Primary  string               `json:"primary"`
		Fallback string               `json:"fallback"`
		Text     string               `json:"text"`
		Size     float64              `json:"size"`
		Shaped   *fontface.ShapedText `json:"shaped"`
		Error    string               `json:"error"`
	}
	type grid struct {
		pairs [][2]string
		sizes []float64
		texts []string
	}
	limits := fontface.ShapeLimits{Runes: 1000, Faces: 64, CoverageChecks: 10000, Runs: 1000, Glyphs: 1000}
	var value any
	switch mode := os.Getenv("D2RS_SHAPING_MODE"); mode {
	case "ascii":
		// ascii_shaping_probe.go.overlay: one face, map rows (sorted keys).
		texts := []string{"(JSON-RPC 2.0)", "JSON-RPC 2.0", "2. JSON-RPC 2.0", "a.b", "office", "AV", "2.0", "(2.0)", "RPC 2.0", "x 2.0"}
		rows := []any{}
		for _, name := range []string{"Regular", "Bold", "Italic"} {
			face := d2rsShapeFace(t, "SourceSansPro-"+name+".ttf")
			face.ID = name
			for _, size := range []int{10, 16, 24} {
				for _, text := range texts {
					var w fontface.ShapingWorkspace
					s, err := w.ShapeTextTransient(context.Background(), text, fixed.Int26_6(size*64), []fontface.ShapeFace{face}, fontface.ShapeLimits{Runes: 1000, Faces: 10, CoverageChecks: 100000, Runs: 1000, Glyphs: 1000})
					if err != nil {
						t.Fatal(err)
					}
					rows = append(rows, map[string]any{"font": name, "size": size, "text": text, "shaped": s})
				}
			}
		}
		value = rows
	default:
		g := map[string]grid{
			"control": {
				pairs: [][2]string{{"SourceSansPro-Regular.ttf", "SourceSansPro-Regular.ttf"}, {"SourceSansPro-Bold.ttf", "SourceSansPro-Bold.ttf"}, {"SourceSansPro-Italic.ttf", "SourceSansPro-Italic.ttf"}},
				sizes: []float64{10, 16, 24},
				texts: []string{"b\rb", "a\tb", "a\nb", "a\r\nb", "\x00\t\n\r\x7f", "a\x00b", "a\x7fb", "a\x1bb", "\r", "a b"},
			},
			"color": {
				pairs: [][2]string{{"SourceSansPro-Regular.ttf", "NotoColorEmoji"}},
				sizes: []float64{10, 16, 13.6},
				texts: []string{"✅ \U0001f600", "\U0001f600 \U0001f600", "\U0001f600 A \U0001f600", "Agm \U0001f600 Agm", "\U0001f469‍\U0001f4bb \U0001f600", "\U0001f1fa\U0001f1f8 ✅", "1️⃣ \U0001f600", "❤️ \U0001f600"},
			},
			"grapheme": {
				pairs: [][2]string{{"FuzzyBubbles-Regular.ttf", "SourceSansPro-Regular.ttf"}, {"SourceCodePro-Regular.ttf", "SourceSansPro-Regular.ttf"}},
				sizes: []float64{10, 13.6},
				texts: []string{"Agm AV fi", "é Ж́ é", "abc Ώ 123", "abc אבג 123", "abc العربية 123", "(abc) [Ж] Ω", "A‍B", "A\U0010ffffB", "\U0001f469‍\U0001f4bb A", "؀A", "͏"},
			},
		}[mode]
		if g.pairs == nil {
			t.Fatalf("unknown D2RS_SHAPING_MODE %q", mode)
		}
		rows := []row{}
		for _, pair := range g.pairs {
			for _, size := range g.sizes {
				for _, text := range g.texts {
					r := row{Primary: pair[0], Fallback: pair[1], Text: text, Size: size}
					fonts := []fontface.ShapeFace{d2rsShapeFace(t, pair[0]), d2rsShapeFace(t, pair[1])}
					shaped, err := new(fontface.ShapingWorkspace).ShapeTextTransient(context.Background(), text, fixed.Int26_6(math.Round(size*64)), fonts, limits)
					if err != nil {
						r.Error = err.Error()
					} else {
						r.Shaped = &shaped
					}
					rows = append(rows, r)
				}
			}
		}
		value = rows
	}
	data, err := json.MarshalIndent(value, "", "  ")
	if os.Getenv("D2RS_SHAPING_MODE") == "ascii" {
		data, err = json.Marshal(value) // the probe's compact json.Encoder layout
	}
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// cmap_contract_probe_test.go: Source Sans Pro with its cmap replaced by each
// committed subtable (tools/fixtures/inputs/renderexport/font-cmap-inputs.json).
func TestD2RSFontCmap(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type input struct {
		Name    string `json:"name"`
		CmapHex string `json:"cmap_hex"`
		Runes   []rune `json:"runes"`
	}
	type outcome struct {
		Name   string   `json:"name"`
		Glyphs []uint16 `json:"glyphs"`
		Error  string   `json:"error"`
	}
	type row struct {
		input
		Source outcome `json:"source"`
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs []input
	if err := json.Unmarshal(data, &inputs); err != nil {
		t.Fatal(err)
	}
	base, err := os.ReadFile("ttf/SourceSansPro-Regular.ttf")
	if err != nil {
		t.Fatal(err)
	}
	rows := []row{}
	for _, in := range inputs {
		font := append([]byte(nil), base...)
		cmap, err := hex.DecodeString(in.CmapHex)
		if err != nil {
			t.Fatal(err)
		}
		offset := len(font)
		font = append(font, cmap...)
		found := false
		for i := 0; i < int(binary.BigEndian.Uint16(font[4:])); i++ {
			at := 12 + 16*i
			if string(font[at:at+4]) == "cmap" {
				binary.BigEndian.PutUint32(font[at+8:], uint32(offset))
				binary.BigEndian.PutUint32(font[at+12:], uint32(len(cmap)))
				found = true
				break
			}
		}
		if !found {
			t.Fatal("cmap missing")
		}
		o := outcome{Name: in.Name, Glyphs: []uint16{}}
		face, err := sfnt.Parse(font)
		if err != nil {
			o.Error = err.Error()
		} else {
			var buffer sfnt.Buffer
			for _, value := range in.Runes {
				glyph, err := face.GlyphIndex(&buffer, value)
				if err != nil {
					t.Fatal(err)
				}
				o.Glyphs = append(o.Glyphs, uint16(glyph))
			}
		}
		rows = append(rows, row{in, o})
	}
	data, err = json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
