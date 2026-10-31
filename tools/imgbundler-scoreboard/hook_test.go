// SPDX-License-Identifier: MPL-2.0
package imgbundler

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/lib/localfile"
	"github.com/d2lang/d2/lib/simplelog"
)

var d2rsCalls = map[string]int{}

func d2rsRecord(t *testing.T, request, got interface{}) {
	if dir := os.Getenv("D2RS_IMGBUNDLER_DUMP"); dir != "" {
		call := d2rsCalls[t.Name()]
		d2rsCalls[t.Name()]++
		name := fmt.Sprintf("%s#%d", t.Name(), call)
		data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request, "got": got})
		if err != nil {
			t.Fatal(err)
		}
		id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
		if err := os.WriteFile(filepath.Join(dir, id+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
}
func d2rsError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsMatches(matches []imageMatch) [][4]int {
	result := make([][4]int, 0, len(matches))
	for _, m := range matches {
		result = append(result, [4]int{m.start, m.end, m.hrefStart, m.hrefEnd})
	}
	return result
}
func d2rsScan(t *testing.T, ctx context.Context, in []byte, remote bool) ([]imageMatch, []string, error) {
	matches, hrefs, err := findImageElements(ctx, in, remote)
	names := append([]string{}, hrefs...)
	d2rsRecord(t, map[string]interface{}{"op": "scan", "source": string(in), "local": !remote, "remote": remote, "canceled": ctx.Err() != nil}, map[string]interface{}{"matches": d2rsMatches(matches), "hrefs": names, "error": d2rsError(err)})
	return matches, hrefs, err
}
func d2rsApply(t *testing.T, ctx context.Context, in []byte, matches []imageMatch, replacements map[string][]byte, limit int64) ([]byte, error) {
	out, err := applyReplacements(ctx, in, matches, replacements, limit)
	values := map[string]string{}
	for key, value := range replacements {
		values[key] = string(value)
	}
	d2rsRecord(t, map[string]interface{}{"op": "apply", "source": string(in), "matches": d2rsMatches(matches), "replacements": values, "limit": limit, "canceled": ctx.Err() != nil}, map[string]interface{}{"output": string(out), "error": d2rsError(err)})
	return out, err
}
func d2rsReserve(t *testing.T, b *bundleBudget, n int64) error {
	before := b.bundledBytes
	sticky := ""
	if b.exhaustedByError != nil {
		sticky = b.exhaustedByError.Error()
	}
	err := b.reserveBundled(n)
	d2rsRecord(t, map[string]interface{}{"op": "reserve", "limit": b.maxBundledBytes, "used": before, "sticky": sticky, "count": n}, map[string]interface{}{"used": b.bundledBytes, "error": d2rsError(err)})
	return err
}
func d2rsLocal(t *testing.T, ctx context.Context, l simplelog.Logger, input string, in []byte, p localfile.Policy, cache bool) ([]byte, error) {
	out, err := BundleLocalWithPolicy(ctx, l, input, in, p, cache)
	resources := map[string]string{}
	matches, hrefs, scanErr := findImageElements(ctx, in, false)
	_ = matches
	if scanErr == nil {
		for _, href := range hrefs {
			path := href
			if !filepath.IsAbs(path) {
				path = filepath.Join(filepath.Dir(input), path)
			}
			if data, e := os.ReadFile(path); e == nil {
				resources[href] = "data:image/svg+xml;base64," + base64.StdEncoding.EncodeToString(data)
			}
		}
	}
	d2rsRecord(t, map[string]interface{}{"op": "bundle", "source": string(in), "local": true, "remote": false, "resources": resources, "prefix": "failed to bundle local images: "}, map[string]interface{}{"output": string(out), "error": d2rsError(err)})
	return out, err
}
func d2rsRemote(t *testing.T, ctx context.Context, l simplelog.Logger, in []byte, cache bool) ([]byte, error) {
	out, err := BundleRemote(ctx, l, in, cache)
	d2rsRecord(t, map[string]interface{}{"op": "bundle", "source": string(in), "local": false, "remote": true, "resources": map[string]string{}, "prefix": "failed to bundle remote images: "}, map[string]interface{}{"output": string(out), "error": d2rsError(err)})
	return out, err
}
