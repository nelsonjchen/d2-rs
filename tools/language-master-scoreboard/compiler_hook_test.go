// SPDX-License-Identifier: MPL-2.0
// Installed only in the pinned trace checkout. Original assertions still run.
package d2compiler_test

import (
 "bytes"
 "crypto/sha256"
 "encoding/json"
 "fmt"
 "io"
 "io/fs"
 "os"
 "path/filepath"
 "sync"
 "testing"

 "github.com/d2lang/d2/d2compiler"
 "github.com/d2lang/d2/d2graph"
 "github.com/d2lang/d2/d2target"
)

var d2rsMasterMu sync.Mutex
var d2rsMasterCalls = map[string]int{}

func d2rsMasterCompile(t *testing.T, path string, reader io.Reader, opts *d2compiler.CompileOptions) (*d2graph.Graph, *d2target.Config, error) {
 if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" { return d2compiler.Compile(path, reader, opts) }
 input, readErr := io.ReadAll(reader)
 if readErr != nil { t.Fatal(readErr) }
 if opts == nil { opts = &d2compiler.CompileOptions{} }
 files := map[string]string{}
 if opts.FS != nil {
  err := fs.WalkDir(opts.FS, ".", func(name string, entry fs.DirEntry, err error) error {
   if err != nil { return err }; if entry.IsDir() { return nil }
   data, err := fs.ReadFile(opts.FS, name); if err != nil { return err }; files[name] = string(data); return nil
  }); if err != nil { t.Fatal(err) }
 }
 graph, config, err := d2compiler.Compile(path, bytes.NewReader(input), opts)
 errText := ""; if err != nil { errText = err.Error() }
 // The full compiler graph is the same contract used by the golden scoreboard.
 request := map[string]interface{}{"input":string(input), "path":path, "files":files, "has_fs":opts.FS != nil,
  "max_variable_expansion":opts.MaxVariableExpansion, "max_glob_expansion":opts.MaxGlobExpansion,
  "max_edge_expansion":opts.MaxEdgeExpansion, "max_edge_expansion_work":opts.MaxEdgeExpansionWork,
  "has_context":opts.Context != nil}
 d2rsMasterMu.Lock(); defer d2rsMasterMu.Unlock()
 call := d2rsMasterCalls[t.Name()]; d2rsMasterCalls[t.Name()]++
 name := fmt.Sprintf("%s#%d", t.Name(), call)
 row := map[string]interface{}{"name":name, "test":t.Name(), "request":request,
  "got":map[string]interface{}{"error":errText, "graph":graph, "config":config}}
 data, marshalErr := json.Marshal(row); if marshalErr != nil { t.Fatal(marshalErr) }
 id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
 if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"),id+".json"),data,0644); err != nil { t.Fatal(err) }
 return graph, config, err
}
