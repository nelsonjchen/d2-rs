// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): synthetic-italic Markdown code frames, matrices and reference bounds.
// Harvested from the v0.9.0-era probe d2raster/markdown_italic_ancestry_probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2raster

import (
 "context"
 "encoding/json"
 "fmt"
 "image"
 "image/color"
 "math"
 "os"
 "strings"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2fonts"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/lib/textmeasure"
)

func TestD2RSMarkdownItalicAncestry(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 family:=d2fonts.SourceCodePro
 ruler,err:=textmeasure.NewRuler();if err!=nil {t.Fatal(err)}
 font,err:=os.ReadFile("../d2fonts/ttf/SourceCodePro-Regular.ttf");if err!=nil {t.Fatal(err)}
 type result struct {SVG string `json:"svg"`; Scale float64 `json:"scale"`; Reference image.Rectangle `json:"reference"`; Pixels []uint8 `json:"pixels"`; Matrix d2scene.Matrix `json:"matrix"`}
 rows:=[]result{}
 for _,source:=range []string{"*`Agm`*"} {for _,size:=range []int{12,16,23,32} {for _,scale:=range []float64{1,2} {for _,position:=range [][2]float64{{12.5,7.25},{-8.25,-5.5}} {for _,underline:=range []bool{false} {
  layout,err:=textmeasure.LayoutMarkdown(source,ruler,&family,&family,size);if err!=nil {t.Fatal(err)}
  root:=d2scene.NewNode(nil);root.Transform=d2scene.Translate(position[0],position[1])
  root.Clip=&d2scene.Clip{Path:d2scene.Path{Commands:[]d2scene.PathCommand{d2scene.MoveTo(0,0),d2scene.LineTo(float64(layout.Width),0),d2scene.LineTo(float64(layout.Width),float64(layout.Height)),d2scene.LineTo(0,float64(layout.Height)),d2scene.ClosePath()}},Transform:d2scene.Identity()}
  var svg strings.Builder
  fmt.Fprintf(&svg,`<svg xmlns="http://www.w3.org/2000/svg" width="100" height="64" viewBox="0 0 100 64" preserveAspectRatio="xMinYMin meet"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="100" height="64" viewBox="0 0 100 64"><svg id="d2-rs-native-md-viewport-control" data-d2-rs-markdown-reference="ordinary" x="%g" y="%g" width="%.3f" height="%.3f" viewBox="0 0 %.3f %.3f" overflow="hidden"><g>`,position[0],position[1],float64(layout.Width),float64(layout.Height),float64(layout.Width),float64(layout.Height))
  for i,p:=range layout.Primitives {switch p.Kind {
  case textmeasure.MarkdownRectPrimitive:
   root.Children=append(root.Children,d2scene.NewNode(d2scene.Rect{Box:d2scene.Box{X:p.X,Y:p.Y,Width:p.Width,Height:p.Height},RadiusX:p.Radius,RadiusY:p.Radius,Fill:d2scene.SolidPaint{Color:color.NRGBA{R:238,G:241,B:248,A:255}}}))
   fmt.Fprintf(&svg,`<rect id="d2-rs-native-md-rect-%d" x="%g" y="%g" width="%g" height="%g" rx="%g" fill="#eef1f8"/>`,i,p.X,p.Y,p.Width,p.Height,p.Radius)
  case textmeasure.MarkdownTextPrimitive:
   root.Children=append(root.Children,d2scene.NewNode(d2scene.TextRun{Text:p.Text,Origin:d2scene.Point{X:p.X,Y:p.Y},Font:d2scene.Font{Asset:"font",Family:"Source Code Pro",Size:p.FontSize},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:10,G:15,B:37,A:255}},Underline:underline,Ink:d2scene.NewBounds(p.X,p.Y-p.Height,p.X+p.Width,p.Y+p.FontSize*.25)}))
   if p.SyntheticItalic {root.Children[len(root.Children)-1].Transform=d2scene.Translate(p.X,p.Y).Mul(d2scene.SkewX(-12*math.Pi/180)).Mul(d2scene.Translate(-p.X,-p.Y))}
   decoration:="";if underline {decoration=` text-decoration="underline"`}
   if p.SyntheticItalic {decoration+=` class="md-text text-mono" font-style="italic"`}
   fmt.Fprintf(&svg,`<text xml:space="preserve" id="native-%d" x="%g" y="%g" data-d2-rs-text-x-bits="%016x" data-d2-rs-text-y-bits="%016x" font-family="Source Code Pro" font-size="%g" fill="#0a0f25"%s>%s</text>`,i,p.X,p.Y,math.Float64bits(p.X),math.Float64bits(p.Y),p.FontSize,decoration,p.Text)
  }}
  svg.WriteString(`</g></svg></svg></svg>`)
  document:=d2scene.NewDocument(d2scene.Box{Width:100,Height:64},root);document.Assets["font"]=d2scene.FontAsset{MIMEType:"font/ttf",Data:font}
  options:=testOptions();options.Scale=scale;options.Background=color.NRGBA{R:255,G:255,B:255,A:255}
  prepared,err:=prepare(context.Background(),document,options);if err!=nil {t.Fatal(err)}
  if err:=setBandReferenceBounds(context.Background(),prepared.root,image.Rect(0,0,prepared.width,prepared.height));err!=nil {t.Fatal(err)}
  reference:=image.Rectangle{};for _,child:=range prepared.root.children {if child.primitive!=nil {reference=child.primitive.referenceBounds;break}}
  frame,err:=Render(context.Background(),document,options);if err!=nil {t.Fatal(err)}
  rows=append(rows,result{svg.String(),scale,reference,append([]uint8(nil),frame.Pix...),prepared.root.children[len(prepared.root.children)-1].primitive.transform})
 }}}}}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil {t.Fatal(err)}
 if err:=os.WriteFile(out,data,0644);err!=nil {t.Fatal(err)}
}
