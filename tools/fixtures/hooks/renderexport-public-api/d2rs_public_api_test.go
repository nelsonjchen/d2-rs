// SPDX-License-Identifier: MPL-2.0
// Oracle hook for the renderexport-public-* recipes (tools/fixtures/renderexport-export-public.toml):
// pinned lib/pptx, lib/pdf, lib/png and archive/zip public APIs. Overlaid into
// lib/pdf (package pdf) so it builds inside the pinned module. Inert unless run by
// scripts/regen-fixtures.py.
// Harvested from the v0.9.0-era witnesses .reference/codex-runs/witness-inputs/
// run{30,31,32,34,36,40}*.go and tools/pdf-master-probe/{png,documents}.
//
// $D2RS_PUBLIC_MODE selects the mode; files are written under $D2RS_PUBLIC_OUT
// (the recipe's stage or an intermediate directory). FPDF's supported fixed clock / catalog-sort
// controls are selected only where the original witness selected them.
package pdf

import (
	"archive/zip"
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash/crc32"
	"image"
	"image/color"
	"image/png"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"testing"
	"time"

	"codeberg.org/go-pdf/fpdf"
	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2target"
	d2png "github.com/d2lang/d2/lib/png"
	"github.com/d2lang/d2/lib/pptx"
	"github.com/d2lang/d2/lib/version"
)

func d2rsCheck(err error) {
	if err != nil {
		panic(err)
	}
}

func d2rsWrite(path string, data []byte) {
	d2rsCheck(os.MkdirAll(filepath.Dir(path), 0o755))
	d2rsCheck(os.WriteFile(path, data, 0o644))
}

func d2rsFixture(out, name string) string {
	return filepath.Join(out, "crates/d2-rs-export/tests/fixtures", name)
}

func TestD2RSPublicAPI(t *testing.T) {
	mode, out := os.Getenv("D2RS_PUBLIC_MODE"), os.Getenv("D2RS_PUBLIC_OUT")
	if mode == "" || os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	if runtime.Version() != "go1.27.0" {
		t.Fatal("requires pinned go1.27.0; got " + runtime.Version())
	}
	switch mode {
	case "pptx":
		d2rsPptxParts(out)
		d2rsPptxProperties(out)
	case "zip":
		d2rsZipEncoding(out, "../pptx/template.pptx")
	case "pdf-navigation":
		d2rsPdfNavigation(out)
	case "pdf-fonts":
		d2rsPdfFonts(out)
	case "png-exif":
		d2rsPngExif(out)
	case "pdf-master":
		d2rsPdfPNG(out)
		d2rsPdfDocuments(out)
	default:
		panic("unknown mode " + mode)
	}
}

// run31-pptx-parts.go: the eight generated XML parts, navigation on/off.
func d2rsPptxParts(out string) {
	var encoded bytes.Buffer
	d2rsCheck(png.Encode(&encoded, image.NewNRGBA(image.Rect(0, 0, 170, 100))))
	for _, nav := range []bool{false, true} {
		name := "plain"
		if nav {
			name = "nav"
		}
		p := pptx.NewPresentation("deck", "description", "subject", "creator", "0.9.0", nav)
		titles := []pptx.BoardTitle{{BoardID: "root", Name: "Root & <π>", LinkToSlide: 1}}
		slide, err := p.AddSlide(encoded.Bytes(), titles)
		d2rsCheck(err)
		slide.Links = []*pptx.Link{{ID: "link1", Index: 0, ExternalUrl: "https://example.com/?q=\"<&π", Tooltip: "URL <&π>", Left: 20, Top: 30, Width: 40, Height: 50}, {ID: "link2", Index: 1, SlideIndex: 2, Tooltip: "child", Left: 60, Top: 70, Width: 80, Height: 90}}
		titles = append(titles, pptx.BoardTitle{BoardID: "root.layers.\"<&π", Name: "Child \"<&π", LinkToSlide: 2})
		_, err = p.AddSlide(encoded.Bytes(), titles)
		d2rsCheck(err)
		var archive bytes.Buffer
		d2rsCheck(p.ExportTo(&archive))
		for name2, data := range d2rsZipParts(archive.Bytes(), "[Content_Types].xml", "_rels/.rels", "ppt/presentation.xml", "ppt/_rels/presentation.xml.rels", "ppt/slides/slide1.xml", "ppt/slides/slide2.xml", "ppt/slides/_rels/slide1.xml.rels", "ppt/slides/_rels/slide2.xml.rels") {
			d2rsWrite(d2rsFixture(out, filepath.Join("pptx-parts", name, name2)), data)
		}
	}
}

