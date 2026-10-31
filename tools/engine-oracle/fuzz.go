// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.
package main

// This file is a small, source-shaped copy of the v0.9.0 TALA fuzz fixture
// generator. It exists only to turn Go's text fuzz corpus records into the
// graphjson boundary consumed by the native Rust adapter. Keeping the
// generator here lets the qualification exercise the same rand.Intn,
// rand.Perm, node placement, shape, container and tree choices as the pinned
// upstream test without changing the pristine source tree.

import (
	"fmt"
	"math"
	"math/rand"
	"strconv"
	"strings"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/limits"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/shape"
)

const (
	minFuzzNodeSize              = 50
	maxFuzzNodeSize              = 750
	maxFuzzNodePlacementAttempts = 5
	fuzzContainerProbability     = 0.5
	fuzzTreeProbability          = 0.1
	maxFuzzTreeDepth             = 10
	fuzzArrowheadProbability     = 0.5
)

var fuzzShapes = []string{
	shape.SQUARE_TYPE, shape.REAL_SQUARE_TYPE, shape.PARALLELOGRAM_TYPE,
	shape.DOCUMENT_TYPE, shape.CYLINDER_TYPE, shape.QUEUE_TYPE, shape.PAGE_TYPE,
	shape.PACKAGE_TYPE, shape.STEP_TYPE, shape.CALLOUT_TYPE, shape.STORED_DATA_TYPE,
	shape.PERSON_TYPE, shape.DIAMOND_TYPE, shape.OVAL_TYPE, shape.CIRCLE_TYPE,
	shape.HEXAGON_TYPE, shape.CLOUD_TYPE,
}

type fuzzConfig struct {
	maxNodes, connectionIterations                     int
	compactionFactor, nodeSubsetPercentageToConnect    float64
	minConnectionProbability, maxConnectionProbability float64
	seed                                               int64
}

func graphFromFuzzCorpus(data []byte) (*layoutgraph.Graph, error) {
	values, err := parseGoFuzzValues(string(data))
	if err != nil {
		return nil, err
	}
	if len(values) != 7 {
		return nil, fmt.Errorf("want 7 fuzz values, got %d", len(values))
	}
	seed := values[0]
	bytes := func(v int64) uint8 { return uint8(v) }
	config := normalizeFuzzConfig(seed, bytes(values[1]), bytes(values[2]), bytes(values[3]), bytes(values[4]), bytes(values[5]), bytes(values[6]))
	return createFuzzGraph(config), nil
}

func parseGoFuzzValues(text string) ([]int64, error) {
	lines := strings.Split(text, "\n")
	if len(lines) == 0 || strings.TrimSpace(lines[0]) != "go test fuzz v1" {
		return nil, fmt.Errorf("invalid Go fuzz corpus header")
	}
	values := make([]int64, 0, 7)
	for _, raw := range lines[1:] {
		line := strings.TrimSpace(raw)
		if line == "" {
			continue
		}
		if strings.HasPrefix(line, "int64(") && strings.HasSuffix(line, ")") {
			v, err := strconv.ParseInt(strings.TrimSuffix(strings.TrimPrefix(line, "int64("), ")"), 10, 64)
			if err != nil {
				return nil, err
			}
			values = append(values, v)
			continue
		}
		if strings.HasPrefix(line, "byte(") && strings.HasSuffix(line, ")") {
			literal := strings.TrimSuffix(strings.TrimPrefix(line, "byte("), ")")
			v, err := strconv.ParseUint(parseByteLiteral(literal), 0, 8)
			if err != nil {
				return nil, fmt.Errorf("byte %q: %w", literal, err)
			}
			values = append(values, int64(v))
			continue
		}
	}
	return values, nil
}

func parseByteLiteral(literal string) string {
	if strings.HasPrefix(literal, "'") && strings.HasSuffix(literal, "'") {
		body := strings.TrimSuffix(strings.TrimPrefix(literal, "'"), "'")
		switch body {
		case "\\a":
			return "7"
		case "\\b":
			return "8"
		case "\\t":
			return "9"
		case "\\n":
			return "10"
		case "\\v":
			return "11"
		case "\\f":
			return "12"
		case "\\r":
			return "13"
		case "\\\\":
			return "92"
		case "\\'":
			return "39"
		}
		if strings.HasPrefix(body, "\\x") {
			return "0" + strings.TrimPrefix(body, "\\x")
		}
		if strings.HasPrefix(body, "\\") {
			return strings.TrimPrefix(body, "\\")
		}
		return strconv.Itoa(int([]byte(body)[0]))
	}
	return literal
}

