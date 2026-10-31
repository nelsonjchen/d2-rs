// SPDX-License-Identifier: MPL-2.0
// Actual original adapter validator, through a virtual trace-worktree test.
package d2talalayout
import("context";"encoding/json";"os";"testing";"github.com/d2lang/d2/d2graph";"github.com/d2lang/d2/lib/geo")
func TestCodexAuthoredGeometry(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_GEOMETRY_INPUT"));if err!=nil{t.Fatal(err)}
 var inputs []json.RawMessage;if err=json.Unmarshal(data,&inputs);err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,raw:=range inputs{
  var r struct{Name string;ObjectID []byte;Width,Height,Top,Left *[]byte};if err=json.Unmarshal(raw,&r);err!=nil{t.Fatal(err)}
  scalar:=func(v *[]byte)*d2graph.Scalar{if v==nil{return nil};return &d2graph.Scalar{Value:string(*v)}}
  object:=&d2graph.Object{ID:string(r.ObjectID),Box:geo.NewBox(geo.NewPoint(0,0),100,80),Attributes:d2graph.Attributes{WidthAttr:scalar(r.Width),HeightAttr:scalar(r.Height),Top:scalar(r.Top),Left:scalar(r.Left)}}
  err=validateD2Objects(context.Background(),[]d2ObjectVisit{{object:object}},false);message:="";if err!=nil{message=err.Error()};rows=append(rows,map[string]any{"request":raw,"error":message})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_GEOMETRY_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