// run30-pptx-properties.go: docProps/{core,app}.xml with escaped metadata.
func d2rsPptxProperties(out string) {
	p := pptx.NewPresentation("Title & <deck> \"'\t\n\r\x00 π", "Description & <text>", "Subject ' \"", "Creator & <name>", "0.9.0", false)
	var encoded bytes.Buffer
	d2rsCheck(png.Encode(&encoded, image.NewNRGBA(image.Rect(0, 0, 1, 1))))
	for _, board := range []string{"root", "root.layers.\"<&π"} {
		_, err := p.AddSlide(encoded.Bytes(), []pptx.BoardTitle{{Name: "display", BoardID: board}})
		d2rsCheck(err)
	}
	var archive bytes.Buffer
	d2rsCheck(p.ExportTo(&archive))
	parts := d2rsZipParts(archive.Bytes(), "docProps/core.xml", "docProps/app.xml")
	d2rsWrite(d2rsFixture(out, "pptx-properties-core.xml"), parts["docProps/core.xml"])
	d2rsWrite(d2rsFixture(out, "pptx-properties-app.xml"), parts["docProps/app.xml"])
}

func d2rsZipParts(archive []byte, names ...string) map[string][]byte {
	z, err := zip.NewReader(bytes.NewReader(archive), int64(len(archive)))
	d2rsCheck(err)
	want := map[string]bool{}
	for _, n := range names {
		want[n] = true
	}
	parts := map[string][]byte{}
	for _, f := range z.File {
		if !want[f.Name] {
			continue
		}
		r, err := f.Open()
		d2rsCheck(err)
		b, err := io.ReadAll(r)
		d2rsCheck(err)
		r.Close()
		parts[f.Name] = b
	}
	if len(parts) != len(names) {
		panic(fmt.Sprintf("archive holds %d of %d parts", len(parts), len(names)))
	}
	return parts
}

// run34-encoders.go (ZIP half): archive/zip Create/Copy over the pinned PPTX template.
func d2rsZipEncoding(out, template string) {
	var outZip bytes.Buffer
	writer := zip.NewWriter(&outZip)
	add := func(name string, data []byte) {
		w, err := writer.Create(name)
		d2rsCheck(err)
		_, err = w.Write(data)
		d2rsCheck(err)
	}
	add("[Content_Types].xml", []byte("<Types/>"))
	t, err := zip.OpenReader(template)
	d2rsCheck(err)
	defer t.Close()
	for _, f := range t.File {
		if f.Name == "_rels/.rels" || f.Name == "ppt/slideMasters/_rels/slideMaster1.xml.rels" {
			continue
		}
		d2rsCheck(writer.Copy(f))
	}
	add("empty/", nil)
	add("empty.txt", nil)
	add("日本語/~name.txt", []byte("repeated archive contents repeated archive contents"))
	d2rsCheck(writer.Close())
	d2rsWrite(d2rsFixture(out, "zip-encoding-go127.zip"), outZip.Bytes())
}

func d2rsDeterministic(document *GoFPDF) {
	inner := document.pdf
	inner.SetCatalogSort(true)
	fixed := time.Date(2000, 1, 2, 3, 4, 5, 0, time.UTC)
	inner.SetCreationDate(fixed)
	inner.SetModificationDate(fixed)
}

