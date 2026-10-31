// SPDX-License-Identifier: MPL-2.0
// Original Go D2 v0.9.0 target bounds, virtual test overlay only.
package d2target
import("encoding/json";"os";"net/url";"testing")
func TestCodexOutsideIconBounds(t *testing.T){
 rows:=[]map[string]any{};icon,err:=url.Parse("data:image/svg+xml;base64,PHN2Zy8+");if err!=nil{t.Fatal(err)}
 for _,kind:=range []string{"rectangle","oval","diamond","hexagon","cloud","cylinder","person","document"}{
  for _,pos:=range []string{"OUTSIDE_TOP_LEFT","OUTSIDE_TOP_CENTER","OUTSIDE_TOP_RIGHT","OUTSIDE_BOTTOM_LEFT","OUTSIDE_BOTTOM_CENTER","OUTSIDE_BOTTOM_RIGHT","OUTSIDE_LEFT_TOP","OUTSIDE_LEFT_MIDDLE","OUTSIDE_LEFT_BOTTOM","OUTSIDE_RIGHT_TOP","OUTSIDE_RIGHT_MIDDLE","OUTSIDE_RIGHT_BOTTOM"}{
   d:=Diagram{Shapes:[]Shape{{ID:"probe",Type:kind,Pos:Point{X:101,Y:202},Width:114,Height:98,StrokeWidth:2,Opacity:1,Icon:icon,IconPosition:pos}}};tl,br:=d.BoundingBox();rows=append(rows,map[string]any{"type":kind,"position":pos,"min":tl,"max":br})
  }
 }
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_OUTSIDE_ICON_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
