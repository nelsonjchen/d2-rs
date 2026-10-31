// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): scanline accumulation intervals, bits and sentinel cleanup.
// Harvested from the v0.9.0-era probe .reference/raster-touched-20261016/probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package scanline

import (
 "context"
 "encoding/json"
 "os"
 "math"
 "testing"
)

type d2rsTouchedState struct {
 Min int `json:"min"`
 Max int `json:"max"`
 Touched bool `json:"touched"`
 Partial []uint32 `json:"partial"`
 Difference []uint32 `json:"difference"`
}
func d2rsTouchedSnapshot(r *Rasterizer) d2rsTouchedState {
 p:=make([]uint32,len(r.partial));d:=make([]uint32,len(r.difference))
 for i,x:=range r.partial {p[i]=math.Float32bits(x)}
 for i,x:=range r.difference {d[i]=math.Float32bits(x)}
 return d2rsTouchedState{r.touchedMin,r.touchedMax,r.touched,p,d}
}
func TestD2RSScanlineTouched(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 type Step struct {Row int `json:"row"`; Before d2rsTouchedState `json:"before"`; After d2rsTouchedState `json:"after"`; X uint64 `json:"x"`; Y uint64 `json:"y"`}
 type Record struct {Width int `json:"width"`; X float64 `json:"x"`;Y float64 `json:"y"`;Slope float64 `json:"slope"`;Winding int8 `json:"winding"`;Steps []Step `json:"steps"`}
 records:=[]Record{}
 for _,width:=range []int{0,1,8,17} {for _,x:=range []float64{-20,-0.5,0,0.25,3,7.75,8,16.5,17,25} {for _,slope:=range []float64{0,-0.25,0.25,-1,1,-12,12} {for _,y:=range []float64{0,0.25,0.999} {for _,wind:=range []int8{-1,1} {
  r:=NewRasterizer(width,2);inv:=0.0;if slope!=0 {inv=1/slope}
  e:=scanEdge{x:x,y:y,bottom:1.75,slope:slope,inverseSlope:inv,winding:wind,end:2}
  rec:=Record{Width:width,X:x,Y:y,Slope:slope,Winding:wind,Steps:[]Step{}}
  for row:=0;row<2;row++ {
   if err:=r.accumulateEdgeRow(context.Background(),&e,row);err!=nil {t.Fatal(err)}
   step:=Step{Row:row,Before:d2rsTouchedSnapshot(r),X:math.Float64bits(e.x),Y:math.Float64bits(e.y)}
   r.clearTouched();step.After=d2rsTouchedSnapshot(r);rec.Steps=append(rec.Steps,step)
  }
  records=append(records,rec)
 }}}}}
 data,err:=json.Marshal(records);if err!=nil {t.Fatal(err)};if err:=os.WriteFile(out,append(data,'\n'),0o644);err!=nil{t.Fatal(err)}
}