// run32-pdf-navigation.go: unequal page heights, board links, navigation on/off.
// Writes <out>/{plain,nav}.pdf; scripts/export_pdf_fixtures.py extracts the fixture.
func d2rsPdfNavigation(out string) {
	for _, nav := range []bool{false, true} {
		name := "plain"
		if nav {
			name = "nav"
		}
		variants := d2rsStableExports(func() *GoFPDF {
			document := Init()
			d2rsDeterministic(document)
			for i, height := range []int{1200, 2} {
				var encoded bytes.Buffer
				d2rsCheck(png.Encode(&encoded, image.NewNRGBA(image.Rect(0, 0, 32, height))))
				titles := []BoardTitle{{Name: "Root", BoardID: "root"}}
				target := "root.layers.child"
				if i == 1 {
					titles = append(titles, BoardTitle{Name: "Child", BoardID: target})
					target = "root"
				}
				shape := d2target.Shape{Link: target, Width: 8, Height: 9}
				shape.Pos.X = 4
				shape.Pos.Y = 5
				shape.StrokeWidth = 1
				d2rsCheck(document.AddPDFPage(encoded.Bytes(), titles, 0, "#ffffff", []d2target.Shape{shape}, 0, 0, 0, map[string]int{"root": 0, "root.layers.child": 1}, nav))
			}
			return document
		}, filepath.Join(out, "../../scratch", name+".pdf"))
		// The extracted page sizes, link rects and destinations do not depend on
		// image object order, so the smallest variant stands for all of them.
		d2rsWrite(filepath.Join(out, name+".pdf"), variants[0])
	}
}

// run36-pdf-fonts.go: FPDF UTF-8 Source Sans Pro subsets. Writes <out>/<name>.pdf.
func d2rsPdfFonts(out string) {
	d2rsCheck(os.MkdirAll(out, 0o755))
	for _, test := range []struct{ name, regular, bold string }{
		{"empty", "", ""},
		{"board", "blah  /  ", "blahdog"},
		{"unicode", "Café (α) \\", "Résumé 東京"},
	} {
		doc := fpdf.NewCustom(&fpdf.InitType{UnitStr: "pt"})
		// FPDF's supported catalog-sort and fixed-clock controls: without the
		// sort, putfonts numbers the two fonts in Go map order.
		doc.SetCatalogSort(true)
		fixed := time.Date(2000, 1, 2, 3, 4, 5, 0, time.UTC)
		doc.SetCreationDate(fixed)
		doc.SetModificationDate(fixed)
		doc.AddUTF8FontFromBytes("source", "", d2fonts.FontFaces.Get(d2fonts.SourceSansPro.Font(0, d2fonts.FONT_STYLE_REGULAR)))
		doc.AddUTF8FontFromBytes("source", "B", d2fonts.FontFaces.Get(d2fonts.SourceSansPro.Font(0, d2fonts.FONT_STYLE_BOLD)))
		doc.AddPage()
		for _, text := range []struct{ style, value string }{{"", test.regular}, {"B", test.bold}} {
			doc.SetFont("source", text.style, 14)
			doc.CellFormat(300, 20, text.value, "", 0, "", false, 0, "")
		}
		d2rsCheck(doc.OutputFileAndClose(filepath.Join(out, test.name+".pdf")))
	}
}

// run40-png-exif.go: lib/png.AddExif over versions and opaque/alpha inputs.
func d2rsPngExif(out string) {
	var rows []map[string]any
	for _, opaque := range []bool{false, true} {
		img := image.NewNRGBA(image.Rect(0, 0, 3, 2))
		for i := range img.Pix {
			img.Pix[i] = byte(i * 19)
		}
		if opaque {
			for i := 3; i < len(img.Pix); i += 4 {
				img.Pix[i] = 255
			}
		}
		var b bytes.Buffer
		d2rsCheck((&png.Encoder{CompressionLevel: png.BestSpeed}).Encode(&b, img))
		for _, model := range []string{"", "a", "abc", "abcd", "éé", "v0.9.0", "v0.8.1-HEAD"} {
			version.Version = model
			o, err := d2png.AddExif(b.Bytes())
			d2rsCheck(err)
			rows = append(rows, map[string]any{"opaque": opaque, "version": model, "input": b.Bytes(), "expected": o})
		}
	}
	data, err := json.Marshal(rows)
	d2rsCheck(err)
	d2rsWrite(d2rsFixture(out, "png-exif-source-v090.json"), append(data, '\n'))
}

