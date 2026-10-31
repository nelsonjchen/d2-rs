// SPDX-License-Identifier: MPL-2.0
// Actual pinned scene construction through a virtual trace-worktree test.
package d2scenebuild

import (
 "context"
 "encoding/json"
 "os"
 "strings"
 "testing"
 "github.com/d2lang/d2/d2renderers/d2fonts"
 "github.com/d2lang/d2/d2renderers/d2scene"
 "github.com/d2lang/d2/d2target"
 "github.com/d2lang/d2/lib/textmeasure"
)

func TestCodexMarkdownSceneLinks(t *testing.T) {
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_SCENE_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;FontSize int;Width,Height int;Appendix,DisableLinks bool;MaxRegions,MaxStringBytes int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 ruler,err:=textmeasure.NewRuler();if err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,r:=range requests {
  primary:=d2fonts.SourceSansPro;if r.Family=="mono"{primary=d2fonts.SourceCodePro};if r.Family=="hand"{primary=d2fonts.HandDrawn};mono:=d2fonts.SourceCodePro
  layout,layoutErr:=textmeasure.LayoutMarkdown(r.Markdown,ruler,&primary,&mono,r.FontSize)
  row:=map[string]any{"request":r};if layoutErr!=nil{row["layout_error"]=layoutErr.Error();rows=append(rows,row);continue}
  row["layout"]=layout;row["svg"]=layout.SVG(textmeasure.MarkdownSVGOptions{RolePaint:map[textmeasure.MarkdownColorRole]textmeasure.MarkdownSVGPaint{textmeasure.MarkdownColorForeground:{Color:"#0A0F25"},textmeasure.MarkdownColorAccent:{Color:"#0D32B2"},textmeasure.MarkdownColorNeutralMuted:{Color:"#EEF1F8"}}})
  w,h:=r.Width,r.Height;if w<0{w=layout.Width};if h<0{h=layout.Height}
  diagram:=d2target.NewDiagram();diagram.FontFamily=&primary;font:="default";if r.Family=="mono"{font="mono"};if r.Family=="hand"{font="hand"}
  shape:=d2target.Shape{ID:"markdown",Type:d2target.ShapeText,Pos:d2target.Point{X:0,Y:0},Width:w,Height:h,Fill:"N7",Stroke:"N1",StrokeWidth:0,Opacity:1,LabelPosition:"INSIDE_TOP_LEFT",Text:d2target.Text{Label:r.Markdown,Language:"markdown",FontFamily:font,FontSize:r.FontSize,LabelWidth:w,LabelHeight:h}}
  if r.DisableLinks{shape.Link="https://outer.example.com";shape.PrettyLink="outer"}
  diagram.Shapes=[]d2target.Shape{shape}
  document,buildErr:=Build(context.Background(),diagram,Options{Appendix:r.Appendix,LinkBudget:LinkBudget{MaxRegions:r.MaxRegions,MaxStringBytes:r.MaxStringBytes}})
  if buildErr!=nil{row["error"]=buildErr.Error()}else{
   row["links"]=document.Links;row["viewbox"]=document.ViewBox;row["logical_width"]=document.LogicalWidth;row["logical_height"]=document.LogicalHeight
   retained:=[]map[string]any{};var visit func(*d2scene.Node);visit=func(n *d2scene.Node){if n==nil{return};if strings.Contains(n.ID,":markdown")||strings.HasPrefix(n.ID,"appendix:"){retained=append(retained,map[string]any{"id":n.ID,"transform":n.Transform,"opacity":n.Opacity,"primitive":n.Primitive,"clip":n.Clip})};for _,child:=range n.Children{visit(child)}};visit(document.Root);row["retained"]=retained
  }
  rows=append(rows,row)
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_SCENE_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
