// SPDX-License-Identifier: MPL-2.0
package d2isometricimg
import (
 "encoding/json"
 "math"
 "os"
 "path/filepath"
 "sort"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2isometric"
)
var d2rsGeometryRows []any
func d2rsGeometryStart(t *testing.T) {
 d2rsGeometryRows=[]any{}
 t.Cleanup(func(){ b,e:=json.Marshal(map[string]any{"name":t.Name(),"operations":d2rsGeometryRows});if e!=nil {t.Fatal(e)};if e=os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"),t.Name()+".json"),b,0600);e!=nil {t.Fatal(e)} })
}
func arrowNumber(v float64) any {if math.IsInf(v,1){return "+Inf"};if math.IsInf(v,-1){return "-Inf"};return v}
func arrowRecord(req,got any){b,e:=json.Marshal(map[string]any{"request":req,"got":got});if e!=nil {panic(e)};var v any;if e=json.Unmarshal(b,&v);e!=nil{panic(e)};d2rsGeometryRows=append(d2rsGeometryRows,v)}
func arrowError(b *meshBuilder) string {if b.err!=nil{return b.err.Error()};return ""}
func arrowTriangles(ts []Triangle) []any {out:=[]any{};for _,t:=range ts {var m any;if t.Material!=nil {m=map[string]any{"color":t.Material.Color,"texture":t.Material.Texture!=nil,"unlit":t.Material.Unlit}};out=append(out,map[string]any{"points":[]Vec{t.V[0].Position,t.V[1].Position,t.V[2].Position},"bias":t.DepthBias,"noDepth":t.NoDepthWrite,"coverage":t.svgCoverageOnly,"material":m})};return out}
func arrowPoint(p routeCaptionPoint) any {return map[string]any{"x":p.x,"z":p.z}}
func arrowPoints(ps []routeCaptionPoint) []any {out:=[]any{};for _,p:=range ps {out=append(out,arrowPoint(p))};return out}
func arrowState(b *meshBuilder) any {
 owners:=map[string]any{};for id,o:=range b.arrowOwners {obstacles:=[]any{};keys:=[]float64{};for k:=range o.obstacles {keys=append(keys,k)};sort.Float64s(keys);for _,k:=range keys {polys:=[]any{};for _,p:=range o.obstacles[k] {polys=append(polys,map[string]any{"points":arrowPoints(p.points),"lo":arrowPoint(p.lo),"hi":arrowPoint(p.hi)})};obstacles=append(obstacles,map[string]any{"key":k,"polygons":polys})};owners[id]=map[string]any{"first":o.first,"last":o.last,"node":o.node,"board":o.board,"caps":arrowTriangles(o.caps),"prepared":o.prepared,"obstacles":obstacles}}
 markers:=[]any{};keys:=[]nativeArrowMarkerKey{};for k:=range b.arrowMarkers {keys=append(keys,k)};sort.Slice(keys,func(i,j int)bool{if keys[i].kind!=keys[j].kind{return keys[i].kind<keys[j].kind};return keys[i].width<keys[j].width});for _,k:=range keys {m:=b.arrowMarkers[k];vertices:=m.vertices;if vertices==nil {vertices=[]Vec{}};markers=append(markers,map[string]any{"kind":k.kind,"width":k.width,"vertices":vertices,"minY":arrowNumber(m.minY),"reach":m.reach})}
 canceled:=b.ctx!=nil&&b.ctx.Err()!=nil
 return map[string]any{"scale":b.scale,"background":b.arrowBackground,"triangles":arrowTriangles(b.triangles),"owners":owners,"markers":markers,"work":b.arrowWork,"cachePoints":b.arrowCachePoints,"error":arrowError(b),"canceled":canceled}
}
func arrowCounters(b *meshBuilder) any {owners:=map[string]any{};for id,o:=range b.arrowOwners {sizes:=[]any{};keys:=[]float64{};for k:=range o.obstacles {keys=append(keys,k)};sort.Float64s(keys);for _,k:=range keys {sizes=append(sizes,[]any{k,len(o.obstacles[k])})};owners[id]=map[string]any{"prepared":o.prepared,"caps":len(o.caps),"obstacles":sizes}};return map[string]any{"work":b.arrowWork,"cachePoints":b.arrowCachePoints,"markers":len(b.arrowMarkers),"owners":owners,"error":arrowError(b)}}
func d2rsArrowRoute(b *meshBuilder,e d2isometric.Edge,p []Vec) []Vec {req:=map[string]any{"op":"arrow","builder":arrowState(b),"edge":e,"points":p};v:=arrowTestRoute(b,e,p);arrowRecord(req,map[string]any{"route":v,"state":arrowCounters(b),"source":p});return v}
func d2rsArrowOccluder(t Triangle,y float64) []routeCaptionPoint {v:=nativeArrowOccluder(t,y);out:=v;if out==nil{out=[]routeCaptionPoint{}};arrowRecord(map[string]any{"op":"occluder","triangle":arrowTriangles([]Triangle{t})[0],"minimumY":y},map[string]any{"points":arrowPoints(out)});return v}