// tools/pdf-master-probe/png: AddPDFPage over PNGs at each compression level and split IDATs.
func d2rsSplitIDAT(input []byte) []byte {
	o := append([]byte{}, input[:8]...)
	for pos := 8; pos < len(input); {
		n := int(binary.BigEndian.Uint32(input[pos:]))
		if string(input[pos+4:pos+8]) != "IDAT" {
			o = append(o, input[pos:pos+n+12]...)
		} else {
			for data := input[pos+8 : pos+8+n]; len(data) > 0; {
				count := min(3, len(data))
				o = binary.BigEndian.AppendUint32(o, uint32(count))
				start := len(o)
				o = append(o, []byte("IDAT")...)
				o = append(o, data[:count]...)
				o = binary.BigEndian.AppendUint32(o, crc32.ChecksumIEEE(o[start:]))
				data = data[count:]
			}
		}
		pos += n + 12
	}
	return o
}

func d2rsExportBytes(document *GoFPDF, scratch string) []byte {
	d2rsCheck(os.MkdirAll(filepath.Dir(scratch), 0o755))
	d2rsCheck(document.Export(scratch))
	data, err := os.ReadFile(scratch)
	d2rsCheck(err)
	d2rsCheck(os.Remove(scratch))
	return data
}

// d2rsStableExports returns every distinct PDF that build produces, sorted.
// fpdf's putimages numbers image objects in Go map order, and catalog sort
// orders them only by width, so documents with several same-width images
// vary from run to run in Go itself. Exporting until no new variant appears
// for 256 runs records the whole set, which keeps the fixture deterministic
// and lets readers accept any output Go can produce.
func d2rsStableExports(build func() *GoFPDF, scratch string) [][]byte {
	seen := map[string]bool{}
	for run, last := 0, 0; run < 8192 && (run < 512 || run-last < 256); run++ {
		data := string(d2rsExportBytes(build(), scratch))
		if !seen[data] {
			seen[data] = true
			last = run
		}
	}
	keys := make([]string, 0, len(seen))
	for data := range seen {
		keys = append(keys, data)
	}
	sort.Strings(keys)
	variants := make([][]byte, len(keys))
	for i, data := range keys {
		variants[i] = []byte(data)
	}
	return variants
}

