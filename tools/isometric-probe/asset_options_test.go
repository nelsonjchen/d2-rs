// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"bytes"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/lib/imageasset"
	"github.com/d2lang/d2/lib/netpolicy"
	"io"
	"net/http"
	"os"
)

// An explicit, immutable transport input. No request may escape this map;
// diagrams and original locators are preserved verbatim for source comparisons.
type assetInput struct {
	Path string `json:"path"`
	MIME string `json:"mime"`
}
type assetTransport map[string]assetInput

func (t assetTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	v, ok := t[req.URL.String()]
	if !ok {
		return nil, fmt.Errorf("unrecorded isometric asset source %q", req.URL.String())
	}
	data, err := os.ReadFile(v.Path)
	if err != nil {
		return nil, err
	}
	if len(data) > 8<<20 {
		return nil, fmt.Errorf("isometric probe asset exceeds 8 MiB")
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{v.MIME}}, ContentLength: int64(len(data)), Body: io.NopCloser(bytes.NewReader(data)), Request: req}, nil
}
func assetOptions(sources map[string]assetInput) *d2scenebuild.AssetOptions {
	if sources == nil {
		return nil
	}
	resolver, err := imageasset.New(imageasset.Options{
		HTTPClient: &http.Client{Transport: assetTransport(sources)}, NetworkPolicy: netpolicy.Policy{AllowPrivateNetworks: true},
		Limits: imageasset.Limits{MaxFetchedBytes: 12 << 20, MaxEncodedBytes: 8 << 20, MaxDecompressedBytes: 8 << 20, MaxSVGBytes: 2 << 20, MaxDecodedWidth: 8192, MaxDecodedHeight: 8192, MaxDecodedPixels: 8 << 20, MaxAssets: 7000, MaxCumulativeEncodedBytes: 16 << 20, MaxCumulativeDecodedBytes: 64 << 20},
	})
	if err != nil {
		panic(err)
	}
	return &d2scenebuild.AssetOptions{Resolver: resolver}
}
