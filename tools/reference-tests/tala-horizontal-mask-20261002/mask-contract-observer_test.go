// Diagnostic observer against pristine D2 v0.9.0; not shipped.
package d2scenebuild

import (
 "context"
 "encoding/json"
 "net/url"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/d2target"
)

func TestCodexMaskPositionContract20261002(t *testing.T) {
 cases := []struct {name, label, icon string; want float64}{
  {"label-outside-top", "OUTSIDE_TOP_CENTER", "", .75},
  {"label-outside-bottom", "OUTSIDE_BOTTOM_CENTER", "", .75},
  {"label-middle-left", "INSIDE_MIDDLE_LEFT", "", 1},
  {"label-middle-center", "INSIDE_MIDDLE_CENTER", "", 1},
  {"label-middle-right", "INSIDE_MIDDLE_RIGHT", "", 1},
  {"label-unlocked-middle", "UNLOCKED_MIDDLE", "", 1},
  {"icon-outside-top", "", "OUTSIDE_TOP_CENTER", .75},
  {"icon-middle", "", "INSIDE_MIDDLE_CENTER", 1},
  {"label-on-icon-off", "INSIDE_MIDDLE_CENTER", "OUTSIDE_TOP_CENTER", 1},
  {"label-off-icon-on", "OUTSIDE_TOP_CENTER", "INSIDE_MIDDLE_CENTER", .75},
 }
 for _, c := range cases { t.Run(c.name, func(t *testing.T) {
  diagram := validDiagram()
  connection := &diagram.Connections[0]
  if c.label != "" { connection.Text = d2target.Text{Label: "label", FontSize:16, LabelWidth:40, LabelHeight:20}; connection.LabelPosition=c.label }
  if c.icon != "" { connection.Icon=&url.URL{Path:"icon.svg"}; connection.IconPosition=c.icon }
  b := builder{ctx:context.Background(), diagram:diagram}
  mask, err := b.buildConnectionMask(d2scene.Box{X:-100,Y:-100,Width:300,Height:300})
  if err != nil { t.Fatal(err) }
  hole := findSceneNode(t,mask.Root,"a-b:label-mask-hole")
  if hole.Opacity != c.want { t.Fatalf("opacity %v want %v",hole.Opacity,c.want) }
  rect := hole.Primitive.(d2scene.Rect)
  data,err := json.Marshal(map[string]any{"name":c.name,"label":c.label,"icon":c.icon,"opacity":hole.Opacity,"hole":rect.Box})
  if err != nil { t.Fatal(err) }; t.Logf("D2_MASK_WITNESS %s",data)
 }) }
}
