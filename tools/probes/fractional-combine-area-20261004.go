// SPDX-License-Identifier: MPL-2.0
// Diagnostic overlay for the pinned Go D2 v0.9.0 component packer.
package packing

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/label"
)

func TestFractionalCombineAreaWitness(t *testing.T) {
	output := os.Getenv("D2_FRACTIONAL_COMBINE_OUTPUT")
	if output == "" { t.Fatal("D2_FRACTIONAL_COMBINE_OUTPUT is required") }
	reports := []map[string]any{}
	for _, control := range []struct {
		name string
		sizes [2][2]float64
	}{
		{"rounded_tie", [2][2]float64{{100.1,100},{100.4,100}}},
		{"subinteger_tie", [2][2]float64{{100.001,10},{100.004,10}}},
		{"rounded_reversal", [2][2]float64{{10.49,100.49},{10.51,99.49}}},
	} {
		graphs := []*layoutgraph.Graph{}
		before := []map[string]any{}
		for i, size := range control.sizes {
			graph := layoutgraph.NewGraph()
			node := layoutgraph.NewNode(layoutgraph.EntityID(i+1),size[0],size[1])
			node.TopLeft = geo.NewPoint(0,0)
			graph.AddNodeUnchecked(node)
			topLeft,bottomRight := graph.BoundingBox()
			before = append(before,map[string]any{"id":node.ID,"size":size,"area":graph.Area(),"area_bits":math.Float64bits(graph.Area()),"bounding_box":[2]*geo.Point{topLeft,bottomRight}})
			graphs = append(graphs,graph)
		}
		combined,err := CombineSubgraphs(context.Background(),layoutgraph.NewGraph(),graphs,nil)
		if err != nil { t.Fatal(err) }
		after := []map[string]any{}
		for _,node := range combined.Nodes {
			after = append(after,map[string]any{"id":node.ID,"position":node.TopLeft,"position_bits":[2]uint64{math.Float64bits(node.TopLeft.X),math.Float64bits(node.TopLeft.Y)}})
		}
		reports = append(reports,map[string]any{"name":control.name,"before":before,"after":after})
	}
	encoded,err := json.MarshalIndent(reports,"","  "); if err != nil { t.Fatal(err) }
	if err = os.WriteFile(filepath.Join(output,"go-controls.json"),append(encoded,'\n'),0644);err != nil { t.Fatal(err) }
	edgeReports := []map[string]any{}
	for _,kind := range []string{"route","main","source","target","unset","empty"} {
		graph := layoutgraph.NewGraph()
		first := layoutgraph.NewNode(1,100.1,10)
		second := layoutgraph.NewNode(2,20.4,5)
		first.TopLeft = geo.NewPoint(0,0)
		second.TopLeft = geo.NewPoint(60,0)
		graph.AddNodeUnchecked(first)
		graph.AddNodeUnchecked(second)
		edge := graph.Connect(first,second)
		edge.Points = []*geo.Point{geo.NewPoint(10.49,-10.49),geo.NewPoint(120.49,-10.49)}
		value := &layoutgraph.Label{Text:"control",Position:label.OutsideTopCenter,Width:240,Height:33.4}
		switch kind {
		case "main": edge.Label=value; edge.LabelPercentage=0.5
		case "source": edge.SourceArrowheadLabel=value
		case "target": edge.TargetArrowheadLabel=value
		case "unset": value.Position=label.Unset; edge.Label=value
		case "empty": edge.Points=nil; edge.Label=value; edge.SourceArrowheadLabel=value; edge.TargetArrowheadLabel=value
		}
		topLeft,bottomRight := graph.BoundingBox()
		edgeReports=append(edgeReports,map[string]any{"kind":kind,"area":graph.Area(),"area_bits":math.Float64bits(graph.Area()),"bounding_box":[2]*geo.Point{topLeft,bottomRight},"points":edge.Points,"label_size":[2]float64{value.Width,value.Height}})
	}
	encoded,err=json.MarshalIndent(edgeReports,"","  ");if err!=nil {t.Fatal(err)}
	if err=os.WriteFile(filepath.Join(output,"go-edge-controls.json"),append(encoded,'\n'),0644);err!=nil {t.Fatal(err)}
	nodeReports := []map[string]any{}
	for _,position := range []label.Position{label.OutsideTopCenter,label.OutsideBottomCenter,label.OutsideLeftMiddle,label.OutsideRightMiddle} {
		graph:=layoutgraph.NewGraph()
		node:=layoutgraph.NewNode(1,128.1,128.4)
		node.TopLeft=geo.NewPoint(0,0)
		node.Label=&layoutgraph.Label{Text:"node",Position:position,Width:141.4,Height:261.1}
		graph.AddNodeUnchecked(node)
		topLeft,bottomRight:=graph.BoundingBox()
		nodeReports=append(nodeReports,map[string]any{"position":position,"area":graph.Area(),"area_bits":math.Float64bits(graph.Area()),"bounding_box":[2]*geo.Point{topLeft,bottomRight},"size":[2]float64{node.Width,node.Height},"label_size":[2]float64{node.Label.Width,node.Label.Height}})
	}
	encoded,err=json.MarshalIndent(nodeReports,"","  ");if err!=nil {t.Fatal(err)}
	if err=os.WriteFile(filepath.Join(output,"go-node-label-controls.json"),append(encoded,'\n'),0644);err!=nil {t.Fatal(err)}
}
