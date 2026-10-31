// SPDX-License-Identifier: MPL-2.0
package imgbundler

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/lib/localfile"
	"github.com/d2lang/d2/lib/simplelog"
)

var d2rsPolicyCalls = map[string]int{}

func d2rsPolicyRecord(t *testing.T, inputPath string, in []byte, policy localfile.Policy, mode string, cache bool, call func() ([]byte, error)) ([]byte, error) {
	dir := os.Getenv("D2RS_IMGBUNDLER_POLICY_DUMP")
	if dir == "" {
		return call()
	}
	root, rooted := policy.RootPath()
	var fixture string
	if rooted {
		fixture = filepath.Dir(root)
	} else {
		matches := imageRegex.FindAllSubmatch(in, -1)
		if len(matches) != 1 {
			t.Fatal("expected single local fixture")
		}
		fixture = filepath.Dir(string(matches[0][1]))
	}
	fixture, err := filepath.Abs(fixture)
	if err != nil {
		t.Fatal(err)
	}
	// Capture actual filesystem inputs before calling the untouched API. Large
	// sparse zero fixtures stay sparse; arbitrary nonzero bytes cannot be omitted.
	entries := map[string]interface{}{}
	err = filepath.WalkDir(fixture, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		relative, err := filepath.Rel(fixture, path)
		if err != nil {
			return err
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		switch {
		case entry.Type()&os.ModeSymlink != 0:
			target, err := os.Readlink(path)
			if err != nil {
				return err
			}
			entries[relative] = map[string]interface{}{"link": target}
		case entry.IsDir():
			entries[relative] = map[string]interface{}{"directory": true}
		case info.Size() > 1<<20:
			file, err := os.Open(path)
			if err != nil {
				return err
			}
			defer file.Close()
			hash := sha256.New()
			buffer := make([]byte, 32<<10)
			for {
				n, err := file.Read(buffer)
				for _, b := range buffer[:n] {
					if b != 0 {
						return fmt.Errorf("cannot omit nonzero fixture %s", path)
					}
				}
				hash.Write(buffer[:n])
				if err == io.EOF {
					break
				}
				if err != nil {
					return err
				}
			}
			entries[relative] = map[string]interface{}{"zero_bytes": info.Size(), "sha256": fmt.Sprintf("%x", hash.Sum(nil))}
		default:
			body, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			entries[relative] = map[string]interface{}{"body": base64.StdEncoding.EncodeToString(body)}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	output, callErr := call()
	errorText := ""
	if callErr != nil {
		errorText = callErr.Error()
	}
	index := d2rsPolicyCalls[t.Name()]
	d2rsPolicyCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), index)
	data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": map[string]interface{}{"op": "policy-bundle", "policy": mode, "root": root, "input_path": inputPath, "source": string(in), "cache": cache, "fixture": fixture, "entries": entries, "prefix": "", "local": true, "remote": false}, "got": map[string]interface{}{"output": string(output), "error": errorText}})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), data, 0600); err != nil {
		t.Fatal(err)
	}
	return output, callErr
}
func d2rsBundleLocal(t *testing.T, ctx context.Context, l simplelog.Logger, inputPath string, in []byte, cache bool) ([]byte, error) {
	return d2rsPolicyRecord(t, inputPath, in, localfile.Policy{}, "denied", cache, func() ([]byte, error) { return BundleLocal(ctx, l, inputPath, in, cache) })
}
func d2rsBundleLocalWithPolicy(t *testing.T, ctx context.Context, l simplelog.Logger, inputPath string, in []byte, p localfile.Policy, cache bool) ([]byte, error) {
	mode := "unrestricted"
	if _, ok := p.RootPath(); ok {
		mode = "rooted"
	}
	return d2rsPolicyRecord(t, inputPath, in, p, mode, cache, func() ([]byte, error) { return BundleLocalWithPolicy(ctx, l, inputPath, in, p, cache) })
}
