// SPDX-License-Identifier: MPL-2.0
package localfile

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"testing"
)

type d2rsTrace struct {
	mu   sync.Mutex
	rows []interface{}
	keys []string
	next int
}

var d2rsTraces sync.Map

func d2rsSession(t *testing.T) *d2rsTrace {
	name := strings.Split(t.Name(), "/")[0]
	value, loaded := d2rsTraces.LoadOrStore(name, &d2rsTrace{rows: []interface{}{}, keys: []string{}})
	trace := value.(*d2rsTrace)
	if !loaded {
		t.Cleanup(func() {
			dir := os.Getenv("D2RS_LOCALFILE_DUMP")
			if dir == "" {
				return
			}
			trace.mu.Lock()
			defer trace.mu.Unlock()
			data, e := json.Marshal(map[string]interface{}{"name": t.Name(), "test": t.Name(), "operations": trace.rows})
			if e != nil {
				t.Fatal(e)
			}
			if e = os.WriteFile(filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name())))), data, 0600); e != nil {
				t.Fatal(e)
			}
		})
	}
	return trace
}
func d2rsErr(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsRecord(t *testing.T, request, got interface{}) {
	s := d2rsSession(t)
	s.mu.Lock()
	defer s.mu.Unlock()
	s.rows = append(s.rows, map[string]interface{}{"request": request, "got": got})
}
func d2rsID(t *testing.T) int {
	s := d2rsSession(t)
	s.mu.Lock()
	defer s.mu.Unlock()
	s.next++
	return s.next
}

type d2rsPolicy struct {
	Policy
	t  *testing.T
	id int
}

func d2rsRooted(t *testing.T, path string) (d2rsPolicy, error) {
	p, e := Rooted(path)
	id := d2rsID(t)
	d2rsRecord(t, map[string]interface{}{"op": "rooted", "path": path, "id": id}, map[string]interface{}{"error": d2rsErr(e)})
	return d2rsPolicy{p, t, id}, e
}
func d2rsUnrestricted(t *testing.T) d2rsPolicy {
	id := d2rsID(t)
	d2rsRecord(t, map[string]interface{}{"op": "unrestricted", "id": id}, map[string]interface{}{})
	return d2rsPolicy{Unrestricted(), t, id}
}
func d2rsZero(t *testing.T) d2rsPolicy { return d2rsPolicy{t: t} }
func (p d2rsPolicy) Open(name string) (fs.File, error) {
	f, e := p.Policy.Open(name)
	id := d2rsID(p.t)
	d2rsRecord(p.t, map[string]interface{}{"op": "open", "id": p.id, "file": id, "path": name}, map[string]interface{}{"error": d2rsErr(e), "denied": errors.Is(e, ErrDenied)})
	if f == nil {
		return nil, e
	}
	return &d2rsFile{f, p.t, id}, e
}

type d2rsFile struct {
	fs.File
	t  *testing.T
	id int
}

func (f *d2rsFile) Read(buffer []byte) (int, error) {
	n, e := f.File.Read(buffer)
	d2rsRecord(f.t, map[string]interface{}{"op": "read", "file": f.id, "size": len(buffer)}, map[string]interface{}{"data": base64.StdEncoding.EncodeToString(buffer[:n]), "error": d2rsErr(e)})
	return n, e
}
func (f *d2rsFile) Close() error {
	e := f.File.Close()
	d2rsRecord(f.t, map[string]interface{}{"op": "file-close", "file": f.id}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func (p d2rsPolicy) CacheKey(name string) (string, error) {
	key, e := p.Policy.CacheKey(name)
	s := d2rsSession(p.t)
	s.mu.Lock()
	defer s.mu.Unlock()
	equal := []bool{}
	for _, prior := range s.keys {
		equal = append(equal, key == prior)
	}
	s.rows = append(s.rows, map[string]interface{}{"request": map[string]interface{}{"op": "cache-key", "id": p.id, "path": name}, "got": map[string]interface{}{"error": d2rsErr(e), "denied": errors.Is(e, ErrDenied), "equal_prior": equal}, "raw_key": key})
	s.keys = append(s.keys, key)
	return key, e
}
func (p d2rsPolicy) Close() error {
	e := p.Policy.Close()
	d2rsRecord(p.t, map[string]interface{}{"op": "close", "id": p.id}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func d2rsTempDir(t *testing.T) string {
	path := t.TempDir()
	d2rsRecord(t, map[string]interface{}{"op": "mkdir", "path": path, "mode": 0700}, map[string]interface{}{"error": ""})
	return path
}
func d2rsMkdir(t *testing.T, path string, mode os.FileMode) error {
	e := os.Mkdir(path, mode)
	d2rsRecord(t, map[string]interface{}{"op": "mkdir", "path": path, "mode": mode}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func d2rsWriteFile(t *testing.T, path string, data []byte, mode os.FileMode) error {
	e := os.WriteFile(path, data, mode)
	d2rsRecord(t, map[string]interface{}{"op": "write", "path": path, "body": base64.StdEncoding.EncodeToString(data), "mode": mode}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func d2rsSymlink(t *testing.T, target, path string) error {
	e := os.Symlink(target, path)
	d2rsRecord(t, map[string]interface{}{"op": "symlink", "target": target, "path": path}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func d2rsRename(t *testing.T, old, path string) error {
	e := os.Rename(old, path)
	d2rsRecord(t, map[string]interface{}{"op": "rename", "old": old, "path": path}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
func d2rsMkfifo(t *testing.T, path string, mode uint32) error {
	e := syscall.Mkfifo(path, mode)
	d2rsRecord(t, map[string]interface{}{"op": "fifo", "path": path, "mode": mode}, map[string]interface{}{"error": d2rsErr(e)})
	return e
}
