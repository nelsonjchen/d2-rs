// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/class-text-origins-source-v090.json
// (docs/FIXTURES.md): class shape TextRun origins (float bits) per font size x
// field/method counts. Overlaid into d2renderers/d2scenebuild; inert unless
// D2RS_RECIPE_OUTPUT is set. Harvested from the v0.9.0-era codex_class_text_origins overlay.
// Original Go D2 v0.9.0 class TextRun controls; virtual overlay only.
package d2scenebuild
import("encoding/json";"fmt";"math";"os";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2target")
func TestD2RSClassTextOrigins(t *testing.T){
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}

 rows:=[]map[string]any{}
 for _,fontSize:=range []int{16,20,30}{for _,counts:=range [][2]int{{4,0},{1,3},{7,2},{0,0}}{
  shape:=structuredTestShape("class",d2target.ShapeClass,1032,101,590,500);shape.FontSize=fontSize;shape.Label="C";shape.LabelWidth=24;shape.LabelHeight=25
  for i:=0;i<counts[0];i++{shape.Fields=append(shape.Fields,d2target.ClassField{Name:fmt.Sprintf("f%d",i),Type:"int",Visibility:"public"})};for i:=0;i<counts[1];i++{shape.Methods=append(shape.Methods,d2target.ClassMethod{Name:fmt.Sprintf("m%d()",i),Return:"void",Visibility:"public"})}
  doc:=buildStructuredDocument(t,shape);node:=findSceneNode(t,doc.Root,shape.ID);runs:=[]map[string]any{}
  for _,child:=range node.Children{if run,ok:=child.Primitive.(d2scene.TextRun);ok{runs=append(runs,map[string]any{"text":run.Text,"x_bits":fmt.Sprintf("%016x",math.Float64bits(run.Origin.X)),"y_bits":fmt.Sprintf("%016x",math.Float64bits(run.Origin.Y)),"anchor":run.Anchor})}}
  rows=append(rows,map[string]any{"font_size":fontSize,"fields":counts[0],"methods":counts[1],"runs":runs})
 }}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
