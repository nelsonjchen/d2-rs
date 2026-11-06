// SPDX-License-Identifier: MPL-2.0
// Diagnostic overlay for the pinned Go D2 v0.9.0 renderer.
package d2raster

import (
 "context"
 "encoding/json"
 "image/color"
 "os"
 "path/filepath"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
)

func TestOpaqueArrowMaskLayerWitness(t *testing.T) {
 output := os.Getenv("D2_ARROW_MASK_OUTPUT")
 if output == "" { t.Fatal("D2_ARROW_MASK_OUTPUT is required") }
 reports := []map[string]any{}
 for _, masked := range []bool{false, true} {
  paint := d2scene.SolidPaint{Color: color.NRGBA{R:13, G:50, B:178, A:255}}
  group := d2scene.NewNode(nil)
  group.Children = []*d2scene.Node{
   d2scene.NewNode(d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(20,10),d2scene.LineTo(20,54)}, Stroke: &d2scene.Stroke{Paint:paint,Width:2,Cap:d2scene.CapRound,Join:d2scene.JoinRound,MiterLimit:4}}),
   d2scene.NewNode(d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(14,47),d2scene.LineTo(20,57),d2scene.LineTo(26,47),d2scene.ClosePath()}, Fill:paint}),
  }
  name := "direct"
  if masked {
   name = "masked"
   mask := d2scene.NewNode(d2scene.Rect{Box:d2scene.Box{Width:80,Height:80},Fill:d2scene.SolidPaint{Color:color.NRGBA{R:255,G:255,B:255,A:255}}})
   group.Mask = &d2scene.Mask{Root:mask,Type:d2scene.MaskLuminance,Transform:d2scene.Identity()}
  }
  document := d2scene.NewDocument(d2scene.Box{Width:80,Height:80},group)
  options := testOptions();options.Scale=2;options.Background=color.White
  frame,err := Render(context.Background(),document,options);if err!=nil {t.Fatal(err)}
  png,err := EncodePNG(context.Background(),frame);if err!=nil {t.Fatal(err)}
  if err=os.WriteFile(filepath.Join(output,"go-"+name+".png"),png,0644);err!=nil {t.Fatal(err)}
  if err=os.WriteFile(filepath.Join(output,"go-"+name+".rgba"),frame.Pix,0644);err!=nil {t.Fatal(err)}
  reports=append(reports,map[string]any{"masked":masked,"width":frame.Bounds().Dx(),"height":frame.Bounds().Dy(),"cap_pixel":frame.NRGBAAt(38,18)})
 }
 encoded,err:=json.MarshalIndent(reports,"","  ");if err!=nil {t.Fatal(err)}
 if err=os.WriteFile(filepath.Join(output,"go-mask-witness.json"),append(encoded,'\n'),0644);err!=nil {t.Fatal(err)}
}
