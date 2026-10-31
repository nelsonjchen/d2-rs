package d2grid

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2graph"
)

// Observe the original call without changing its inputs or assertions.
func d2rsGridSearch(t *testing.T, gd *gridDiagram, target float64, columns bool) [][]*d2graph.Object {
	result := gd.getBestLayout(target, columns)
	if directory := os.Getenv("D2RS_GRID_SEARCH_CAPTURE"); directory != "" {
		values := make([]float64, len(gd.objects))
		ids := make([]string, len(gd.objects))
		groups, gap := gd.rows, gd.horizontalGap
		if columns {
			groups, gap = gd.columns, gd.verticalGap
		}
		for i, object := range gd.objects {
			values[i], ids[i] = object.Width, object.ID
			if columns {
				values[i] = object.Height
			}
		}
		rows := make([][]string, len(result))
		for i, row := range result {
			if row != nil {
				rows[i] = make([]string, len(row))
			}
			for j, object := range row {
				rows[i][j] = object.ID
			}
		}
		data, err := json.Marshal(map[string]any{
			"test":   t.Name(),
			"input":  map[string]any{"values": values, "ids": ids, "groups": groups, "gap": gap, "target": target},
			"result": rows,
		})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.MkdirAll(directory, 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(directory, strings.ReplaceAll(t.Name(), "/", "_")+".json"), data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	return result
}
