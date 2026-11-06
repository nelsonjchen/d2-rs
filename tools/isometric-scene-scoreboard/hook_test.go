// SPDX-License-Identifier: MPL-2.0
package d2isometric

import (
	"encoding/json"
	"github.com/d2lang/d2/d2target"
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

var d2rsSceneRows []interface{}
var d2rsSceneIDs map[*Scene]int

func d2rsSceneSnapshot(v interface{}) interface{} {
	b, e := json.Marshal(v)
	if e != nil {
		panic(e)
	}
	var out interface{}
	if e = json.Unmarshal(b, &out); e != nil {
		panic(e)
	}
	return out
}
func d2rsSceneStart(t *testing.T) {
	d2rsSceneRows = []interface{}{}
	d2rsSceneIDs = map[*Scene]int{}
	t.Cleanup(func() {
		b, e := json.Marshal(map[string]interface{}{"name": t.Name(), "operations": d2rsSceneRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func d2rsSceneRecord(request, got interface{}) {
	d2rsSceneRows = append(d2rsSceneRows, map[string]interface{}{"request": d2rsSceneSnapshot(request), "got": d2rsSceneSnapshot(got)})
}
func d2rsSceneBuild(t *testing.T, d *d2target.Diagram, o *RenderOpts) (*Scene, error) {
	request := d2rsSceneSnapshot(map[string]interface{}{"op": "build", "diagram": d, "options": o, "icon_urls": d2rsSceneIconURLs(d)})
	s, e := BuildScene(d, o)
	id := len(d2rsSceneIDs) + 1
	d2rsSceneIDs[s] = id
	request.(map[string]interface{})["id"] = id
	message := ""
	if e != nil {
		message = e.Error()
	}
	d2rsSceneRecord(request, map[string]interface{}{"scene": s, "source": d, "error": message})
	return s, e
}
func d2rsSceneMutate(t *testing.T, d *d2target.Diagram, s *Scene, mutation string) {
	switch mutation {
	case "column":
		s.Nodes[0].Metadata.Original.Columns[0].Constraint[0] = "changed"
	case "field":
		s.Nodes[1].Metadata.Original.Fields[0].Name = "changed"
	case "route444":
		s.Edges[0].Metadata.Original.Route[0].X = 444
	case "route998":
		s.Edges[0].Metadata.Original.Route[0].X = 998
	case "source-label":
		s.Edges[0].SourceLabel.Label = "changed"
	case "point":
		s.Edges[0].Points[0].X = 999
	default:
		t.Fatal("unknown mutation", mutation)
	}
	d2rsSceneRecord(map[string]interface{}{"op": "mutate", "id": d2rsSceneIDs[s], "mutation": mutation}, map[string]interface{}{"source": d, "error": ""})
}

// URL.Userinfo is private and encoding/json writes {}. Preserve the authored
// string separately as a SOURCE INPUT, rather than inventing credentials.
func d2rsSceneIconURLs(d *d2target.Diagram) map[string]interface{} {
	out := map[string]interface{}{}
	for i, s := range d.Shapes {
		if s.Icon != nil {
			out["shape:"+strconv.Itoa(i)] = s.Icon.String()
		}
	}
	for i, c := range d.Connections {
		if c.Icon != nil {
			out["edge:"+strconv.Itoa(i)] = c.Icon.String()
		}
	}
	return out
}
