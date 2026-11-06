// SPDX-License-Identifier: MPL-2.0
// Development reference only. No production Rust package invokes this program.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2format"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2parser"
	"github.com/d2lang/d2/lib/textmeasure"
	"github.com/d2lang/mathjax-go"
	"io"
	"math/big"
	"os"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"time"
)

func run() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("usage: d2-oracle {parse|format|compile|measure|layout|position} [options]")
	}
	operation := os.Args[1]
	flags := flag.NewFlagSet(operation, flag.ContinueOnError)
	seedText := flags.String("tala-seeds", "1", "signed 64-bit seeds")
	path := flags.String("path", "input.d2", "source path")
	utf16 := flags.Bool("utf16", false, "UTF-16 source positions")
	if err := flags.Parse(os.Args[2:]); err != nil {
		return err
	}
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		return err
	}
	switch operation {
	case "import-paths":
		rows := []any{}
		for _, pair := range [][2]string{{"a", "/b"}, {"../a", "../../b"}, {"/a", "../../b"}, {"", ""}, {"", "a\\b"}, {"a//", "./b/../c"}} {
			i := &d2ast.Import{Pre: pair[0], Path: d2ast.MakeKeyPath([]string{pair[1], "identifier"}).Path}
			rows = append(rows, map[string]any{"record": i, "path": i.PathWithPre(), "dir": i.Dir(), "ids": len(i.IDA()), "children": len(i.Children())})
		}
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "math":
		var expressions []string
		if err := json.Unmarshal(input, &expressions); err != nil {
			return err
		}
		rows := make([]map[string]any, 0, len(expressions))
		for _, expression := range expressions {
			svg, renderErr := mathjax.Render(expression)
			width, height, measureErr := mathjax.Measure(expression)
			row := map[string]any{
				"expression": expression,
				"width":      width,
				"height":     height,
				"svg":        svg,
			}
			if renderErr != nil {
				row["render_error"] = renderErr.Error()
			}
			if measureErr != nil {
				row["measure_error"] = measureErr.Error()
			}
			rows = append(rows, row)
		}
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "ast-edges":
		edge := &d2ast.Edge{Src: d2ast.MakeKeyPath([]string{"a"}), Dst: d2ast.MakeKeyPath([]string{"b.c"}), SrcArrow: "<", DstArrow: ">"}
		same := *edge
		same.Range.Path = "different.d2"
		changed := *edge
		changed.DstArrow = "*"
		full := []string{}
		pruned := []string{}
		paths := 0
		d2ast.Walk(edge, func(n d2ast.Node) bool { full = append(full, n.Type()); return true })
		d2ast.Walk(edge, func(n d2ast.Node) bool {
			pruned = append(pruned, n.Type())
			if n.Type() == "key path" {
				paths++
				return paths != 1
			}
			return true
		})
		shared := new(3)
		indices := []*d2ast.EdgeIndex{{}, {}, {Glob: true}, {Int: shared}, {Int: shared}, {Int: new(3)}, {Int: shared, Glob: true}}
		matrix := [][]bool{}
		for _, a := range indices {
			row := []bool{}
			for _, b := range indices {
				row = append(row, a.Equals(b))
			}
			matrix = append(matrix, row)
		}
		return json.NewEncoder(os.Stdout).Encode(map[string]any{"edge": edge, "empty": &d2ast.Edge{}, "same": edge.Equals(&same), "changed": edge.Equals(&changed), "walk": full, "pruned": pruned, "indices": indices, "index_equality": matrix})
	case "key-paths":
		paths := []*d2ast.KeyPath{}
		for _, values := range [][]string{nil, {"a", "b"}, {"a.b", "*", "$v", ""}, {"null", "a-", "é"}} {
			paths = append(paths, d2ast.MakeKeyPath(values))
		}
		paths = append(paths, &d2ast.KeyPath{Path: []*d2ast.StringBox{}})
		paths = append(paths, d2ast.MakeKeyPathString([]d2ast.String{&d2ast.SingleQuotedString{Value: "a.b"}, &d2ast.DoubleQuotedString{Value: []d2ast.InterpolationBox{{String: new("a")}, {String: new("b")}}}}))
		for _, pattern := range [][]string{{"*"}, {"*", "", "*"}, {"*", "", "*", "", "*"}, {"*", "x", "*"}} {
			kp := d2ast.MakeKeyPath([]string{"a", "*"})
			kp.Last().UnquotedString = &d2ast.UnquotedString{Pattern: pattern}
			paths = append(paths, kp)
		}
		rows := []any{}
		for _, kp := range paths {
			rows = append(rows, map[string]any{"path": kp, "copy": kp.Copy(), "ids": kp.StringIDA(), "children": len(kp.Children()), "first_glob": kp.FirstGlob(), "has_glob": kp.HasGlob(), "multi": kp.HasMultiGlob(), "triple": kp.HasTripleGlob(), "equals_copy": kp.Equals(kp.Copy())})
		}
		original := d2ast.MakeKeyPath([]string{"a", "b"})
		copied := original.Copy()
		copied.Path[0] = d2ast.RawStringBox("c", true)
		copied.Path[1].UnquotedString.SetString("shared")
		rows = append(rows, map[string]any{"original": original, "edited_copy": copied, "equals": original.Equals(copied)})
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "raw-strings":
		var requests []struct {
			Text  string `json:"text"`
			InKey bool   `json:"in_key"`
		}
		if err := json.Unmarshal(input, &requests); err != nil {
			return err
		}
		result := make([]*d2ast.StringBox, 0, len(requests))
		for _, request := range requests {
			result = append(result, d2ast.RawStringBox(request.Text, request.InKey))
		}
		return json.NewEncoder(os.Stdout).Encode(result)
	case "ast-interpolation":
		sub := &d2ast.Substitution{Path: []*d2ast.StringBox{{SingleQuotedString: &d2ast.SingleQuotedString{Value: "var"}}}}
		cases := [][]d2ast.InterpolationBox{
			nil, {}, {{String: new("a")}, {String: new("b")}},
			{{String: new("a"), StringRaw: new("raw a")}, {Substitution: sub}, {String: new("tail")}},
			{{Substitution: sub}, {String: new("tail")}},
			{{String: new("a"), Substitution: sub}, {String: new("b")}},
		}
		rows := []any{}
		for index, parts := range cases {
			for _, quoted := range []bool{false, true} {
				var node d2ast.String
				if quoted {
					node = &d2ast.DoubleQuotedString{Value: parts}
				} else {
					node = &d2ast.UnquotedString{Value: parts, Pattern: []string{"*"}}
				}
				before, err := json.Marshal(node)
				if err != nil {
					return err
				}
				text := node.ScalarString()
				children := len(node.Children())
				copy := node.Copy()
				copy.SetString("replacement")
				switch value := node.(type) {
				case *d2ast.UnquotedString:
					value.Coalesce()
				case *d2ast.DoubleQuotedString:
					value.Coalesce()
				}
				rows = append(rows, map[string]any{"case": index, "quoted": quoted, "before": json.RawMessage(before), "scalar_before": text, "children_before": children, "coalesced": node, "edited_copy": copy})
			}
		}
		original := d2ast.FlatUnquotedString("original")
		copied := original.Copy().(*d2ast.UnquotedString)
		copied.Value[0].String = new("shared mutation")
		rows = append(rows, map[string]any{"copy_alias_original": original, "copy_alias_copy": copied})
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "ast-key-helpers":
		return keyHelpersOracle()
	case "ast-composite":
		return compositeOracle()
	case "ast-numbers":
		var requests []struct {
			Raw         string
			Numerator   *string
			Denominator string
		}
		if err := json.Unmarshal(input, &requests); err != nil {
			return err
		}
		rows := []any{}
		for _, request := range requests {
			n := &d2ast.Number{Raw: request.Raw}
			if request.Numerator != nil {
				numerator, ok := new(big.Int).SetString(*request.Numerator, 10)
				if !ok {
					return fmt.Errorf("invalid numerator")
				}
				denominator, ok := new(big.Int).SetString(request.Denominator, 10)
				if !ok {
					return fmt.Errorf("invalid denominator")
				}
				n.Value = new(big.Rat).SetFrac(numerator, denominator)
			}
			rows = append(rows, map[string]any{"node": n, "type": n.Type(), "scalar": n.ScalarString(), "children_count": len(n.Children())})
		}
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "ast-leaves":
		rows := []any{}
		for _, r := range []d2ast.Range{{}, {Path: "folder,a-é.d2", Start: d2ast.Position{Line: 1, Column: 2, Byte: 3}, End: d2ast.Position{Line: 4, Column: 5, Byte: 6}}} {
			nodes := []d2ast.Node{
				&d2ast.Comment{Range: r, Value: "text\n# é"}, &d2ast.BlockComment{Range: r, Value: "text\n# é"},
				&d2ast.Null{Range: r}, &d2ast.Suspension{Range: r}, &d2ast.Suspension{Range: r, Value: true},
				&d2ast.Boolean{Range: r}, &d2ast.Boolean{Range: r, Value: true},
			}
			single := &d2ast.SingleQuotedString{Range: r, Raw: "old spelling", Value: "old"}
			editedSingle := single.Copy()
			editedSingle.SetString("new\né")
			block := &d2ast.BlockString{Range: r, Quote: "||||", Tag: "md", Value: "old"}
			editedBlock := block.Copy()
			editedBlock.SetString("new\né")
			nodes = append(nodes, single, editedSingle, block, editedBlock, &d2ast.BlockString{Range: r})
			for _, node := range nodes {
				var scalar any
				if s, ok := node.(d2ast.Scalar); ok {
					scalar = s.ScalarString()
				}
				rows = append(rows, map[string]any{"node": node, "type": node.Type(), "range": node.GetRange().String(), "children_count": len(node.Children()), "scalar": scalar})
			}
			e := d2ast.Error{Range: r, Message: "bad key\nsecond line"}
			rows = append(rows, map[string]any{"error": e, "display": e.Error()})
		}
		return json.NewEncoder(os.Stdout).Encode(rows)
	case "keywords":
		sets := map[string]map[string]struct{}{
			"simple": d2ast.SimpleReservedKeywords, "holders": d2ast.ReservedKeywordHolders,
			"composite": d2ast.CompositeReservedKeywords, "style": d2ast.StyleKeywords,
			"boards": d2ast.BoardKeywords, "reserved": d2ast.ReservedKeywords,
		}
		result := map[string]any{}
		for name, set := range sets {
			keys := make([]string, 0, len(set))
			for key := range set {
				keys = append(keys, key)
			}
			sort.Strings(keys)
			result[name] = keys
		}
		result["near"] = d2ast.NearConstantsArray
		result["labels"] = d2ast.LabelPositionsArray
		result["tooltips"] = d2ast.TooltipPositionsArray
		result["fill_patterns"] = d2ast.FillPatterns
		result["text_transforms"] = d2ast.TextTransforms
		positions := map[string]string{}
		for name, position := range d2ast.LabelPositionsMapping {
			positions[name] = position.String()
		}
		result["positions"] = positions
		return json.NewEncoder(os.Stdout).Encode(result)
	case "position":
		p := d2ast.Position{}
		positions := []string{p.Debug()}
		for _, r := range string(input) {
			p = p.Advance(r, *utf16)
			positions = append(positions, p.Debug())
		}
		return json.NewEncoder(os.Stdout).Encode(positions)
	case "parse", "format":
		tree, parseErr := d2parser.Parse(*path, strings.NewReader(string(input)), &d2parser.ParseOptions{UTF16Pos: *utf16})
		if operation == "parse" {
			message := ""
			if parseErr != nil {
				message = parseErr.Error()
			}
			return json.NewEncoder(os.Stdout).Encode(map[string]any{"ast": tree, "error": message})
		}
		if parseErr != nil {
			return parseErr
		}
		_, err = fmt.Fprint(os.Stdout, d2format.Format(tree))
		return err
	case "compile", "measure":
		g, _, err := d2compiler.Compile(*path, strings.NewReader(string(input)), &d2compiler.CompileOptions{UTF16Pos: *utf16})
		if err != nil {
			return err
		}
		if operation == "measure" {
			if err = g.ApplyTheme(0); err != nil {
				return err
			}
			ruler, err := textmeasure.NewRuler()
			if err != nil {
				return err
			}
			if err = g.SetDimensions(nil, ruler, nil, nil); err != nil {
				return err
			}
		}
		output, err := d2graph.SerializeGraph(g)
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(output)
		return err
	case "layout":
		var seeds []int64
		for _, s := range strings.Split(*seedText, ",") {
			n, err := strconv.ParseInt(s, 10, 64)
			if err != nil {
				return err
			}
			seeds = append(seeds, n)
		}
		var g d2graph.Graph
		if err = d2graph.DeserializeGraph(input, &g); err != nil {
			return err
		}
		ctx, cancel := context.WithTimeout(context.Background(), 120*time.Second)
		defer cancel()
		if err = d2talalayout.Layout(ctx, &g, &d2talalayout.Options{Seeds: seeds, MaxConcurrency: 1}); err != nil {
			return err
		}
		output, err := d2graph.SerializeGraph(&g)
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(output)
		return err
	default:
		return fmt.Errorf("unknown reference operation %q", operation)
	}
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// Test independent JSON box fields against upstream's selection methods.
func compositeOracle() error {
	rows := []any{}
	name := func(n d2ast.Node) any {
		if n == nil {
			return nil
		}
		return n.Type()
	}
	for _, prototype := range []any{d2ast.MapNodeBox{}, d2ast.ArrayNodeBox{}, d2ast.ValueBox{}, d2ast.ScalarBox{}} {
		t := reflect.TypeOf(prototype)
		masks := []int{0, (1 << t.NumField()) - 1}
		for i := 0; i < t.NumField(); i++ {
			masks = append(masks, 1<<i)
			for j := i + 1; j < t.NumField(); j++ {
				masks = append(masks, (1<<i)|(1<<j))
			}
		}
		for _, mask := range masks {
			v := reflect.New(t).Elem()
			for i := 0; i < t.NumField(); i++ {
				if mask&(1<<i) != 0 {
					v.Field(i).Set(reflect.New(t.Field(i).Type.Elem()))
				}
			}
			row := map[string]any{"box": v.Interface(), "mask": mask, "kind": t.Name()}
			switch b := v.Interface().(type) {
			case d2ast.MapNodeBox:
				row["selected"] = name(b.Unbox())
				row["board"] = b.IsBoardNode()
			case d2ast.ArrayNodeBox:
				row["selected"] = name(b.Unbox())
			case d2ast.ValueBox:
				row["selected"] = name(b.Unbox())
				row["scalar_box"] = b.ScalarBox()
				row["string_box"] = b.StringBox()
			case d2ast.ScalarBox:
				row["selected"] = name(b.Unbox())
				if b.Unbox() != nil {
					row["text"] = b.ScalarString()
				}
			}
			rows = append(rows, row)
		}
	}
	rows = append(rows, map[string]any{"suspension_array": d2ast.MakeArrayNodeBox(&d2ast.Suspension{Value: true})})
	for _, board := range []string{"layers", "scenarios", "steps", "Layers", "ordinary"} {
		k := &d2ast.Key{Key: d2ast.MakeKeyPath([]string{board}), Ampersand: true,
			Edges: []*d2ast.Edge{nil, {}}, EdgeIndex: &d2ast.EdgeIndex{}, EdgeKey: d2ast.MakeKeyPath([]string{"label"}),
			Primary: d2ast.ScalarBox{Boolean: &d2ast.Boolean{Value: true}}, Value: d2ast.ValueBox{Array: &d2ast.Array{Nodes: []d2ast.ArrayNodeBox{{}, {Null: &d2ast.Null{}}}}}}
		box := d2ast.MapNodeBox{Comment: &d2ast.Comment{}, MapKey: k}
		m := &d2ast.Map{Nodes: []d2ast.MapNodeBox{{}, box, d2ast.MakeMapNodeBox(k)}}
		types := []string{}
		d2ast.Walk(m, func(n d2ast.Node) bool { types = append(types, n.Type()); return true })
		before, _ := json.Marshal(m)
		k.SetScalar(d2ast.ScalarBox{Number: &d2ast.Number{Raw: "replacement"}})
		rows = append(rows, map[string]any{"board_name": board, "board": box.IsBoardNode(), "filter": m.HasFilter(), "file": m.IsFileMap(), "before": json.RawMessage(before), "walk": types, "after": m})
	}
	return json.NewEncoder(os.Stdout).Encode(rows)
}

