// SPDX-License-Identifier: MPL-2.0
package d2grid

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type d2rsMeasurementSession struct {
	Test   string         `json:"test"`
	Input  map[string]any `json:"input"`
	Result []any          `json:"result"`
}

var d2rsMeasurementSessions = map[*gridRowMeasurements]*d2rsMeasurementSession{}

func d2rsMeasurement(t *testing.T, m *gridRowMeasurements, start, end int) gridRowMeasurement {
	t.Helper()
	got := m.get(start, end)
	session := d2rsMeasurementSessions[m]
	if session == nil {
		sizes := make([]string, len(m.sizes))
		for i, v := range m.sizes {
			sizes[i] = fmt.Sprintf("%016x", math.Float64bits(v))
		}
		session = &d2rsMeasurementSession{Test: t.Name(), Input: map[string]any{"sizes": sizes, "gap": fmt.Sprintf("%016x", math.Float64bits(m.gap)), "ranges": [][2]int{}}, Result: []any{}}
		d2rsMeasurementSessions[m] = session
		// Tests and subtests execute sequentially; each measurement object owns one
		// session and one cleanup. The original assertions run before cleanup.
		t.Cleanup(func() {
			dir := os.Getenv("D2RS_GRID_MEASUREMENT_CAPTURE")
			if err := os.MkdirAll(dir, 0700); err != nil {
				t.Fatal(err)
			}
			files, err := filepath.Glob(filepath.Join(dir, "*.json"))
			if err != nil {
				t.Fatal(err)
			}
			data, err := json.Marshal(session)
			if err != nil {
				t.Fatal(err)
			}
			name := fmt.Sprintf("%04d-%s.json", len(files), strings.ReplaceAll(session.Test, "/", "_"))
			if err := os.WriteFile(filepath.Join(dir, name), data, 0600); err != nil {
				t.Fatal(err)
			}
			delete(d2rsMeasurementSessions, m)
		})
	}
	session.Input["ranges"] = append(session.Input["ranges"].([][2]int), [2]int{start, end})
	session.Result = append(session.Result, []any{got.start, got.end, fmt.Sprintf("%016x", math.Float64bits(got.size)), fmt.Sprintf("%016x", math.Float64bits(got.withGap))})
	return got
}
