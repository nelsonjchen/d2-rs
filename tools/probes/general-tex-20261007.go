// SPDX-License-Identifier: MPL-2.0
package d2latex
import("encoding/json";"os";"testing")
func TestCodexGeneralTex(t *testing.T){data,e:=os.ReadFile(os.Getenv("CODEX_TEX_INPUT"));if e!=nil{t.Fatal(e)};var requests []struct{Expression string `json:"expression"`};if e=json.Unmarshal(data,&requests);e!=nil{t.Fatal(e)};rows:=[]map[string]any{};for _,r:=range requests {svg,e:=Render(r.Expression);if e!=nil{t.Fatal(e)};w,h,e:=Measure(r.Expression);if e!=nil{t.Fatal(e)};rows=append(rows,map[string]any{"expression":r.Expression,"svg":svg,"width":w,"height":h})};out,e:=json.MarshalIndent(rows,"","  ");if e!=nil{t.Fatal(e)};if e=os.WriteFile(os.Getenv("CODEX_TEX_OUTPUT"),append(out,'\n'),0644);e!=nil{t.Fatal(e)}}
