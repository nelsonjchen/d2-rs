// SPDX-License-Identifier: MPL-2.0
package main
import("encoding/json";"os";"strings";"github.com/d2lang/d2/d2compiler")
type Input struct{Name string `json:"name"`; Source string `json:"source"`}
func main(){var inputs []Input;if err:=json.NewDecoder(os.Stdin).Decode(&inputs);err!=nil{panic(err)};rows:=[]any{};for _,in:=range inputs{func(){defer func(){if p:=recover();p!=nil{rows=append(rows,map[string]any{"input":in,"panic":p})}}();g,_,err:=d2compiler.Compile("-",strings.NewReader(in.Source),nil);if err!=nil{rows=append(rows,map[string]any{"input":in,"error":err.Error()});return};edges:=[]any{};for _,e:=range g.Edges{edges=append(edges,map[string]any{"src":e.Src.AbsID(),"dst":e.Dst.AbsID(),"src_arrow":e.SrcArrow,"dst_arrow":e.DstArrow,"index":e.Index,"abs_id":e.AbsID()})};rows=append(rows,map[string]any{"input":in,"error":"","edges":edges})}()};enc:=json.NewEncoder(os.Stdout);enc.SetIndent("","  ");if err:=enc.Encode(rows);err!=nil{panic(err)}}
