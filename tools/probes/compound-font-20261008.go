// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing")
func TestCodexCompoundFont(t *testing.T){
 data,e:=os.ReadFile(os.Getenv("CODEX_COMPOUND_INPUT"));if e!=nil{t.Fatal(e)}
 var rows []map[string]any;if e=json.Unmarshal(data,&rows);e!=nil{t.Fatal(e)}
 for _,r:=range rows {expression:=r["expression"].(string);svg,e:=Render(expression);if e!=nil{r["render_error"]=e.Error()};w,h,e:=Measure(expression);if e!=nil{r["measure_error"]=e.Error()};r["svg"]=svg;r["width"]=w;r["height"]=h}
 out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)}
 if e=os.WriteFile(os.Getenv("CODEX_COMPOUND_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}
}
