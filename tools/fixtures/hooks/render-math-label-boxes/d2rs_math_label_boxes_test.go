// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/math-label-boxes-source-v090.json
// (docs/FIXTURES.md): intrinsic LaTeX label image placement per shape kind x
// label position x 3d/multiple effect x font size. Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set. Harvested
// from the v0.9.0-era codex_math_label_boxes overlay.
// Original Go v0.9.0 intrinsic math image placement, virtual overlay only.
package d2scenebuild
import("context";"encoding/json";"fmt";"math";"os";"testing";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2latex";"github.com/d2lang/d2/d2target")
func TestD2RSMathLabelBoxes(t *testing.T){
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}

 rows:=[]map[string]any{};formula:=`1 + 1`;width,height,err:=d2latex.Measure(formula);if err!=nil{t.Fatal(err)}
 for _,kind:=range []string{"rectangle","oval","diamond"}{for _,position:=range []string{"UNSET","INSIDE_MIDDLE_CENTER","INSIDE_TOP_LEFT","OUTSIDE_TOP_RIGHT","BORDER_TOP_CENTER","BORDER_LEFT_MIDDLE"}{for _,effect:=range d2rsMathBoxEffects(kind){for _,fontSize:=range []int{16,30}{
  shape:=structuredTestShape("formula",kind,20,30,200,130);shape.Label=formula;shape.Language="latex";shape.LabelWidth=width;shape.LabelHeight=height;shape.FontSize=fontSize;shape.Color="#ff00ff";shape.Stroke="#123456";shape.LabelPosition=position;shape.ThreeDee=effect=="3d";shape.Multiple=effect=="multiple"
  diagram:=d2target.NewDiagram();diagram.Root.Fill="#ffffff";diagram.Root.Stroke="none";diagram.Shapes=[]d2target.Shape{shape};pad:=int64(0);doc,err:=Build(context.Background(),diagram,Options{Pad:&pad,Assets:testAssetOptions(t)});if err!=nil{t.Fatal(err)};image:=findSceneNode(t,doc.Root,"formula:label:0").Primitive.(d2scene.Image)
  rows=append(rows,map[string]any{"kind":kind,"position":position,"effect":effect,"font_size":fontSize,"label_width":width,"label_height":height,"x_bits":fmt.Sprintf("%016x",math.Float64bits(image.Box.X)),"y_bits":fmt.Sprintf("%016x",math.Float64bits(image.Box.Y)),"intrinsic_width":image.Box.Width,"intrinsic_height":image.Box.Height})
 }}}}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}

// Source only supports 3D rectangle among these target types.
func d2rsMathBoxEffects(kind string) []string {if kind=="rectangle" {return []string{"none","3d","multiple"}};return []string{"none","multiple"}}
