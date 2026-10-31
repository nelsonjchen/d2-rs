// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var d2rsOrderRows []interface{}

func d2rsOrderStart(t *testing.T) {
	d2rsOrderRows = []interface{}{}
	t.Cleanup(func() {
		name := strings.Split(t.Name(), "/")[0]
		data, err := json.Marshal(map[string]interface{}{"name": name, "operations": d2rsOrderRows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), name+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}

func d2rsOrderUnits(units []*svgPaintUnit, unitIDs map[*svgPaintUnit]int, batchIDs map[*svgPaintBatch]int) []interface{} {
	result := []interface{}{}
	for _, u := range units {
		if unitIDs[u] == 0 {
			unitIDs[u] = len(unitIDs) + 1
		}
		batches := []interface{}{}
		for _, b := range u.batches {
			if batchIDs[b] == 0 {
				batchIDs[b] = len(batchIDs) + 1
			}
			polygons := []interface{}{}
			for _, points := range b.polygons {
				polygon := [][3]float64{}
				for _, p := range points {
					polygon = append(polygon, [3]float64{p.x, p.y, p.z})
				}
				polygons = append(polygons, polygon)
			}
			batches = append(batches, map[string]interface{}{"id": batchIDs[b], "first": b.first, "polygons": polygons})
		}
		result = append(result, map[string]interface{}{"id": unitIDs[u], "batches": batches, "first": u.first, "opacity": u.opacity, "opaque": u.opaque, "depth": u.depth, "count": u.count})
	}
	return result
}

func d2rsOrderObserve(ctx context.Context, units []*svgPaintUnit) error {
	_, err := d2rsOrderCall(ctx, units, svgDefaultVisibilityLimits, false, false)
	return err
}
func d2rsOrderWithLimits(ctx context.Context, units []*svgPaintUnit, limits svgVisibilityLimits) (svgPaintOrderStats, error) {
	return d2rsOrderCall(ctx, units, limits, true, true)
}
func d2rsOrderDiscardStats(ctx context.Context, units []*svgPaintUnit, limits svgVisibilityLimits) (svgPaintOrderStats, error) {
	return d2rsOrderCall(ctx, units, limits, true, false)
}
func d2rsOrderCall(ctx context.Context, units []*svgPaintUnit, limits svgVisibilityLimits, explicitLimits, withStats bool) (svgPaintOrderStats, error) {
	unitIDs, batchIDs := map[*svgPaintUnit]int{}, map[*svgPaintBatch]int{}
	request := map[string]interface{}{
		"op": "order", "units": d2rsOrderUnits(units, unitIDs, batchIDs), "canceled": ctx.Err() != nil, "with_stats": withStats,
		"limits": map[string]int{"faces": limits.faces, "vertices": limits.vertices, "fragments": limits.fragments, "work": limits.work, "grid_references": limits.gridReferences},
	}
	var stats svgPaintOrderStats
	var err error
	if explicitLimits {
		stats, err = svgOrderPaintUnitsWithLimits(ctx, units, limits)
	} else {
		err = svgOrderPaintUnits(ctx, units)
	}
	var observedStats interface{}
	if withStats {
		observedStats = map[string]int{"relations": stats.relations, "cyclic_groups": stats.cyclicGroups, "conflicting_pairs": stats.conflictingPairs}
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsOrderRows = append(d2rsOrderRows, map[string]interface{}{"request": request, "got": map[string]interface{}{"units": d2rsOrderUnits(units, unitIDs, batchIDs), "stats": observedStats, "error": message}})
	return stats, err
}