func keyHelpersOracle() error {
	rows := []any{}
	c := &d2ast.Comment{Value: "cursor"}
	missing := &d2ast.Comment{Value: "cursor"}
	added := &d2ast.Comment{Value: "new"}
	for _, after := range []bool{false, true} {
		for _, cursor := range []d2ast.MapNode{nil, c, missing} {
			m := &d2ast.Map{Nodes: []d2ast.MapNodeBox{{}, d2ast.MakeMapNodeBox(c), {}, d2ast.MakeMapNodeBox(c)}}
			old := *m
			if after {
				m.InsertAfter(cursor, added)
			} else {
				m.InsertBefore(cursor, added)
			}
			rows = append(rows, map[string]any{"after": after, "map": m, "old": old})
		}
	}
	keys := []*d2ast.Key{{}, {Ampersand: true}, {NotAmpersand: true}, {Key: d2ast.MakeKeyPath([]string{"a"})},
		{Primary: d2ast.ScalarBox{Boolean: &d2ast.Boolean{Value: true}}},
		{Value: d2ast.ValueBox{Null: &d2ast.Null{}}}, {Value: d2ast.ValueBox{Suspension: &d2ast.Suspension{}}},
		{Value: d2ast.ValueBox{Map: &d2ast.Map{}}}, {Value: d2ast.ValueBox{Array: &d2ast.Array{}}},
		{EdgeIndex: &d2ast.EdgeIndex{Int: new(3)}}, {EdgeIndex: &d2ast.EdgeIndex{Int: new(3)}},
		{Edges: []*d2ast.Edge{{Src: d2ast.MakeKeyPath([]string{"a"}), Dst: d2ast.MakeKeyPath([]string{"b"})}}},
		{Edges: []*d2ast.Edge{{Src: d2ast.MakeKeyPath([]string{"a"}), Dst: d2ast.MakeKeyPath([]string{"c"})}}},
		{Value: d2ast.ValueBox{Map: &d2ast.Map{Nodes: []d2ast.MapNodeBox{{MapKey: &d2ast.Key{Ampersand: true}}}}}},
		{Value: d2ast.ValueBox{Map: &d2ast.Map{Nodes: []d2ast.MapNodeBox{{Comment: &d2ast.Comment{}}}}}}}
	for _, a := range keys {
		eq := []bool{}
		oracle := []bool{}
		for _, b := range keys {
			eq = append(eq, a.Equals(b))
			oracle = append(oracle, a.D2OracleEquals(b))
		}
		rows = append(rows, map[string]any{"equals": eq, "oracle": oracle})
	}
	for _, pattern := range [][]string{nil, {"*"}, {"*", "", "*"}, {"*", "", "*", "", "*"}} {
		path := &d2ast.KeyPath{Path: []*d2ast.StringBox{{UnquotedString: &d2ast.UnquotedString{Pattern: pattern}}}}
		for location := 0; location < 5; location++ {
			k := &d2ast.Key{}
			switch location {
			case 0:
				k.Key = path
			case 1:
				k.Edges = []*d2ast.Edge{{Src: path}}
			case 2:
				k.Edges = []*d2ast.Edge{{Dst: path}}
			case 3:
				k.EdgeKey = path
			case 4:
				k.EdgeIndex = &d2ast.EdgeIndex{Glob: true}
			}
			rows = append(rows, map[string]any{"glob": k.HasGlob(), "multi": k.HasMultiGlob(), "triple": k.HasTripleGlob(), "filters": k.SupportsGlobFilters()})
		}
	}
	return json.NewEncoder(os.Stdout).Encode(rows)
}
