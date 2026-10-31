// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
 "context"
 "encoding/json"
 "image"
 "os"
 "path/filepath"
 "reflect"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2isometric"
)
var d2rsGeometryRows []any
func d2rsGeometryStart(t *testing.T) {
 d2rsGeometryRows=[]any{}
 t.Cleanup(func(){ data,err:=json.Marshal(map[string]any{"name":t.Name(),"operations":d2rsGeometryRows});if err!=nil {t.Fatal(err)};if err=os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"),t.Name()+".json"),data,0600);err!=nil {t.Fatal(err)} })
}
func d2rsInkSegments(ctx context.Context,n d2isometric.Node,ts []Triangle,radius ...float64)([]classicInkSegment,error){
 triangles:=[]any{}
 zeroSlice:=len(ts)>maxClassicInkTriangles
 for _,tr:=range ts {if zeroSlice {if !reflect.DeepEqual(tr,Triangle{}) {panic("oversized fixture is not a zero slice")};continue};var material any;if tr.Material!=nil {material=map[string]any{"color":tr.Material.Color,"texture":tr.Material.Texture!=nil}};triangles=append(triangles,map[string]any{"vertices":tr.V,"bias":tr.DepthBias,"noDepth":tr.NoDepthWrite,"shadow":tr.CastShadow,"material":material})}
 v,err:=classicInkSegments(ctx,n,ts,radius...)
 message:="";if err!=nil {message=err.Error()}
 req:=map[string]any{"op":"segments","canceled":ctx.Err()!=nil,"node":n,"triangles":triangles,"zeroCount":0,"radius":radius}
 if zeroSlice {req["zeroCount"]=len(ts)}
 data,e:=json.Marshal(map[string]any{"request":req,"got":map[string]any{"count":len(v),"error":message}});if e!=nil {panic(e)}
 d2rsGeometryRows=append(d2rsGeometryRows,json.RawMessage(data));return v,err
}

func d2rsInkFit(b *meshBuilder, tex *image.RGBA, hex bool) float64 {
 var storage any
 if tex!=nil {storage=map[string]any{"rect":tex.Rect,"stride":tex.Stride,"pixels":tex.Pix}}
 prior:="";if b.err!=nil {prior=b.err.Error()}
 req:=map[string]any{"op":"fit","canceled":b.ctx.Err()!=nil,"error":prior,"texture":storage,"hex":hex}
 fit:=b.solidInkFit(tex,hex)
 message:="";if b.err!=nil {message=b.err.Error()}
 data,err:=json.Marshal(map[string]any{"request":req,"got":map[string]any{"fit":fit,"error":message}});if err!=nil {panic(err)}
 d2rsGeometryRows=append(d2rsGeometryRows,json.RawMessage(data));return fit
}
