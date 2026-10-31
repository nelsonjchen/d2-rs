// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/polygon-arrowheads-source-v090.json
// (docs/FIXTURES.md): d2scenebuild polygon arrowhead paths and placement
// matrices (float bits) per arrowhead x stroke width x side x direction.
// Overlaid into d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set.
// Harvested from the v0.9.0-era codex_polygon_arrowheads overlay.
// Virtual test overlay for the pinned original Go D2 v0.9.0 scene builder.
package d2scenebuild
import (
 "context"
 "encoding/json"
 "fmt"
 "image/color"
 "math"
 "os"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/d2target"
 "github.com/d2lang/d2/d2themes/d2themescatalog"
 "github.com/d2lang/d2/lib/geo"
)
func TestD2RSPolygonArrowheads(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}

 b:=builder{ctx:context.Background(),theme:d2themescatalog.NeutralDefault}
 rows:=[]map[string]any{}
 paint:=d2scene.SolidPaint{Color:color.NRGBA{R:13,G:50,B:178,A:255}}
 for _, name:=range []string{"arrow","unfilled-triangle","line","diamond","filled-diamond","box","filled-box"} {
  for _, width:=range []int{1,2,4,8,15} {
   for _, target:=range []bool{false,true} {
    for _, delta:=range []d2scene.Point{{X:20,Y:10},{X:0,Y:20},{X:-20,Y:-10}} {
     start:=d2scene.Point{X:80.25,Y:190.75};end:=d2scene.Point{X:start.X+delta.X,Y:start.Y+delta.Y};endpoint:=start;if target {endpoint=end};endpoint.X-=0.5;endpoint.Y+=0.25
     node,err:=b.arrowhead(d2target.Connection{ID:"probe",StrokeWidth:width},d2target.Arrowhead(name),target,endpoint,geo.NewVector(delta.X,delta.Y),paint);if err!=nil {t.Fatal(err)}
     path,ok:=node.Primitive.(d2scene.Path);if !ok {t.Fatal("expected path")}
     commands:=[]map[string]any{};for _, command:=range path.Commands {commands=append(commands,map[string]any{"kind":command.Kind,"x":command.P1.X,"y":command.P1.Y})}
     matrix:=[]string{};for _, v:=range []float64{node.Transform.A,node.Transform.C,node.Transform.B,node.Transform.D,node.Transform.E,node.Transform.F} {matrix=append(matrix,fmt.Sprintf("%016x",math.Float64bits(v)))}
     row:=map[string]any{"name":name,"width":width,"target":target,"delta":[]float64{delta.X,delta.Y},"commands":commands,"matrix_bits":matrix,"fill":path.Fill!=nil,"stroke":path.Stroke!=nil}
     if path.Stroke!=nil {row["stroke_width"]=path.Stroke.Width;row["cap"]=path.Stroke.Cap;row["join"]=path.Stroke.Join;row["miter_limit"]=path.Stroke.MiterLimit}
     rows=append(rows,row)
    }
   }
  }
 }
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil {t.Fatal(err)}
 if err=os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"),append(data,'\n'),0644);err!=nil {t.Fatal(err)}
}
