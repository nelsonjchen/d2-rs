// SPDX-License-Identifier: MPL-2.0
// Original pinned uniseg API controls in a virtual D2 trace-worktree overlay.
package d2scenebuild

import (
 "encoding/json"
 "os"
 "testing"
 "github.com/rivo/uniseg"
)
func TestCodexUnisegBoundaries(t *testing.T) {
 data,err:=os.ReadFile(os.Getenv("CODEX_UNISEG_INPUT"));if err!=nil{t.Fatal(err)}
 var requests []string;if err=json.Unmarshal(data,&requests);err!=nil{t.Fatal(err)}
 rows:=[]map[string]any{}
 for _,input:=range requests {
  segments:=[]map[string]any{};line:=[][2]any{};remaining:=input;state:=-1;offset:=0
  for remaining!="" {
   segment,rest,must,next:=uniseg.FirstLineSegmentInString(remaining,state)
   offset+=len(segment);line=append(line,[2]any{offset,must});segments=append(segments,map[string]any{"segment":segment,"rest":rest,"must_break":must,"new_state":next})
   remaining=rest;state=next
  }
  graphemes:=uniseg.NewGraphemes(input);ends:=[]int{}
  for graphemes.Next(){_,end:=graphemes.Positions();ends=append(ends,end)}
  rows=append(rows,map[string]any{"string_width":uniseg.StringWidth(input),"input":input,"line_boundaries":line,"grapheme_boundaries":ends,"source_segments":segments})
 }
 data,err=json.Marshal(rows);if err!=nil{t.Fatal(err)}
 if err=os.WriteFile(os.Getenv("CODEX_UNISEG_OUTPUT"),append(data,'\n'),0644);err!=nil{t.Fatal(err)}
}
