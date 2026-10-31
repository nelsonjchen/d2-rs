// SPDX-License-Identifier: MPL-2.0
// Test-oracle probe for crates/d2-rs-fontface. Go's internal-package rule
// only admits importers under d2renderers/, so it is built inside the trace
// checkout as d2renderers/cmd/d2rs-fontface-probe.
//
// usage: d2rs-fontface-probe <d2 checkout> < requests.jsonl > records.jsonl
//        d2rs-fontface-probe -gen-tables > crates/d2-rs-fontface/src/shaping_tables.rs
//
// -gen-tables emits the pinned golang.org/x/text unicode/bidi properties
// (class, bracket kind) and go-text language.ScriptRanges as Rust tables.
//
// Each request line is {"key", "text", "font", "fallback", "size"}: font is a
// bundled D2 face name (e.g. "SourceSansPro-Bold"), fallback appends the
// bundled Noto Color Emoji face, size is the 26.6 ppem. For each request the
// probe writes one record with ShapeTextTransient's ShapedText (or its error
// text), then one record per newly seen inked (font, glyph) with its
// LoadGlyph segments at 64 ppem (or the error text).
//
// Extended requests: {"kind": "glyphs", "font"} dumps, for every glyph ID
// of the face (keyed "<key>:<glyph>"), LoadGlyph, GlyphAdvance,
// GlyphRenderBounds, COLR0GlyphLayers and, for the bundled emoji face, the
// COLRv1 plan; {"kind": "font", "font"} dumps
// Metrics, Bounds, the cmap below U+3000, SupportsRenderableRune below
// U+2800 and Kern for every printable ASCII pair.
package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"image/color"
	"io"
	"os"
	"path/filepath"

	"github.com/andybalholm/brotli"
	"github.com/go-text/typesetting/language"
	"golang.org/x/text/unicode/bidi"
	"golang.org/x/image/font"
	"golang.org/x/image/font/sfnt"
	"golang.org/x/image/math/fixed"

	"github.com/d2lang/d2/d2renderers/internal/fontface"
)

type request struct {
	Key      string `json:"key"`
	Kind     string `json:"kind"`
	Text     string `json:"text"`
	Font     string `json:"font"`
	Fallback bool   `json:"fallback"`
	Size     int32  `json:"size"`
}

type glyphRecord struct {
	ID          uint32   `json:"id"`
	Face        int      `json:"face"`
	PositionX   float64  `json:"x"`
	PositionY   float64  `json:"y"`
	Advance     float64  `json:"advance"`
	Ink         [4]int32 `json:"ink"`
	HasInk      bool     `json:"hasInk"`
	Empty       bool     `json:"empty"`
	Source      int32    `json:"source"`
	SourceIndex int      `json:"sourceIndex"`
}

type shapedRecord struct {
	Glyphs         []glyphRecord `json:"glyphs"`
	Advance        float64       `json:"advance"`
	Runes          int           `json:"runes"`
	CoverageChecks int64         `json:"coverageChecks"`
	Runs           int           `json:"runs"`
}

type record struct {
	Key      string        `json:"key"`
	Error    *string       `json:"error,omitempty"`
	Shaped   *shapedRecord `json:"shaped,omitempty"`
	Segments [][]int32     `json:"segments,omitempty"`
	Values   map[string]any `json:"values,omitempty"`
}

func errText(err error) any {
	if err == nil {
		return nil
	}
	return err.Error()
}

func segmentRows(segments sfnt.Segments) [][]int32 {
	rows := make([][]int32, 0, len(segments))
	for _, s := range segments {
		rows = append(rows, []int32{int32(s.Op),
			int32(s.Args[0].X), int32(s.Args[0].Y), int32(s.Args[1].X), int32(s.Args[1].Y), int32(s.Args[2].X), int32(s.Args[2].Y)})
	}
	return rows
}

func rect(r fixed.Rectangle26_6) []int32 {
	return []int32{int32(r.Min.X), int32(r.Min.Y), int32(r.Max.X), int32(r.Max.Y)}
}

func colorJSON(c color.NRGBA) []int { return []int{int(c.R), int(c.G), int(c.B), int(c.A)} }

func lineJSON(line fontface.COLRv1ColorLine) []any {
	stops := []any{}
	for _, stop := range line.Stops {
		stops = append(stops, []any{stop.Offset, colorJSON(stop.Color)})
	}
	return stops
}