func d2rsPdfPNG(out string) {
	inputs := map[string][]byte{}
	for _, entry := range []struct {
		name  string
		level png.CompressionLevel
	}{
		{"default", png.DefaultCompression}, {"none", png.NoCompression},
		{"speed", png.BestSpeed}, {"best", png.BestCompression},
	} {
		img := image.NewNRGBA(image.Rect(0, 0, 4, 2))
		for y := 0; y < 2; y++ {
			for x := 0; x < 4; x++ {
				img.SetNRGBA(x, y, color.NRGBA{R: 60, G: 100, B: 180, A: 255})
			}
		}
		var buffer bytes.Buffer
		d2rsCheck((&png.Encoder{CompressionLevel: entry.level}).Encode(&buffer, img))
		inputs[entry.name] = buffer.Bytes()
	}
	inputs["split"] = d2rsSplitIDAT(inputs["default"])
	cases := []map[string]any{}
	for _, name := range []string{"default", "none", "speed", "best", "split", "mixed"} {
		pages := [][]byte{inputs[name]}
		if name == "mixed" {
			pages = [][]byte{inputs["default"], inputs["none"], inputs["speed"], inputs["best"], inputs["split"], inputs["default"]}
		}
		encodedPages := []string{}
		for _, data := range pages {
			encodedPages = append(encodedPages, hex.EncodeToString(data))
		}
		variants := d2rsStableExports(func() *GoFPDF {
			document := Init()
			d2rsDeterministic(document)
			for index, data := range pages {
				boardID := fmt.Sprintf("page%d", index)
				d2rsCheck(document.AddPDFPage(data, []BoardTitle{{Name: "Root", BoardID: boardID}}, 0, "#ffffff", nil, 0, 0, 0, map[string]int{boardID: index}, false))
			}
			return document
		}, filepath.Join(out, "scratch", name+".pdf"))
		row := map[string]any{"name": name, "pages": encodedPages, "pdf": hex.EncodeToString(variants[0])}
		if len(variants) > 1 {
			encodedVariants := []string{}
			for _, data := range variants {
				encodedVariants = append(encodedVariants, hex.EncodeToString(data))
			}
			row["pdf_variants"] = encodedVariants
		}
		cases = append(cases, row)
	}
	data, err := json.MarshalIndent(cases, "", "  ")
	d2rsCheck(err)
	d2rsWrite(d2rsFixture(out, "pdf-png-idat-fpdf-master.json"), append(data, '\n'))
}

// tools/pdf-master-probe/documents: links, navigation, page fills, opaque images.
func d2rsPdfDocuments(out string) {
	documents := map[string]string{}
	for _, name := range []string{"plain", "nav", "opaque", "opaque_default"} {
		document := Init()
		d2rsDeterministic(document)
		widths := []int{32, 8, 12, 8}
		heights := []int{1200, 2, 6, 2}
		if name == "opaque" || name == "opaque_default" {
			widths = []int{4}
			heights = []int{2}
		}
		for i, width := range widths {
			img := image.NewNRGBA(image.Rect(0, 0, width, heights[i]))
			if name == "opaque" || name == "opaque_default" {
				for y := 0; y < heights[i]; y++ {
					for x := 0; x < width; x++ {
						img.SetNRGBA(x, y, color.NRGBA{R: 60, G: 100, B: 180, A: 255})
					}
				}
			}
			var encoded bytes.Buffer
			level := png.BestSpeed
			if name == "opaque_default" {
				level = png.DefaultCompression
			}
			d2rsCheck((&png.Encoder{CompressionLevel: level}).Encode(&encoded, img))
			titles := []BoardTitle{{Name: "Root (α) \\", BoardID: "root"}}
			if i > 0 {
				titles = append(titles, BoardTitle{Name: "Child café", BoardID: []string{"root", "child", "other", "same"}[i]})
			}
			shape := d2target.Shape{Link: "root", Width: 8, Height: 9}
			shape.Pos.X = 4
			shape.Pos.Y = 5
			shape.StrokeWidth = 1
			shapes := []d2target.Shape{shape}
			if i == 0 {
				shapes[0].Link = "https://example.test/(x)"
			}
			fill := "#ffffff"
			if i == 1 {
				fill = "#101020"
			}
			d2rsCheck(document.AddPDFPage(encoded.Bytes(), titles, 0, fill, shapes, 0, 0, 0, map[string]int{"root": 0, "child": 1, "other": 2, "same": 3}, name == "nav"))
		}
		data := d2rsExportBytes(document, filepath.Join(out, "scratch", name+".pdf"))
		if name == "opaque_default" {
			d2rsWrite(d2rsFixture(out, "pdf-opaque-default-fpdf-master.pdf"), data)
		} else {
			documents[name] = hex.EncodeToString(data)
		}
	}
	data, err := json.MarshalIndent(documents, "", "  ")
	d2rsCheck(err)
	d2rsWrite(d2rsFixture(out, "pdf-documents-fpdf-master.json"), append(data, '\n'))
}
