// SPDX-License-Identifier: MPL-2.0
package textmeasure
import("encoding/json";"os";"testing")
func TestCodexMarkdownWriter(t *testing.T){
 data,err:=os.ReadFile(os.Getenv("CODEX_WRITER_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []struct{Name,Markdown string};if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)};rows:=[]map[string]any{}
 for _,r:=range requests{html,err:=RenderMarkdown(r.Markdown);message:="";if err!=nil{message=err.Error()};rows=append(rows,map[string]any{"request":r,"html":html,"error":message})}
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)};if err=os.WriteFile(os.Getenv("CODEX_WRITER_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
