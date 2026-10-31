// SPDX-License-Identifier: MPL-2.0 AND MIT
// Actual pinned Goldmark document block state reached by original D2 v0.9.0.
package textmeasure
import("encoding/json";"os";"testing";"github.com/yuin/goldmark/ast";gmtext "github.com/yuin/goldmark/text")
func TestCodexListBlockState(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MARKDOWN_NATURAL_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown,Family string;FontSize int};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 rows:=[]map[string]any{}
 for _,r:=range requests{
  doc:=markdownRenderer.Parser().Parse(gmtext.NewReader([]byte(r.Markdown)));nodes:=[]map[string]any{};ids:=map[ast.Node]int{}
  err=ast.Walk(doc,func(n ast.Node,entering bool)(ast.WalkStatus,error){if !entering{return ast.WalkContinue,nil};id:=len(ids);ids[n]=id;parent:=-1;if n.Parent()!=nil{parent=ids[n.Parent()]};if n.Type()==ast.TypeBlock{v:=map[string]any{"id":id,"parent":parent,"kind":n.Kind().String(),"blank_previous":n.HasBlankPreviousLines(),"position":n.Pos(),"children":n.ChildCount()};if l,ok:=n.(*ast.List);ok{v["tight"]=l.IsTight};if item,ok:=n.(*ast.ListItem);ok{v["offset"]=item.Offset};nodes=append(nodes,v)};return ast.WalkContinue,nil});if err!=nil{t.Fatal(err)}
  rows=append(rows,map[string]any{"request":r,"blocks":nodes})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MARKDOWN_NATURAL_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
