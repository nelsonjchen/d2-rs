// SPDX-License-Identifier: MPL-2.0
package d2ir

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2format"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

var d2rsSmallMu sync.Mutex
var d2rsSmallCalls = map[string]int{}

func d2rsSmallWrite(t *testing.T, request, got interface{}) {
	t.Helper()
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return
	}
	d2rsSmallMu.Lock()
	defer d2rsSmallMu.Unlock()
	call := d2rsSmallCalls[t.Name()]
	d2rsSmallCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request, "got": got})
	if err != nil {
		t.Fatal(err)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
func d2rsSmallEnsure(t *testing.T, m *Map, kp *d2ast.KeyPath, refctx *RefContext, create bool, c *compiler) ([]*Field, error) {
	fields, err := m.EnsureField(kp, refctx, create, c)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsSmallWrite(t, map[string]interface{}{"op": "ensure", "key": d2format.Format(kp), "create": create}, map[string]interface{}{"fields": fields, "error": message})
	return fields, err
}
func d2rsSmallContains(t *testing.T, root Composite, target Node) bool {
	got := compositeContainsNode(root, target)
	ids := map[Node]int{}
	nodes := []map[string]interface{}{}
	var walk func(Node) int
	walk = func(n Node) int {
		if i, ok := ids[n]; ok {
			return i
		}
		i := len(nodes)
		ids[n] = i
		nodes = append(nodes, nil)
		row := map[string]interface{}{}
		switch n := n.(type) {
		case *Map:
			row["kind"] = "map"
			children := []int{}
			for _, f := range n.Fields {
				children = append(children, walk(f))
			}
			row["children"] = children
		case *Field:
			row["kind"] = "field"
			if n.Composite != nil {
				row["composite"] = walk(n.Composite)
			} else {
				row["composite"] = nil
			}
		default:
			t.Fatalf("unsupported original node %T", n)
		}
		nodes[i] = row
		return i
	}
	r := walk(root)
	dst := walk(target)
	d2rsSmallWrite(t, map[string]interface{}{"op": "contains", "nodes": nodes, "root": r, "target": dst}, got)
	return got
}
