// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

var d2rsPrecisionRows []interface{}

func d2rsPrecisionStart(t *testing.T) {
	d2rsPrecisionRows = []interface{}{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]interface{}{"name": strings.Split(t.Name(), "/")[0], "operations": d2rsPrecisionRows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func d2rsPrecisionRecord(request, got interface{}) {
	d2rsPrecisionRows = append(d2rsPrecisionRows, map[string]interface{}{"request": request, "got": got})
}
func d2rsPrecisionPoints(points []svgPoint) [][3]float64 {
	out := [][3]float64{}
	for _, p := range points {
		out = append(out, [3]float64{p.x, p.y, p.z})
	}
	return out
}
func d2rsPrecisionBits(v float64) string { return fmt.Sprintf("%016x", math.Float64bits(v)) }
func d2rsPrecisionPath(t *testing.T, polygons [][]svgPoint) string {
	input := [][3]float64{} // single-polygon original calls
	if len(polygons) != 1 {
		t.Fatal("unexpected polygon count")
	}
	input = d2rsPrecisionPoints(polygons[0])
	got := svgPolygonPath(polygons)
	d2rsPrecisionRecord(map[string]interface{}{"op": "path", "points": input}, map[string]interface{}{"path": got, "source": d2rsPrecisionPoints(polygons[0])})
	return got
}
func d2rsPrecisionEdge(a, b svgPoint) func(svgPoint) float64 {
	return func(p svgPoint) float64 {
		got := svgEdgeDistance(a, b)(p)
		d2rsPrecisionRecord(map[string]interface{}{"op": "edge", "points": d2rsPrecisionPoints([]svgPoint{a, b, p})}, d2rsPrecisionBits(got))
		return got
	}
}
func d2rsPrecisionArea(points []svgPoint) float64 {
	got := svgPolygonArea(points)
	d2rsPrecisionRecord(map[string]interface{}{"op": "area", "points": d2rsPrecisionPoints(points)}, d2rsPrecisionBits(got))
	return got
}
func d2rsPrecisionDepth(plane svgDepthPlane, point svgPoint) float64 {
	got := plane.at(point)
	d2rsPrecisionRecord(map[string]interface{}{"op": "depth", "points": d2rsPrecisionPoints([]svgPoint{plane.origin, point}), "dx": plane.dx, "dy": plane.dy}, d2rsPrecisionBits(got))
	return got
}
func d2rsPrecisionSplit(points []svgPoint, distance func(svgPoint) float64) ([]svgPoint, []svgPoint) {
	inside, outside := svgSplitPolygon(points, distance)
	d2rsPrecisionRecord(map[string]interface{}{"op": "split_y", "points": d2rsPrecisionPoints(points)}, map[string]interface{}{"inside": d2rsPrecisionPoints(inside), "outside": d2rsPrecisionPoints(outside), "source": d2rsPrecisionPoints(points)})
	return inside, outside
}
func d2rsPrecisionState(s *svgVisibleFragmentSet, err error) interface{} {
	message := ""
	if err != nil {
		message = err.Error()
	}
	ids := []int{}
	for id := range s.fragments {
		ids = append(ids, id)
	}
	sort.Ints(ids)
	fragments := []interface{}{}
	for _, id := range ids {
		f := s.fragments[id]
		fragments = append(fragments, map[string]interface{}{"id": id, "points": d2rsPrecisionPoints(f.points), "box": [4]float64{f.box.minX, f.box.minY, f.box.maxX, f.box.maxY}})
	}
	cells := [][]int{}
	for _, cell := range s.cells {
		values := []int{}
		for id := range cell {
			values = append(values, id)
		}
		sort.Ints(values)
		cells = append(cells, values)
	}
	large := []int{}
	for id := range s.large {
		large = append(large, id)
	}
	sort.Ints(large)
	return map[string]interface{}{"error": message, "fragments": fragments, "cells": cells, "large": large, "next": s.next, "vertices": s.vertices, "references": s.references, "work": s.budget.work, "nx": s.grid.nx, "ny": s.grid.ny, "bounds": [4]float64{s.grid.bounds.minX, s.grid.bounds.minY, s.grid.bounds.maxX, s.grid.bounds.maxY}}
}
func d2rsPrecisionNew(points []svgPoint, budget *svgVisibilityBudget, maxFragments, maxVertices int) (*svgVisibleFragmentSet, error) {
	s, err := svgNewVisibleFragmentSet(points, budget, maxFragments, maxVertices)
	d2rsPrecisionRecord(map[string]interface{}{"op": "new", "points": d2rsPrecisionPoints(points), "max_fragments": maxFragments, "max_vertices": maxVertices, "grid_references": budget.limits.gridReferences}, d2rsPrecisionState(s, err))
	return s, err
}
func d2rsPrecisionAdd(s *svgVisibleFragmentSet, points []svgPoint) error {
	err := s.add(points)
	d2rsPrecisionRecord(map[string]interface{}{"op": "add", "points": d2rsPrecisionPoints(points)}, d2rsPrecisionState(s, err))
	return err
}
func d2rsPrecisionRemoved(s *svgVisibleFragmentSet, id int) {
	d2rsPrecisionRecord(map[string]interface{}{"op": "remove", "id": id}, d2rsPrecisionState(s, nil))
}
