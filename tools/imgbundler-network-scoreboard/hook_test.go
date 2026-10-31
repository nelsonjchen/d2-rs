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
	"sync/atomic"
	"testing"

	"github.com/d2lang/d2/lib/netpolicy"
	"github.com/d2lang/d2/lib/simplelog"
)

var d2rsNetworkIndex int

func d2rsNetworkBundle(t *testing.T, hits *atomic.Int32, ctx context.Context, logger simplelog.Logger, source []byte, cache bool, policy netpolicy.Policy) ([]byte, error) {
	before := hits.Load()
	output, err := BundleRemoteWithPolicy(ctx, logger, source, cache, policy)
	text := ""
	if err != nil {
		text = err.Error()
	}
	name := fmt.Sprintf("%s#%d", t.Name(), d2rsNetworkIndex)
	d2rsNetworkIndex++
	value := map[string]interface{}{"name": name, "test": t.Name(), "request": map[string]interface{}{"source": string(source), "cache": cache, "allow_private": policy.AllowPrivateNetworks, "body": base64.StdEncoding.EncodeToString(testPNGFile)}, "got": map[string]interface{}{"output": string(output), "error": text, "hits": hits.Load() - before}}
	if dir := os.Getenv("D2RS_IMGBUNDLER_NETWORK_DUMP"); dir != "" {
		data, e := json.Marshal(value)
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(dir, fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), data, 0600); e != nil {
			t.Fatal(e)
		}
	}
	return output, err
}
func d2rsBundleRemote(t *testing.T, hits *atomic.Int32, ctx context.Context, logger simplelog.Logger, source []byte, cache bool) ([]byte, error) {
	return d2rsNetworkBundle(t, hits, ctx, logger, source, cache, netpolicy.Policy{})
}
func d2rsBundleRemoteWithPolicy(t *testing.T, hits *atomic.Int32, ctx context.Context, logger simplelog.Logger, source []byte, cache bool, policy netpolicy.Policy) ([]byte, error) {
	return d2rsNetworkBundle(t, hits, ctx, logger, source, cache, policy)
}
