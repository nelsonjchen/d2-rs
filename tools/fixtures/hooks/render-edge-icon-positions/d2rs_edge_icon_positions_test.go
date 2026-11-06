// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/edge-icon-positions-source-v090.json
// (docs/FIXTURES.md): d2target.Connection.GetIconPosition for the request
// list scripts/render_probe_fixtures.py expands from
// tools/fixtures/seeds/render-edge-icon-positions.json. Overlaid into
// d2target; inert unless D2RS_RECIPE_INPUT is set. Harvested from the
// v0.9.0-era .reference/edge-icon-route-20261016/source-module/main.go.
package d2target

import (
	"encoding/json"
	"net/url"
	"os"
	"testing"
)

func TestD2RSEdgeIconPositions(t *testing.T) {
	input, output := os.Getenv("D2RS_RECIPE_INPUT"), os.Getenv("D2RS_RECIPE_OUTPUT")
	if input == "" || output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	var requests []struct {
		Connection Connection `json:"connection"`
		HasIcon    bool       `json:"has_icon"`
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	out := []any{}
	for _, r := range requests {
		if r.HasIcon {
			r.Connection.Icon = &url.URL{}
		}
		out = append(out, r.Connection.GetIconPosition())
	}
	data, err = json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
