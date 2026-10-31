// SPDX-License-Identifier: MPL-2.0
// Source internal paragraph paint controls; virtual trace-worktree overlay.
package textmeasure
import("encoding/json";"os";"strings";"testing";"github.com/PuerkitoBio/goquery";"github.com/d2lang/d2/d2renderers/d2fonts";"golang.org/x/net/html")
func TestCodexMarkdownWrap(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_WRAP_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;Width float64;FontSize int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 ruler,err:=NewRuler();if err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,r:=range requests{
  primary:=d2fonts.SourceSansPro;if r.Family=="mono"{primary=d2fonts.SourceCodePro};if r.Family=="hand"{primary=d2fonts.HandDrawn};mono:=d2fonts.SourceCodePro
  rendered,err:=RenderMarkdown(r.Markdown);if err!=nil{t.Fatal(err)};doc,err:=goquery.NewDocumentFromReader(strings.NewReader(rendered));if err!=nil{t.Fatal(err)};body:=doc.Find("body").First().Nodes[0];normalizeMarkdownWhitespaceTree(body)
  var paragraph *html.Node;for n:=body.FirstChild;n!=nil;n=n.NextSibling{if n.Type==html.ElementNode&&n.Data=="p"{if paragraph!=nil{t.Fatal("multiple paragraphs")};paragraph=n}}
  if paragraph==nil{t.Fatal("missing paragraph")};nodes:=[]*html.Node{};for n:=paragraph.FirstChild;n!=nil;n=n.NextSibling{nodes=append(nodes,n)}
  ctx:=markdownPaintContext{fontFamily:&primary,monoFontFamily:&mono,fontSize:r.FontSize,fontScale:1,fontStyle:d2fonts.FONT_STYLE_REGULAR,lineHeightFactor:MarkdownLineHeight,mono:primary==mono,role:MarkdownColorForeground}
  painter:=markdownPainter{ruler:ruler,layout:&MarkdownLayout{}}
  height:=painter.paintInlineNodes(nodes,0,0,ctx,r.Width)
  rows=append(rows,map[string]any{"request":r,"rendered_html":rendered,"painted_height":height,"primitives":painter.layout.Primitives,"svg":painter.layout.SVG(MarkdownSVGOptions{RolePaint:map[MarkdownColorRole]MarkdownSVGPaint{MarkdownColorForeground:{Color:"#0A0F25"},MarkdownColorAccent:{Color:"#0D32B2"},MarkdownColorNeutralMuted:{Color:"#EEF1F8"}}})})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_WRAP_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
