// SPDX-License-Identifier: MPL-2.0
// Original natural Markdown layout API; virtual trace-worktree test overlay.
package textmeasure
import("encoding/json";"os";"testing";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestCodexNaturalMarkdown(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_NATURAL_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;FontSize int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 ruler,err:=NewRuler();if err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,request:=range requests{
  primary:=d2fonts.SourceSansPro;if request.Family=="mono"{primary=d2fonts.SourceCodePro};if request.Family=="hand"{primary=d2fonts.HandDrawn};mono:=d2fonts.SourceCodePro
  layout,err:=LayoutMarkdown(request.Markdown,ruler,&primary,&mono,request.FontSize);if err!=nil{t.Fatal(err)}
  html,err:=RenderMarkdown(request.Markdown);if err!=nil{t.Fatal(err)}
  rows=append(rows,map[string]any{"request":request,"rendered_html":html,"layout":layout,"svg":layout.SVG(MarkdownSVGOptions{})})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_NATURAL_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
