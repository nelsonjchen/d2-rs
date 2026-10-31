// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/live-external-label-source-v090.json
// (docs/FIXTURES.md): rendered outside-label text origins and viewBoxes per
// variant x label position x size. Overlaid into d2renderers/d2svg; inert unless
// D2RS_RECIPE_OUTPUT is set. Harvested from the v0.9.0-era
// d2renderers/d2svg/live_external_label_probe_test.go.
package d2svg
import("testing";"os";"encoding/json";"encoding/xml";"bytes";"io";"strconv";"github.com/d2lang/d2/d2target";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestD2RSLiveExternalLabel(t *testing.T){
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}

 positions:=[]string{"OUTSIDE_TOP_LEFT","OUTSIDE_TOP_CENTER","OUTSIDE_TOP_RIGHT","OUTSIDE_LEFT_TOP","OUTSIDE_LEFT_MIDDLE","OUTSIDE_LEFT_BOTTOM","OUTSIDE_RIGHT_TOP","OUTSIDE_RIGHT_MIDDLE","OUTSIDE_RIGHT_BOTTOM","OUTSIDE_BOTTOM_LEFT","OUTSIDE_BOTTOM_CENTER","OUTSIDE_BOTTOM_RIGHT"};rows:=[]map[string]any{}
 for _,variant:=range []string{"rectangle","rectangle-multiple","rectangle-3d","person","person-multiple"}{for _,position:=range positions{for _,grow:=range []bool{true,false}{
 width,height,size:=129,27,20;if !grow{width,height,size=73,15,12};kind:="rectangle";if variant=="person"||variant=="person-multiple"{kind="person"}
 s:=d2target.Shape{ID:"probe",Type:kind,Pos:d2target.Point{X:15,Y:26},Width:160,Height:160,Opacity:1,StrokeWidth:2,Fill:"B3",Stroke:"B1",Multiple:variant=="rectangle-multiple"||variant=="person-multiple",ThreeDee:variant=="rectangle-3d",LabelPosition:position,Text:d2target.Text{Label:"Probe",LabelWidth:width,LabelHeight:height,FontSize:size,FontFamily:"DEFAULT",Color:"N1",Bold:true}}
 font:=d2fonts.SourceSansPro;mono:=d2fonts.SourceCodePro;data,err:=Render(&d2target.Diagram{Shapes:[]d2target.Shape{s},FontFamily:&font,MonoFontFamily:&mono},nil);if err!=nil{t.Fatal(err)};decoder:=xml.NewDecoder(bytes.NewReader(data));viewbox:="";x,y:=0.,0.;count:=0
 for{token,err:=decoder.Token();if err==io.EOF{break};if err!=nil{t.Fatal(err)};if el,ok:=token.(xml.StartElement);ok{if el.Name.Local=="svg"&&viewbox==""{for _,a:=range el.Attr{if a.Name.Local=="viewBox"{viewbox=a.Value}}};if el.Name.Local=="text"{count++;for _,a:=range el.Attr{if a.Name.Local=="x"{x,err=strconv.ParseFloat(a.Value,64)};if a.Name.Local=="y"{y,err=strconv.ParseFloat(a.Value,64)};if err!=nil{t.Fatal(err)}}}}};if count!=1{t.Fatalf("text count %d",count)}
 rows=append(rows,map[string]any{"variant":variant,"position":position,"grow":grow,"width":width,"height":height,"font_size":size,"x":x,"y":y,"viewbox":viewbox})
 }}}
 data,err:=json.MarshalIndent(rows,"","  ");if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"),data,0644);err!=nil{t.Fatal(err)}
}
