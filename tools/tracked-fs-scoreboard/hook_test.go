// SPDX-License-Identifier: MPL-2.0
// Input-only native transport; original trackedFS assertions stay active.
package d2cli

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/d2lang/d2/lib/localfile"
)

var d2rsTrackedCalls = map[string]int{}
var d2rsTrackedDirs = map[string]int{}
var d2rsTrackedTest *testing.T

func d2rsTrackedTempDir(t *testing.T) string {
	d2rsTrackedTest = t
	n := d2rsTrackedDirs[t.Name()]
	d2rsTrackedDirs[t.Name()] = n + 1
	path := filepath.Join(os.Getenv("D2RS_TRACKED_WORK"), fmt.Sprintf("%x-%03d", sha256.Sum256([]byte(t.Name())), n))
	if err := os.Mkdir(path, 0700); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.RemoveAll(path); err != nil {
			t.Error(err)
		}
	})
	return path
}

type d2rsTrackedFS struct {
	localFiles localfile.Policy
	opened     []string
}

func (f *d2rsTrackedFS) Open(path string) (fs.File, error) {
	t := d2rsTrackedTest
	n := d2rsTrackedCalls[t.Name()]
	d2rsTrackedCalls[t.Name()] = n + 1
	policy, root := "denied", ""
	// The observer transforms constructors, not the test assertions.
	if p, ok := d2rsTrackedPolicies[f.localFiles]; ok {
		policy, root = p[0], p[1]
	}
	request := map[string]any{"policy": policy, "root": root, "path": path}
	var result struct {
		Error  string   `json:"error"`
		Denied bool     `json:"denied"`
		Opened []string `json:"opened"`
	}
	var file fs.File
	var err error
	base := filepath.Join(os.Getenv("D2RS_TRACKED_CAPTURE"), fmt.Sprintf("%x-%03d", sha256.Sum256([]byte(t.Name())), n))
	if binary := os.Getenv("D2RS_TRACKED_NATIVE"); binary != "" {
		data, e := json.Marshal(request)
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(base+"-request.json", data, 0600); e != nil {
			t.Fatal(e)
		}
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, binary, "--exact", "watch::tracked_fs_api_tests::replay_open", "--ignored", "--nocapture")
		cmd.Env = append(os.Environ(), "D2RS_TRACKED_REQUEST="+base+"-request.json", "D2RS_TRACKED_RESPONSE="+base+"-response.json")
		output, e := cmd.CombinedOutput()
		if e != nil {
			t.Fatalf("native tracked transport: %v: %s", e, output)
		}
		data, e = os.ReadFile(base + "-response.json")
		if e != nil {
			t.Fatal(e)
		}
		if e = json.Unmarshal(data, &result); e != nil {
			t.Fatal(e)
		}
		if result.Error != "" {
			if result.Denied {
				err = fmt.Errorf("%s: %w", result.Error, localfile.ErrDenied)
			} else {
				err = errors.New(result.Error)
			}
		} else {
			file = d2rsTrackedClosedFile{}
		}
	} else {
		original := trackedFS{localFiles: f.localFiles}
		file, err = original.Open(path)
		if err != nil {
			result.Error = err.Error()
			result.Denied = errors.Is(err, localfile.ErrDenied)
		} else {
			result.Opened = original.opened
		}
	}
	f.opened = append(f.opened, result.Opened...)
	data, e := json.Marshal(map[string]any{"test": t.Name(), "call": n, "input": request, "result": result, "opened": f.opened})
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(base+"-call.json", data, 0600); e != nil {
		t.Fatal(e)
	}
	return file, err
}

// The original tests only close a successfully opened descriptor. No second
// native open is performed: it would bypass the boundary under test.
type d2rsTrackedClosedFile struct{}

func (d2rsTrackedClosedFile) Close() error { return nil }
func (d2rsTrackedClosedFile) Read([]byte) (int, error) {
	return 0, errors.New("transport is close-only")
}
func (d2rsTrackedClosedFile) Stat() (fs.FileInfo, error) {
	return nil, errors.New("transport is close-only")
}

var d2rsTrackedPolicies = map[localfile.Policy][2]string{}

func d2rsTrackedRooted(path string) (localfile.Policy, error) {
	p, e := localfile.Rooted(path)
	if e == nil {
		d2rsTrackedPolicies[p] = [2]string{"rooted", path}
	}
	return p, e
}
func d2rsTrackedUnrestricted() localfile.Policy {
	p := localfile.Unrestricted()
	d2rsTrackedPolicies[p] = [2]string{"unrestricted", ""}
	return p
}
