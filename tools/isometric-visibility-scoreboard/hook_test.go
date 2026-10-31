// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var d2rsVisibilityRows []interface{}

func d2rsVisibilityStart(t *testing.T) {
	d2rsVisibilityRows = []interface{}{}
	t.Cleanup(func() {
		name := strings.Split(t.Name(), "/")[0]
		data, err := json.Marshal(map[string]interface{}{"name": name, "operations": d2rsVisibilityRows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), name+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}

func d2rsVisibilityNumber(n float64) interface{} {
	if math.IsNaN(n) {
		return "NaN"
	}
	if math.IsInf(n, 1) {
		return "+Inf"
	}
	if math.IsInf(n, -1) {
		return "-Inf"
	}
	return n
}

func d2rsVisibilityWorkLimit(n int) int {
	got := svgVisibilityWorkLimit(n)
	d2rsVisibilityRows = append(d2rsVisibilityRows, map[string]interface{}{"request": map[string]interface{}{"op": "work_limit", "faces": n}, "got": got})
	return got
}

func d2rsVisibilityObserve(ctx context.Context, faces []svgVisibilityFace) ([][][]svgPoint, error) {
	return d2rsVisibilityCall(ctx, faces, nil)
}
func d2rsVisibilityWithLimits(ctx context.Context, faces []svgVisibilityFace, limits svgVisibilityLimits) ([][][]svgPoint, error) {
	return d2rsVisibilityCall(ctx, faces, &limits)
}
func d2rsVisibilityCall(ctx context.Context, faces []svgVisibilityFace, limits *svgVisibilityLimits) ([][][]svgPoint, error) {
	groups, owners := map[*nativeOpacityGroup]int{}, map[*nativePaintOwner]int{}
	input := []interface{}{}
	for _, face := range faces {
		var group, owner interface{}
		if face.group != nil {
			if groups[face.group] == 0 {
				groups[face.group] = len(groups) + 1
			}
			group = groups[face.group]
		}
		if face.owner != nil {
			if owners[face.owner] == 0 {
				owners[face.owner] = len(owners) + 1
			}
			owner = owners[face.owner]
		}
		points := []interface{}{}
		for _, p := range face.points {
			points = append(points, []interface{}{d2rsVisibilityNumber(p.x), d2rsVisibilityNumber(p.y), d2rsVisibilityNumber(p.z)})
		}
		input = append(input, map[string]interface{}{"points": points, "opaque": face.opaque, "group": group, "owner": owner, "contour": face.contour, "order": face.order})
	}
	var l interface{}
	if limits != nil {
		l = map[string]int{"faces": limits.faces, "vertices": limits.vertices, "fragments": limits.fragments, "work": limits.work, "grid_references": limits.gridReferences}
	}
	request := map[string]interface{}{"op": "visibility", "faces": input, "limits": l, "canceled": ctx.Err() != nil}
	var got [][][]svgPoint
	var err error
	if limits == nil {
		got, err = svgVisibleFaces(ctx, faces)
	} else {
		got, err = svgVisibleFacesWithLimits(ctx, faces, *limits)
	}
	var visible interface{}
	if got != nil {
		out := make([]interface{}, len(got))
		for i, fragments := range got {
			if fragments == nil {
				continue
			}
			polygons := []interface{}{}
			for _, points := range fragments {
				polygon := []interface{}{}
				for _, p := range points {
					polygon = append(polygon, []float64{p.x, p.y, p.z})
				}
				polygons = append(polygons, polygon)
			}
			out[i] = polygons
		}
		visible = out
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsVisibilityRows = append(d2rsVisibilityRows, map[string]interface{}{"request": request, "got": map[string]interface{}{"visible": visible, "error": message}})
	return got, err
}
