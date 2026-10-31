// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): transformed stroked Source Code Pro text frames.
// Harvested from the v0.9.0-era probe d2raster/text_stroke_probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2raster

import (
 "context"
 "encoding/json"
 "image/color"
 "math"
 "os"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSTextStroke(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 font,err:=os.ReadFile("../d2fonts/ttf/SourceCodePro-Regular.ttf");if err!=nil {t.Fatal(err)}
 type row struct { Size float64 `json:"size"`; Width float64 `json:"width"`; Scale int `json:"scale"`; Matrix [6]float64 `json:"matrix"`; Pixels []uint8 `json:"pixels"` }
 rows:=[]row{}
 matrices:=[][6]float64{{1,0,0,1,0,0},{1.25,0,0,.75,3.125,4.5},{1,0,.25,1,3.125,4.5}}
 for _,size:=range []float64{12*.85,23*.85,32*.85} {for _,scale:=range []int{1,2} {for _,m:=range matrices {
  width:=math.Max(.35,size/32)
  paint:=d2scene.SolidPaint{Color:color.NRGBA{R:10,G:15,B:37,A:255}}
  node:=d2scene.NewNode(d2scene.TextRun{Text:"Agm",Origin:d2scene.Point{X:5.44,Y:32},Font:d2scene.Font{Family:"Source Code Pro",Size:size,Asset:"font"},Fill:paint,Stroke:&d2scene.Stroke{Paint:paint,Width:width,Cap:d2scene.CapRound,Join:d2scene.JoinRound,MiterLimit:4}})
  node.Transform=d2scene.Matrix{A:m[0],B:m[1],C:m[2],D:m[3],E:m[4],F:m[5]}
  document:=d2scene.NewDocument(d2scene.Box{Width:100,Height:64},node)
  document.Assets["font"]=d2scene.FontAsset{MIMEType:"font/ttf",Data:font}
  options:=testOptions();options.Scale=float64(scale);options.Background=color.NRGBA{R:255,G:255,B:255,A:255}
  frame,err:=Render(context.Background(),document,options);if err!=nil {t.Fatal(err)}
  rows=append(rows,row{size,width,scale,m,append([]uint8(nil),frame.Pix...)})
 }}}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil {t.Fatal(err)}
 if err:=os.WriteFile(out,data,0644);err!=nil {t.Fatal(err)}
 t.Logf("%d complete original text stroke frames",len(rows))
}
