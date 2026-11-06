// SPDX-License-Identifier: MPL-2.0
package d2grid

import (
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2graph"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func d2rsGridRecord(t *testing.T, input, result map[string]any) {
	t.Helper()
	dir := filepath.Join(os.Getenv("D2RS_GRID_CAPACITY_CAPTURE"), strings.ReplaceAll(t.Name(), "/", "_"))
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	files, err := filepath.Glob(filepath.Join(dir, "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(map[string]any{"test": t.Name(), "input": input, "result": result})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, fmt.Sprintf("%02d.json", len(files))), data, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsGridConstruct(t *testing.T, root *d2graph.Object) (*gridDiagram, error) {
	t.Helper()
	rows, _ := strconv.Atoi(root.GridRows.Value)
	columns, _ := strconv.Atoi(root.GridColumns.Value)
	input := map[string]any{"operation": "construct", "rows": rows, "columns": columns, "objects": len(root.ChildrenArray)}
	gd, err := newGridDiagram(root)
	result := map[string]any{"error": ""}
	if err != nil {
		result["error"] = err.Error()
	} else {
		occupiedRows, occupiedColumns := gd.occupiedDimensions()
		result["rows"] = gd.rows
		result["columns"] = gd.columns
		result["occupied_rows"] = occupiedRows
		result["occupied_columns"] = occupiedColumns
	}
	d2rsGridRecord(t, input, result)
	return gd, err
}

func d2rsGridCapacity(t *testing.T, rows, columns int) (int, error) {
	t.Helper()
	capacity, err := d2graph.GridCapacity(rows, columns)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsGridRecord(t, map[string]any{"operation": "capacity", "rows": rows, "columns": columns, "objects": 0}, map[string]any{"error": message, "capacity": capacity})
	return capacity, err
}

func d2rsGridLayout(t *testing.T, gd *gridDiagram, g *d2graph.Graph, obj *d2graph.Object) {
	t.Helper()
	input := map[string]any{"operation": "layout", "rows": gd.rows, "columns": gd.columns, "objects": len(gd.objects)}
	gd.layoutEvenly(g, obj)
	d2rsGridRecord(t, input, map[string]any{"error": "", "width": gd.width, "height": gd.height})
}
