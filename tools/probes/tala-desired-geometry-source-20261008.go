// SPDX-License-Identifier: MPL-2.0
package d2talalayout
import("context";"encoding/json";"os";"testing";"github.com/d2lang/d2/d2graph";"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph";"github.com/d2lang/d2/lib/geo")
func TestCodexDesiredGeometry(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_GEOMETRY_INPUT"));if err!=nil{t.Fatal(err)}
 var inputs []json.RawMessage;if err=json.Unmarshal(data,&inputs);err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,raw:=range inputs{
  var r struct{Name,Shape string;Measured [2]float64;Width,Height,Top,Left *string};if err=json.Unmarshal(raw,&r);err!=nil{t.Fatal(err)}
  scalar:=func(v *string)*d2graph.Scalar{if v==nil{return nil};return &d2graph.Scalar{Value:*v}}
  g:=d2graph.NewGraph();o:=&d2graph.Object{Graph:g,Parent:g.Root,ID:"node",IDVal:"node",Box:geo.NewBox(geo.NewPoint(0,0),r.Measured[0],r.Measured[1]),Children:map[string]*d2graph.Object{},Attributes:d2graph.Attributes{Shape:d2graph.Scalar{Value:r.Shape},WidthAttr:scalar(r.Width),HeightAttr:scalar(r.Height),Top:scalar(r.Top),Left:scalar(r.Left)}};g.Root.Children["node"]=o;g.Root.ChildrenArray=[]*d2graph.Object{o};g.Objects=[]*d2graph.Object{o}
  tg:=layoutgraph.NewGraph();_,err=translateGraph(context.Background(),g,tg,false);message:="";var state any
  if err!=nil{message=err.Error()}else{if len(tg.Nodes)!=1{t.Fatal("incomplete translated nodes")};n:=tg.Nodes[0];var fixed any;if n.FixedTopLeft!=nil{fixed=[2]float64{n.FixedTopLeft.X,n.FixedTopLeft.Y}};state=map[string]any{"measured":[2]float64{n.Width,n.Height},"desired":[2]*float64{n.DesiredWidth,n.DesiredHeight},"fixed":fixed}}
  rows=append(rows,map[string]any{"request":raw,"error":message,"state":state})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_GEOMETRY_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
