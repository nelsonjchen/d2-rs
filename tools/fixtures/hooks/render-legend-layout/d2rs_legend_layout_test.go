// SPDX-License-Identifier: MPL-2.0
// Oracle for crates/d2-rs-render/tests/fixtures/legend-layout-source-v090.json
// (docs/FIXTURES.md): legend panel layouts and label origins over label sets x
// font family x pad through public d2scenebuild.Build.
// Harvested from .reference/legend-layout-20261016/source-module.
// Overlaid into d2renderers/d2scenebuild as an external test (public APIs only); inert unless
// D2RS_RECIPE_OUTPUT is set.
package d2scenebuild_test
import("context";"testing";"encoding/json";"fmt";"os";"strings";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2scenebuild";"github.com/d2lang/d2/d2target";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestD2RSLegendLayout(t *testing.T) {
	outputPath := os.Getenv("D2RS_RECIPE_OUTPUT")
	if outputPath == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	stdout, err := os.Create(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	defer stdout.Close()
rows:=[]any{};labels:=[][]string{{"A","Longer relation"},{"",""},{"only"},{"  a   b \t c\nnext\rline  "},{"first\nsecond\nthird","😃漢字"},{""},{"supercalifragilisticexpialidocious"}};for ci,values:=range labels {for _,family:=range []string{"default","mono","hand"}{for _,pad:=range []int64{0,4,100}{
 d:=d2target.NewDiagram();switch family{case "mono":f:=d2fonts.SourceCodePro;d.FontFamily=&f;case "hand":f:=d2fonts.HandDrawn;d.FontFamily=&f};d.Root.Fill="#ffffff";d.Root.Stroke="none";s:=*d2target.BaseShape();s.ID="host";s.Type=d2target.ShapeRectangle;s.Pos=d2target.Point{X:10,Y:20};s.Width=100;s.Height=80;d.Shapes=[]d2target.Shape{s};d.Legend=&d2target.Legend{};if ci%2==0{d.Legend.Label="A very long title that does not determine panel width"}
 for i,v:=range values {e:=*d2target.BaseShape();e.ID=fmt.Sprintf("s%d",i);e.Type=d2target.ShapeRectangle;e.Label=v;e.FontFamily=family;d.Legend.Shapes=append(d.Legend.Shapes,e)}
 if ci!=2 {c:=*d2target.BaseConnection();c.ID="edge";c.Label="edge\nlabel";if ci==1||ci==5{c.Label=""};c.FontFamily=family;d.Legend.Connections=[]d2target.Connection{c}}
 tl,br:=d.BoundingBox();doc,err:=d2scenebuild.Build(context.Background(),d,d2scenebuild.Options{Pad:&pad});if err!=nil{panic(err)}
 nodes:=[]any{};var walk func(*d2scene.Node);walk=func(n *d2scene.Node){if n.ID=="legend:panel"{nodes=append(nodes,map[string]any{"id":n.ID,"primitive":n.Primitive})};if n.ID=="legend:title"||strings.HasSuffix(n.ID,":label"){switch p:=n.Primitive.(type){case d2scene.TextRun:nodes=append(nodes,map[string]any{"id":n.ID,"text":p.Text,"origin":p.Origin,"ink":p.Ink,"font":p.Font})}};for _,c:=range n.Children{walk(c)}};walk(doc.Root)
 rows=append(rows,map[string]any{"name":fmt.Sprintf("case%d-%s-pad%d",ci,family,pad),"family":family,"pad":pad,"legend":d.Legend,"bounds":[]int{tl.X,tl.Y,br.X,br.Y},"viewBox":doc.ViewBox,"nodes":nodes})
 }}};enc:=json.NewEncoder(stdout);enc.SetIndent("","  ");if err:=enc.Encode(rows);err!=nil{panic(err)}}
