// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): sparse/dense/partial-alpha scanline frames.
// Harvested from the v0.9.0-era probe .reference/raster-touched-20261016/probe-paint_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package scanline
import("context";"encoding/json";"os";"image";"image/color";"testing")
func TestD2RSScanlineTouchedPaint(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 type Record struct{Width int `json:"width"`;Contours [][][2]float64 `json:"contours"`;Color [4]uint8 `json:"color"`;Background [4]uint8 `json:"background"`;Pixels []byte `json:"pixels"`}
 records:=[]Record{}
 for _,width:=range []int{0,1,8,64,513}{
  w:=float64(width)
  shapes:=[][][][2]float64{
   {{{1.25,.25},{w-1.5,.25},{w-1.5,7.75},{1.25,7.75}}},
   {{{-5,-.5},{w+3,1.25},{w-1,7.5},{-.5,6.75}}},
   {{{0,.5},{w,.5},{w,7.5},{0,7.5}},{{2,2},{2,6},{w-2,6},{w-2,2}}},
   {{{0,0},{w/4,1},{w/2,0},{w,2},{w/2,3},{w,4},{w/2,5},{w,6},{w/2,8},{0,8},{w/4,6},{0,4}}},
  }
  for _,shape:=range shapes{for _,paint:=range [][4]uint8{{192,32,224,255},{47,192,65,97},{255,0,0,0}}{for _,bg:=range [][4]uint8{{0,0,0,0},{20,40,60,100}}{
   r:=NewRasterizer(width,8)
   for _,points:=range shape{r.MoveTo64(points[0][0],points[0][1]);for _,p:=range points[1:]{r.LineTo64(p[0],p[1])};r.ClosePath()}
   dst:=image.NewRGBA(image.Rect(0,0,width,8));for i:=0;i<len(dst.Pix);i+=4{copy(dst.Pix[i:i+4],bg[:])}
   budget:=NewWorkBudget(1<<60);if err:=r.DrawRGBA(context.Background(),&budget,dst,color.NRGBA{R:paint[0],G:paint[1],B:paint[2],A:paint[3]});err!=nil{t.Fatal(err)}
   records=append(records,Record{width,shape,paint,bg,dst.Pix})
  }}}
 }
 data,err:=json.Marshal(records);if err!=nil{t.Fatal(err)};if err:=os.WriteFile(out,append(data,'\n'),0o644);err!=nil{t.Fatal(err)}
}