func paintJSON(paint fontface.COLRv1Paint) any {
	switch p := paint.(type) {
	case fontface.COLRv1Layers:
		children := []any{}
		for _, child := range p.Paints {
			children = append(children, paintJSON(child))
		}
		return map[string]any{"layers": children}
	case fontface.COLRv1Solid:
		return map[string]any{"solid": colorJSON(p.Color)}
	case fontface.COLRv1LinearGradient:
		return map[string]any{"linear": []any{lineJSON(p.ColorLine), p.X0, p.Y0, p.X1, p.Y1, p.X2, p.Y2}}
	case fontface.COLRv1RadialGradient:
		return map[string]any{"radial": []any{lineJSON(p.ColorLine), p.X0, p.Y0, p.Radius0, p.X1, p.Y1, p.Radius1}}
	case fontface.COLRv1Glyph:
		return map[string]any{"glyph": p.GlyphID, "paint": paintJSON(p.Paint)}
	case fontface.COLRv1Transform:
		m := p.Matrix
		return map[string]any{"transform": []float64{m.Xx, m.Yx, m.Xy, m.Yy, m.Dx, m.Dy}, "paint": paintJSON(p.Paint)}
	case fontface.COLRv1Composite:
		return map[string]any{"composite": uint8(p.Mode), "source": paintJSON(p.Source), "backdrop": paintJSON(p.Backdrop)}
	}
	panic(fmt.Sprintf("unknown paint %T", paint))
}

func glyphValues(face *fontface.ParsedFace, glyph uint32) map[string]any {
	values := map[string]any{}
	var buffer sfnt.Buffer
	segments, err := face.Outline.LoadGlyph(&buffer, sfnt.GlyphIndex(glyph), fixed.I(64), nil)
	values["segments"], values["segmentsError"] = segmentRows(segments), errText(err)
	if err != nil {
		values["segments"] = nil
	}
	advance, err := face.Outline.GlyphAdvance(&buffer, sfnt.GlyphIndex(glyph), fixed.I(64), 0)
	values["advance"], values["advanceError"] = int32(advance), errText(err)
	for _, size := range []fixed.Int26_6{fixed.I(64), fixed.I(15)} {
		bounds, ink, err := face.GlyphRenderBounds(glyph, size)
		values[fmt.Sprintf("bounds%d", size)] = []any{rect(bounds), ink, errText(err)}
	}
	layers, color, err := face.COLR0GlyphLayers(glyph)
	rows := []any{}
	for _, layer := range layers {
		rows = append(rows, []any{layer.GlyphID, colorJSON(layer.Color), layer.Foreground})
	}
	// The Rust API reports an error without Go's accompanying flag.
	values["colr0"] = []any{rows, color && err == nil, errText(err)}
	if face.IsBundledNotoColorEmoji() {
		plan, found, err := face.CompileBundledNotoColorEmojiCOLRv1Plan(glyph)
		var planJSON any
		if plan != nil {
			var clip any
			if plan.Clip != nil {
				clip = []float64{plan.Clip.XMin, plan.Clip.YMin, plan.Clip.XMax, plan.Clip.YMax}
			}
			u := plan.Usage
			planJSON = map[string]any{"root": paintJSON(plan.Root), "clip": clip, "usage": []int{u.PaintNodes, u.MaxDepth, u.MaxLayers, u.MaxGradientStops}}
		}
		values["plan"] = []any{planJSON, found, errText(err)}
	}
	return values
}

