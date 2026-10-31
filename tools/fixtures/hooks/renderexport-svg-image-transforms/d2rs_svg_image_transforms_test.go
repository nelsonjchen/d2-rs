// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): imported SVG path matrices under vector-image placement.
// Harvested from the v0.9.0-era probe tools/probes/svg-image-transforms-20261009.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2svgimport
import("context";"encoding/json";"os";"testing";"github.com/d2lang/d2/d2renderers/d2scene")
func TestD2RSSVGImageTransforms(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 data,err:=os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,SVG,Aspect string;Box,Canvas [4]float64;Scale float64};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,request:=range requests {
  imported,err:=ImportNode(context.Background(),"control",[]byte(request.SVG),generousImportLimits());if err!=nil{t.Fatal(request.Name,err)}
  aspect:=d2scene.AspectRatio{Align:d2scene.AlignXMidYMid,Fit:d2scene.AspectMeet};if request.Aspect=="none"{aspect.Align=d2scene.AlignNone};if request.Aspect=="xMinYMin meet"{aspect.Align=d2scene.AlignXMinYMin};if request.Aspect=="xMaxYMax slice"{aspect.Align=d2scene.AlignXMaxYMax;aspect.Fit=d2scene.AspectSlice}
  display,err:=d2scene.AspectRatioMatrix(d2scene.Box{Width:imported.Width,Height:imported.Height},d2scene.Box{Width:request.Box[2],Height:request.Box[3]},aspect);if err!=nil{t.Fatal(err)}
  outer:=d2scene.Scale(request.Scale,request.Scale).Mul(d2scene.Translate(-request.Canvas[0],-request.Canvas[1])).Mul(d2scene.Translate(request.Box[0],request.Box[1]))
  prefix:=outer.Mul(display).Mul(imported.ViewportTransform);matrices:=map[string][6]float64{}
  var walk func(*d2scene.Node,d2scene.Matrix);walk=func(node *d2scene.Node,parent d2scene.Matrix){if node==nil{return};matrix:=parent.Mul(node.Transform);if _,ok:=node.Primitive.(d2scene.Path);ok{matrices[node.ID]=[6]float64{matrix.A,matrix.B,matrix.C,matrix.D,matrix.E,matrix.F}};for _,child:=range node.Children{walk(child,matrix)}};walk(imported.Root,prefix)
  rows=append(rows,map[string]any{"request":request,"width":imported.Width,"height":imported.Height,"matrices":matrices})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(out,append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
