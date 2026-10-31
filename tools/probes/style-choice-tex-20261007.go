// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing")
func TestCodexStyleChoice(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_MHC_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows {expression:=r["expression"].(string);svg,e:=Render(expression);if e!=nil{t.Fatal(e)};w,h,e:=Measure(expression);if e!=nil{t.Fatal(e)};r["svg"]=svg;r["width"]=w;r["height"]=h}
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_MHC_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
