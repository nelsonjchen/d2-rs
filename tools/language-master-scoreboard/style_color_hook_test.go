// SPDX-License-Identifier: MPL-2.0
// Env-gated replay of the retained compiler color corpus against the master API.
package color_test

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/lib/color"
)

func TestD2rsMasterStyleColors(t *testing.T) {
	input := os.Getenv("D2RS_STYLE_COLOR_INPUT")
	output := os.Getenv("D2RS_STYLE_COLOR_OUTPUT")
	if input == "" || output == "" {
		return
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct {
		Input string `json:"input"`
		Valid bool   `json:"valid"`
	}
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	if len(cases) == 0 {
		t.Fatal("empty color corpus")
	}
	for i := range cases {
		cases[i].Valid = color.ValidColor(cases[i].Input)
	}
	data, err = json.Marshal(cases)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, data, 0644); err != nil {
		t.Fatal(err)
	}
}
