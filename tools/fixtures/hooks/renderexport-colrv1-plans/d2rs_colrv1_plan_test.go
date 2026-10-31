// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): COLRv1 paint plan or error for every bundled emoji glyph.
// Harvested from the v0.9.0-era probe d2fonts/colrv1_plan_probe_test.go; retargeted to the manifest
// pin. Inert unless run by scripts/regen-fixtures.py.
package d2fonts
import("testing";"os";"encoding/json";"github.com/d2lang/d2/d2renderers/internal/fontface")
func d2rsPaintRecord(p fontface.COLRv1Paint) any {
 switch p:=p.(type) {
 case fontface.COLRv1Layers:
  children:=make([]any,0,len(p.Paints));for _,v:=range p.Paints {children=append(children,d2rsPaintRecord(v))};return map[string]any{"kind":"layers","paints":children}
 case fontface.COLRv1Solid:return map[string]any{"kind":"solid","color":[4]uint8{p.Color.R,p.Color.G,p.Color.B,p.Color.A}}
 case fontface.COLRv1LinearGradient:return map[string]any{"kind":"linear","stops":d2rsStopRecords(p.ColorLine),"points":[6]float64{p.X0,p.Y0,p.X1,p.Y1,p.X2,p.Y2}}
 case fontface.COLRv1RadialGradient:return map[string]any{"kind":"radial","stops":d2rsStopRecords(p.ColorLine),"circles":[6]float64{p.X0,p.Y0,p.Radius0,p.X1,p.Y1,p.Radius1}}
 case fontface.COLRv1Glyph:return map[string]any{"kind":"glyph","glyph_id":p.GlyphID,"paint":d2rsPaintRecord(p.Paint)}
 case fontface.COLRv1Transform:return map[string]any{"kind":"transform","matrix":[6]float64{p.Matrix.Xx,p.Matrix.Yx,p.Matrix.Xy,p.Matrix.Yy,p.Matrix.Dx,p.Matrix.Dy},"paint":d2rsPaintRecord(p.Paint)}
 case fontface.COLRv1Composite:return map[string]any{"kind":"composite","source":d2rsPaintRecord(p.Source),"backdrop":d2rsPaintRecord(p.Backdrop),"mode":p.Mode}
 default:panic("unexpected Source paint type")
 }
}
func d2rsStopRecords(line fontface.COLRv1ColorLine) []any {
 out:=make([]any,0,len(line.Stops));for _,p:=range line.Stops {out=append(out,map[string]any{"offset":p.Offset,"color":[4]uint8{p.Color.R,p.Color.G,p.Color.B,p.Color.A}})};return out
}
func TestD2RSCOLRv1Plans(t *testing.T) {
 out := os.Getenv("D2RS_RECIPE_OUTPUT"); if out == "" { t.Skip("run via scripts/regen-fixtures.py") }

 data,err:=bundledNotoColorEmoji();if err!=nil {t.Fatal(err)};face,err:=fontface.ParseFace(data,0);if err!=nil {t.Fatal(err)}
 rows:=[]any{};n:=face.Outline.NumGlyphs()
 for i:=0;i<=n+1;i++ {plan,found,err:=face.CompileBundledNotoColorEmojiCOLRv1Plan(uint32(i));row:=map[string]any{"glyph_id":i,"found":found,"error":"","plan":nil};if err!=nil {row["error"]=err.Error()};if plan!=nil {var clip any;if plan.Clip!=nil {p:=plan.Clip;clip=[4]float64{p.XMin,p.YMin,p.XMax,p.YMax}};row["plan"]=map[string]any{"glyph_id":plan.GlyphID,"root":d2rsPaintRecord(plan.Root),"clip":clip,"usage":map[string]int{"paint_nodes":plan.Usage.PaintNodes,"max_depth":plan.Usage.MaxDepth,"max_layers":plan.Usage.MaxLayers,"max_gradient_stops":plan.Usage.MaxGradientStops}}};rows=append(rows,row)}
 bytes,err:=json.MarshalIndent(rows,"","  ");if err!=nil {t.Fatal(err)};if err:=os.WriteFile(out,bytes,0644);err!=nil {t.Fatal(err)}
}
