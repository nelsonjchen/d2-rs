// SPDX-License-Identifier: MPL-2.0
package textmeasure_test

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/lib/textmeasure"
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
func d2rsSmallDanger(t *testing.T, link string) bool {
	got := textmeasure.IsDangerousLink(link)
	d2rsSmallWrite(t, map[string]interface{}{"op": "danger", "link": link}, got)
	return got
}
func d2rsSmallSafe(t *testing.T, link string) string {
	got := textmeasure.SafeMarkdownLink(link)
	d2rsSmallWrite(t, map[string]interface{}{"op": "safe", "link": link}, got)
	return got
}
