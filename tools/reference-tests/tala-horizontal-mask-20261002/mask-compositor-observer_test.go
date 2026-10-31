// Diagnostic observer against pristine D2 v0.9.0; not shipped.
package d2raster

import (
 "context"
 "encoding/json"
 "image"
 "image/color"
 "image/draw"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/d2renderers/d2raster/internal/scanline"
)

func TestCodexJupyterHorizontalMaskPixel20261002(t *testing.T) {
 ctx:=context.Background()
 // Exact first destination triangle from the captured public route/marker.
 matrix:=d2scene.Translate(452,235).Mul(d2scene.Rotate(0)).Mul(d2scene.Translate(-8.5,-8))
 device:=d2scene.Matrix{A:2,D:2,E:204,F:204}.Mul(matrix)
 vertices:=[]d2scene.Point{{},{X:13,Y:8},{Y:16}}
 for index,vertex:=range vertices { vertices[index]=device.Point(vertex) }
 wantVertices:=[]d2scene.Point{{X:1091,Y:658},{X:1117,Y:674},{X:1091,Y:690}}
 for index:=range vertices { if vertices[index]!=wantVertices[index] { t.Fatalf("vertex %d %v",index,vertices[index]) } }
 bounds:=image.Rect(1090,657,1120,692)
 source:=image.NewRGBA(bounds)
 r:=scanline.NewRasterizer(bounds.Dx(),bounds.Dy())
 // The captured production reference origin is (560,657). The cropped
 // destination changes only the post-float32 origin offset.
 r.SetOriginOffset(560-float64(bounds.Min.X),657-float64(bounds.Min.Y))
 r.MoveTo64(vertices[0].X-560,vertices[0].Y-657)
 r.LineTo64(vertices[1].X-560,vertices[1].Y-657)
 r.LineTo64(vertices[2].X-560,vertices[2].Y-657)
 r.ClosePath()
 budget:=scanline.NewWorkBudget(1_000_000)
 if err:=r.DrawRGBA(ctx,&budget,source,color.NRGBA{R:46,G:125,B:50,A:255});err!=nil {t.Fatal(err)}
 raw:=source.RGBAAt(1091,658)
 if raw!=(color.RGBA{R:32,G:87,B:34,A:177}) {t.Fatalf("source %v",raw)}
 for _, opacity:=range []float64{.75,1} {t.Run(map[float64]string{.75:"partial",1:"opaque"}[opacity],func(t *testing.T) {
  layer:=image.NewRGBA(bounds);copy(layer.Pix,source.Pix)
  mask:=image.NewRGBA(bounds);draw.Draw(mask,bounds,image.NewUniform(color.RGBA{R:255,G:255,B:255,A:255}),image.Point{},draw.Src)
  black:=image.NewRGBA(bounds);draw.Draw(black,bounds,image.NewUniform(color.RGBA{A:255}),image.Point{},draw.Src)
  if err:=compositeLayerOver(ctx,mask,black,opacity);err!=nil {t.Fatal(err)}
  maskPixel:=mask.RGBAAt(1091,658)
  if err:=multiplyLayerByRGBALuminance(ctx,layer,mask);err!=nil {t.Fatal(err)}
  masked:=layer.RGBAAt(1091,658)
  dst:=image.NewRGBA(bounds);draw.Draw(dst,bounds,image.NewUniform(color.RGBA{R:227,G:242,B:253,A:255}),image.Point{},draw.Src)
  if err:=compositeLayerOver(ctx,dst,layer,1);err!=nil {t.Fatal(err)}
  final:=dst.RGBAAt(1091,658)
  want:=color.RGBA{R:196,G:222,B:218,A:255};if opacity==1 {want=color.RGBA{R:227,G:242,B:253,A:255}}
  if final!=want {t.Fatalf("final %v want %v",final,want)}
  data,err:=json.Marshal(map[string]any{"opacity":opacity,"source_matrix":matrix,"device_matrix":device,"device_vertices":vertices,"reference_origin":[]int{560,657},"pixel":[]int{1091,658},"source_premultiplied":raw,"mask_rgba":maskPixel,"masked_premultiplied":masked,"final_rgba":final})
  if err!=nil {t.Fatal(err)};t.Logf("D2_MASK_PIXEL_WITNESS %s",data)
 })}
}
