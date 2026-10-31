// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/outside-icon-bounds-source-v090.json
// (docs/FIXTURES.md): d2target BoundingBox of one shape with an outside icon per
// shape kind x icon position. Overlaid into d2target; inert unless
// D2RS_RECIPE_OUTPUT is set. Harvested from the v0.9.0-era codex_outside_icon_bounds overlay.
// Original Go D2 v0.9.0 target bounds, virtual test overlay only.
package d2target
import("encoding/json";"os";"net/url";"testing")
func TestD2RSOutsideIconBounds(t *testing.T){
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}

 rows:=[]map[string]any{};icon,err:=url.Parse("data:image/svg+xml;base64,PHN2Zy8+");if err!=nil{t.Fatal(err)}
 for _,kind:=range []string{"rectangle","oval","diamond","hexagon","cloud","cylinder","person","document"}{
  for _,pos:=range []string{"OUTSIDE_TOP_LEFT","OUTSIDE_TOP_CENTER","OUTSIDE_TOP_RIGHT","OUTSIDE_BOTTOM_LEFT","OUTSIDE_BOTTOM_CENTER","OUTSIDE_BOTTOM_RIGHT","OUTSIDE_LEFT_TOP","OUTSIDE_LEFT_MIDDLE","OUTSIDE_LEFT_BOTTOM","OUTSIDE_RIGHT_TOP","OUTSIDE_RIGHT_MIDDLE","OUTSIDE_RIGHT_BOTTOM"}{
   d:=Diagram{Shapes:[]Shape{{ID:"probe",Type:kind,Pos:Point{X:101,Y:202},Width:114,Height:98,StrokeWidth:2,Opacity:1,Icon:icon,IconPosition:pos}}};tl,br:=d.BoundingBox();rows=append(rows,map[string]any{"type":kind,"position":pos,"min":tl,"max":br})
  }
 }
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
