// SPDX-License-Identifier: MPL-2.0
// Actual original addMarkdownLinkRegion transaction and budget admission.
package d2scenebuild
import("context";"encoding/json";"os";"testing";"github.com/d2lang/d2/d2renderers/d2scene")
func TestCodexMarkdownLinkTransactions(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_MD_TRANSACTION_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,URL,Title string;MaxRegions,MaxBytes,Seed int;Appendix bool;Box d2scene.Box};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 rows:=[]map[string]any{}
 for _,r:=range requests {
  b:=builder{ctx:context.Background(),options:Options{Appendix:r.Appendix,LinkBudget:LinkBudget{MaxRegions:r.MaxRegions,MaxStringBytes:r.MaxBytes}}}
  for i:=0;i<r.Seed;i++{b.links=append(b.links,d2scene.LinkRegion{Box:d2scene.Box{Width:1,Height:1},URL:"s"});b.linkBytes++;b.appendixItems=append(b.appendixItems,appendixItem{object:"seed",field:"tooltip",text:"seed"});b.appendixStringBytes+=4}
  err:=b.addMarkdownLinkRegion("node",3,r.URL,r.Title,r.Box);titles:=[]string{};for _,item:=range b.appendixItems{titles=append(titles,item.text)};errorText:="";if err!=nil{errorText=err.Error()}
  rows=append(rows,map[string]any{"request":r,"error":errorText,"links":b.links,"link_bytes":b.linkBytes,"appendix":titles,"appendix_bytes":b.appendixStringBytes})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_MD_TRANSACTION_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
