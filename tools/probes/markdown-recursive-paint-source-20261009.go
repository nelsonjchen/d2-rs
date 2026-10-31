// SPDX-License-Identifier: MPL-2.0
// Original natural Markdown layout API; virtual trace-worktree test overlay.
package textmeasure
import("golang.org/x/net/html";"strings";"github.com/PuerkitoBio/goquery";"encoding/json";"os";"testing";"github.com/d2lang/d2/d2renderers/d2fonts")
func TestCodexRecursivePaint(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_NATURAL_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;FontSize int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 ruler,err:=NewRuler();if err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,request:=range requests{
  primary:=d2fonts.SourceSansPro;if request.Family=="mono"{primary=d2fonts.SourceCodePro};if request.Family=="hand"{primary=d2fonts.HandDrawn};mono:=d2fonts.SourceCodePro
  layout,err:=LayoutMarkdown(request.Markdown,ruler,&primary,&mono,request.FontSize);if err!=nil{rows=append(rows,map[string]any{"request":request,"error":err.Error()});continue}
  html,err:=RenderMarkdown(request.Markdown);if err!=nil{t.Fatal(err)}
  doc,err:=goquery.NewDocumentFromReader(strings.NewReader(html));if err!=nil{t.Fatal(err)}
  ruler.boundsWithDot=true;ruler.LineHeightFactor=MarkdownLineHeight;cells:=[]map[string]any{}
  for _,n:=range doc.Find("td,th").Nodes {style:=d2fonts.FONT_STYLE_REGULAR;header:=n.Data=="th"||hasAncestorElement(n,"thead");if header{style=d2fonts.FONT_STYLE_SEMIBOLD};attrs:=ruler.measureNode(0,n,&primary,&mono,request.FontSize,style,header,false);children:=[]map[string]any{};for child:=n.FirstChild;child!=nil;child=child.NextSibling{ca:=ruler.measureNode(0,child,&primary,&mono,request.FontSize,style,header,false);children=append(children,map[string]any{"data":child.Data,"width":ca.width,"height":ca.height})};cells=append(cells,map[string]any{"tag":n.Data,"width":attrs.width,"height":attrs.height,"children":children})}
  ruler.boundsWithDot=false;ruler.LineHeightFactor=1
  rows=append(rows,map[string]any{"request":request,"rendered_html":html,"layout":layout,"svg":layout.SVG(MarkdownSVGOptions{RolePaint:map[MarkdownColorRole]MarkdownSVGPaint{MarkdownColorForeground:{Color:"#0A0F25"},MarkdownColorAccent:{Color:"#0D32B2"},MarkdownColorNeutralMuted:{Color:"#EEF1F8"},MarkdownColorCanvas:{Color:"#FFFFFF"},MarkdownColorCanvasSubtle:{Color:"#EEF1F8"},MarkdownColorBorder:{Color:"#0D32B2"},MarkdownColorBorderMuted:{Color:"#0D32B2"},MarkdownColorMuted:{Color:"#6F7E8B"}}}),"legacy_cells":cells})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_NATURAL_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
func TestCodexRecursiveCSS(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_NATURAL_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;FontSize int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 ruler,err:=NewRuler();if err!=nil{t.Fatal(err)};ruler.boundsWithDot=true;rows:=[]map[string]any{}
 for _,request:=range requests{
  primary:=d2fonts.SourceSansPro;if request.Family=="mono"{primary=d2fonts.SourceCodePro};if request.Family=="hand"{primary=d2fonts.HandDrawn};mono:=d2fonts.SourceCodePro
  rendered,err:=RenderMarkdown(request.Markdown);if err!=nil{t.Fatal(err)};doc,err:=goquery.NewDocumentFromReader(strings.NewReader(rendered));if err!=nil{t.Fatal(err)};body:=doc.Find("body").Nodes[0];normalizeMarkdownWhitespaceTree(body)
  ctx:=markdownPaintContext{fontFamily:&primary,monoFontFamily:&mono,fontSize:request.FontSize,fontScale:1,fontStyle:d2fonts.FONT_STYLE_REGULAR,lineHeightFactor:MarkdownLineHeight,mono:primary==mono,role:MarkdownColorForeground};p:=markdownPainter{ruler:ruler,layout:&MarkdownLayout{}};nodes:=[]map[string]any{}
  var walk func(*html.Node,markdownPaintContext);walk=func(n *html.Node,ctx markdownPaintContext){a:=p.measure(n,ctx);nodes=append(nodes,map[string]any{"kind":n.Type,"data":n.Data,"width":a.width,"height":a.height,"top":a.marginTop,"bottom":a.marginBottom});next:=p.childContext(n,ctx);for c:=n.FirstChild;c!=nil;c=c.NextSibling{walk(c,next)}};walk(body,ctx);rows=append(rows,map[string]any{"request":request,"nodes":nodes})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_NATURAL_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