func normalizeFuzzConfig(seed int64, maxNodes, iterations, compaction, subset, minProbability, maxProbability uint8) fuzzConfig {
	minPercent := int(minProbability) % 51
	maxPercent := minPercent + int(maxProbability)%((90-minPercent)+1)
	return fuzzConfig{seed: seed, maxNodes: 2 + int(maxNodes)%29, connectionIterations: int(iterations) % 6,
		compactionFactor:              float64(10+int(compaction)%81) / 100,
		nodeSubsetPercentageToConnect: float64(10+int(subset)%81) / 100,
		minConnectionProbability:      float64(minPercent) / 100,
		maxConnectionProbability:      float64(maxPercent) / 100}
}

func createFuzzGraph(config fuzzConfig) *layoutgraph.Graph {
	random := rand.New(rand.NewSource(config.seed))
	g := layoutgraph.NewGraph()
	nNodes := int(randomInBetween(random, float64(config.maxNodes)/2, float64(config.maxNodes)))
	maxNodeSize := math.Min(maxFuzzNodeSize, float64(limits.MaxGraphSize)/float64(nNodes))
	minNodeSize := math.Max(minFuzzNodeSize, math.Sqrt(maxNodeSize))
	maxLength := computeMaxGraphSideLength(config, nNodes, maxNodeSize)
	treeRoots := make(map[*layoutgraph.Node]struct{})
	addFuzzNodes(random, nNodes, treeRoots, g, nil, geo.NewPoint(0, 0), geo.NewPoint(float64(maxLength), float64(maxLength)), minNodeSize, maxNodeSize)
	for i := 0; i < config.connectionIterations; i++ {
		connectFuzzNodes(random, config, g, treeRoots, maxLength)
	}
	return g
}

func computeMaxGraphSideLength(config fuzzConfig, nNodes int, maxNodeSize float64) int {
	maxTopLeft := limits.MaxGraphSize - maxFuzzNodeSize
	length := nNodes * int(maxNodeSize)
	length = int(float64(length) * 1.25)
	length = int(float64(length) * config.compactionFactor)
	if length > maxTopLeft {
		length = maxTopLeft
	}
	return length
}

func addFuzzNodes(random *rand.Rand, nNodes int, treeRoots map[*layoutgraph.Node]struct{}, g *layoutgraph.Graph, container *layoutgraph.Node, tl, br *geo.Point, minNodeSize, maxNodeSize float64) {
	for nNodes > 0 {
		nNodes--
		node := addFuzzNode(random, g, container, tl, br, minNodeSize, maxNodeSize)
		isContainer := random.Float64() <= fuzzContainerProbability
		hasContainerSize := math.Min(node.Width, node.Height) > minFuzzNodeSize
		nDescendants := int(math.Min(math.Min(node.Width, node.Height)/minFuzzNodeSize-1, float64(nNodes)))
		if isContainer && hasContainerSize && nNodes > 0 && nDescendants > 0 {
			nDescendants = int(randomInBetween(random, 1, float64(nDescendants)))
			maxChildrenSize := math.Min(node.Width, node.Height) / float64(nDescendants)
			addFuzzNodes(random, nDescendants, treeRoots, g, node, node.TopLeft, geo.NewPoint(node.TopLeft.X+node.Width-maxChildrenSize, node.TopLeft.Y+node.Height-maxChildrenSize), maxChildrenSize*.75, maxChildrenSize)
			nNodes -= nDescendants
		} else if random.Float64() <= fuzzTreeProbability {
			nTree := makeFuzzTree(random, node, nNodes, g, tl, br, minNodeSize, maxNodeSize)
			nNodes -= nTree
			treeRoots[node] = struct{}{}
		}
	}
}