func fontValues(face *fontface.ParsedFace) map[string]any {
	values := map[string]any{"numGlyphs": face.Outline.NumGlyphs(), "unitsPerEm": int32(face.Outline.UnitsPerEm())}
	var buffer sfnt.Buffer
	for _, size := range []fixed.Int26_6{fixed.I(64), fixed.I(15), 1000} {
		for _, hinting := range []font.Hinting{font.HintingNone, font.HintingFull} {
			m, err := face.Outline.Metrics(&buffer, size, hinting)
			values[fmt.Sprintf("metrics%d/%d", size, hinting)] = []any{[]int32{int32(m.Height), int32(m.Ascent), int32(m.Descent), int32(m.XHeight), int32(m.CapHeight), int32(m.CaretSlope.X), int32(m.CaretSlope.Y)}, errText(err)}
			b, err := face.Outline.Bounds(&buffer, size, hinting)
			values[fmt.Sprintf("bounds%d/%d", size, hinting)] = []any{rect(b), errText(err)}
		}
	}
	runes := []any{}
	for r := rune(0); r < 0x3000; r++ {
		g, err := face.Outline.GlyphIndex(&buffer, r)
		if g != 0 || err != nil {
			runes = append(runes, []any{r, g, errText(err)})
		}
	}
	values["cmap"] = runes
	supported := []any{}
	for r := rune(0x20); r < 0x2800; r++ {
		ok, err := face.SupportsRenderableRune(r)
		if ok || err != nil {
			supported = append(supported, []any{r, ok, errText(err)})
		}
	}
	values["supports"] = supported
	kerns := []any{}
	for a := rune(0x21); a < 0x7f; a++ {
		for b := rune(0x21); b < 0x7f; b++ {
			ga, _ := face.Outline.GlyphIndex(&buffer, a)
			gb, _ := face.Outline.GlyphIndex(&buffer, b)
			k, err := face.Outline.Kern(&buffer, ga, gb, fixed.I(64), font.HintingNone)
			if k != 0 || (err != nil && err != sfnt.ErrNotFound) {
				kerns = append(kerns, []any{a, b, int32(k), errText(err)})
			}
		}
	}
	values["kern"] = kerns
	return values
}

const notoName = "NotoColorEmoji"

func genTables() {
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	fmt.Fprintln(out, "// SPDX-License-Identifier: BSD-3-Clause AND (Unlicense OR BSD-3-Clause)")
	fmt.Fprintln(out, "// Generated by tools/fontface-probe -gen-tables from golang.org/x/text")
	fmt.Fprintln(out, "// v0.41.0 unicode/bidi (Go 1.27: Unicode 17.0.0 tables) and go-text/typesetting")
	fmt.Fprintln(out, "// v0.3.4 language.ScriptRanges. DO NOT EDIT.")
	fmt.Fprintln(out)
	fmt.Fprintln(out, "/// (first, last, bidi.Class, bracket: 0 none, 1 opening, 2 closing).")
	fmt.Fprintln(out, "pub(crate) static BIDI: &[(u32, u32, u8, u8)] = &[")
	key := func(r rune) [2]uint8 {
		if r >= 0xd800 && r <= 0xdfff {
			return [2]uint8{255, 255}
		}
		p, _ := bidi.LookupRune(r)
		bracket := uint8(0)
		if p.IsOpeningBracket() {
			bracket = 1
		} else if p.IsBracket() {
			bracket = 2
		}
		return [2]uint8{uint8(p.Class()), bracket}
	}
	start, current := rune(0), key(0)
	for r := rune(1); r <= 0x110000; r++ {
		var next [2]uint8
		if r < 0x110000 {
			next = key(r)
		}
		if r == 0x110000 || next != current {
			if current[0] != 255 {
				fmt.Fprintf(out, "    (0x%x, 0x%x, %d, %d),\n", start, r-1, current[0], current[1])
			}
			start, current = r, next
		}
	}
	fmt.Fprintln(out, "];")
	fmt.Fprintln(out)
	fmt.Fprintf(out, "/// Unknown script tag for runes outside SCRIPTS.\npub(crate) const UNKNOWN_SCRIPT: [u8; 4] = *b%q;\n\n", language.Unknown.String())
	fmt.Fprintln(out, "/// (first, last, ISO 15924 tag) from language.ScriptRanges.")
	fmt.Fprintln(out, "pub(crate) static SCRIPTS: &[(u32, u32, [u8; 4])] = &[")
	for _, entry := range language.ScriptRanges {
		fmt.Fprintf(out, "    (0x%x, 0x%x, *b%q),\n", entry.Start, entry.End, entry.Script.String())
	}
	fmt.Fprintln(out, "];")
}