func addFuzzNode(random *rand.Rand, g *layoutgraph.Graph, container *layoutgraph.Node, tl, br *geo.Point, minNodeSize, maxNodeSize float64) *layoutgraph.Node {
	width := randomInBetween(random, minNodeSize, maxNodeSize)
	height := randomInBetween(random, minNodeSize, maxNodeSize)
	node := layoutgraph.NewNode(layoutgraph.EntityID(len(g.Nodes)+1), math.Trunc(width), math.Trunc(height))
	node.SetShape(fuzzShapes[int(width)%len(fuzzShapes)])
	for i := 0; i < maxFuzzNodePlacementAttempts; i++ {
		node.TopLeft = geo.NewPoint(randomInBetween(random, tl.X, br.X), randomInBetween(random, tl.Y, br.Y))
		placed := g.AddNode(node)
		if placed.ID == node.ID {
			g.AddNodeToContainer(container, node)
			break
		}
	}
	return node
}

func makeFuzzTree(random *rand.Rand, root *layoutgraph.Node, maxNodes int, g *layoutgraph.Graph, tl, br *geo.Point, minNodeSize, maxNodeSize float64) int {
	treeNodes := 0
	depth := int(randomInBetween(random, 1, maxFuzzTreeDepth))
	previousLayer := []*layoutgraph.Node{root}
	for i := 0; i < depth; i++ {
		remaining := float64(maxNodes - treeNodes)
		layerNodes := int(randomInBetween(random, 0, remaining/float64(maxFuzzTreeDepth)))
		treeNodes += layerNodes
		if layerNodes == 0 {
			break
		}
		next := make([]*layoutgraph.Node, 0, layerNodes)
		for j := 0; j < layerNodes; j++ {
			node := addFuzzNode(random, g, root.Container, tl, br, minNodeSize, maxNodeSize)
			next = append(next, node)
			connectTo := int(randomInBetween(random, 0, float64(len(previousLayer)-1)))
			e := g.Connect(previousLayer[connectTo], node)
			e.ID = layoutgraph.EntityID(len(g.Edges))
			if random.Float32() <= fuzzArrowheadProbability {
				e.TargetArrowhead = layoutgraph.TriangleArrowhead
			}
		}
		previousLayer = next
	}
	return treeNodes
}

func randomInBetween(random *rand.Rand, min, max float64) float64 {
	if int(max-min) == 0 {
		return min
	}
	return float64(random.Intn(int(max-min+1))) + min
}

func connectFuzzNodes(random *rand.Rand, config fuzzConfig, g *layoutgraph.Graph, treeRoots map[*layoutgraph.Node]struct{}, maxLength int) {
	nodesToSkip := make(map[*layoutgraph.Node]struct{})
	indices := random.Perm(len(g.Nodes))
	for _, e := range g.Edges {
		if _, ok := treeRoots[e.From]; !ok {
			nodesToSkip[e.From] = struct{}{}
		}
		if _, ok := treeRoots[e.To]; !ok {
			nodesToSkip[e.To] = struct{}{}
		}
	}
	nNodesToConnect := int(float64(len(indices)) * config.nodeSubsetPercentageToConnect)
	if nNodesToConnect >= len(indices) {
		nNodesToConnect = len(indices) - 1
	}
	for i := 0; i < nNodesToConnect; i++ {
		n1 := g.Nodes[indices[i]]
		n2 := g.Nodes[indices[i+1]]
		if shouldConnectFuzz(random, config, n1, n2, nodesToSkip, maxLength) {
			edge := g.Connect(n1, n2)
			edge.ID = layoutgraph.EntityID(len(g.Edges))
			if random.Float32() <= fuzzArrowheadProbability {
				edge.SourceArrowhead = layoutgraph.TriangleArrowhead
			}
			if random.Float32() <= fuzzArrowheadProbability {
				edge.TargetArrowhead = layoutgraph.TriangleArrowhead
			}
		}
	}
}

func shouldConnectFuzz(random *rand.Rand, config fuzzConfig, n1, n2 *layoutgraph.Node, skip map[*layoutgraph.Node]struct{}, maxLength int) bool {
	if _, ok := skip[n1]; ok {
		return false
	}
	if _, ok := skip[n2]; ok {
		return false
	}
	if n1.DoesOverlapAt(n2, n1.TopLeft) || n2.DoesOverlapAt(n1, n2.TopLeft) {
		return false
	}
	d := geo.EuclideanDistance(n1.TopLeft.X, n1.TopLeft.Y, n2.TopLeft.X, n2.TopLeft.Y)
	p := 1 - d/float64(maxLength)
	if p < config.minConnectionProbability {
		p = config.minConnectionProbability
	}
	if p > config.maxConnectionProbability {
		p = config.maxConnectionProbability
	}
	return random.Float64() <= p
}