func main() {
	if len(os.Args) == 2 && os.Args[1] == "-gen-tables" {
		genTables()
		return
	}
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: d2rs-fontface-probe <d2 checkout>")
		os.Exit(2)
	}
	root := os.Args[1]
	faces := map[string]*fontface.ParsedFace{}
	load := func(name string) (*fontface.ParsedFace, error) {
		if face := faces[name]; face != nil {
			return face, nil
		}
		var face *fontface.ParsedFace
		if name == notoName {
			compressed, err := os.ReadFile(filepath.Join(root, "d2renderers/d2fonts/encoded/NotoColorEmoji-COLRv1-v2.051.ttf.br"))
			if err != nil {
				return nil, err
			}
			data, err := io.ReadAll(brotli.NewReader(bytes.NewReader(compressed)))
			if err != nil {
				return nil, err
			}
			source, ok, err := fontface.RegisteredBundledNotoColorEmoji(data, 0)
			if err != nil || !ok {
				return nil, fmt.Errorf("register bundled Noto Color Emoji: %v %v", ok, err)
			}
			if face, err = source.CloneReadOnly(); err != nil {
				return nil, err
			}
		} else {
			data, err := os.ReadFile(filepath.Join(root, "d2renderers/d2fonts/ttf", name+".ttf"))
			if err != nil {
				return nil, err
			}
			if err := fontface.RegisterBundledFace(data, sha256.Sum256(data)); err != nil {
				return nil, err
			}
			source, ok, err := fontface.RegisteredBundledFace(data, 0)
			if err != nil || !ok {
				return nil, fmt.Errorf("registered bundled face %s: %v %v", name, ok, err)
			}
			if face, err = source.CloneReadOnly(); err != nil {
				return nil, err
			}
		}
		faces[name] = face
		return face, nil
	}
	limits := fontface.ShapeLimits{Runes: 100_000, Faces: 64, CoverageChecks: 50_000_000, Runs: 100_000, Glyphs: 10_000_000}
	var workspace fontface.ShapingWorkspace
	seen := map[string]bool{}
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1<<20), 1<<26)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	encoder := json.NewEncoder(out)
	encoder.SetEscapeHTML(false)
	emit := func(r record) {
		if err := encoder.Encode(r); err != nil {
			panic(err)
		}
	}
	fail := func(key string, err error) {
		text := err.Error()
		emit(record{Key: key, Error: &text})
	}
	for scanner.Scan() {
		var req request
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			panic(err)
		}
		if req.Kind == "glyphs" || req.Kind == "font" {
			face, err := load(req.Font)
			if err != nil {
				fail(req.Key, err)
				continue
			}
			if req.Kind == "glyphs" {
				for glyph := 0; glyph < face.Outline.NumGlyphs(); glyph++ {
					emit(record{Key: fmt.Sprintf("%s:%d", req.Key, glyph), Values: glyphValues(face, uint32(glyph))})
				}
			} else {
				emit(record{Key: req.Key, Values: fontValues(face)})
			}
			continue
		}
		names := []string{req.Font}
		if req.Fallback {
			names = append(names, notoName)
		}
		shapeFaces := make([]fontface.ShapeFace, 0, len(names))
		var loadErr error
		for _, name := range names {
			face, err := load(name)
			if err != nil {
				loadErr = err
				break
			}
			shapeFaces = append(shapeFaces, fontface.ShapeFace{ID: name, Face: face})
		}
		if loadErr != nil {
			fail(req.Key, loadErr)
			continue
		}
		shaped, err := workspace.ShapeTextTransient(context.Background(), req.Text, fixed.Int26_6(req.Size), shapeFaces, limits)
		if err != nil {
			fail(req.Key, err)
			continue
		}
		result := &shapedRecord{Glyphs: []glyphRecord{}, Advance: shaped.Advance, Runes: shaped.Runes, CoverageChecks: shaped.CoverageChecks, Runs: shaped.Runs}
		for _, g := range shaped.Glyphs {
			result.Glyphs = append(result.Glyphs, glyphRecord{
				ID: g.ID, Face: g.Face, PositionX: g.PositionX, PositionY: g.PositionY, Advance: g.Advance,
				Ink:    [4]int32{int32(g.Ink.Min.X), int32(g.Ink.Min.Y), int32(g.Ink.Max.X), int32(g.Ink.Max.Y)},
				HasInk: g.HasInk, Empty: g.Empty, Source: g.Source, SourceIndex: g.SourceIndex,
			})
		}
		emit(record{Key: req.Key, Shaped: result})
		for _, g := range shaped.Glyphs {
			if !g.HasInk {
				continue
			}
			name := names[g.Face]
			key := fmt.Sprintf("outline:%s:%d", name, g.ID)
			if seen[key] {
				continue
			}
			seen[key] = true
			var buffer sfnt.Buffer
			segments, err := shapeFaces[g.Face].Face.Outline.LoadGlyph(&buffer, sfnt.GlyphIndex(g.ID), fixed.I(64), nil)
			if err != nil {
				fail(key, err)
				continue
			}
			emit(record{Key: key, Segments: segmentRows(segments)})
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
