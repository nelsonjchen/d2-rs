// SPDX-License-Identifier: MPL-2.0
// Compiled alongside a verified copy of pinned parse.go, with package renamed.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2format"
	"github.com/d2lang/d2/d2ir"
	"github.com/d2lang/d2/d2parser"
	goast "go/ast"
	goparser "go/parser"
	"go/token"
	"math/big"
	"os"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"unicode"
	"unsafe"
)

// Private calls are diagnostic oracle hooks; the linked implementation is
// the pristine d2ir package, not a reimplementation in this harness.
//
//go:linkname appendFieldRef github.com/d2lang/d2/d2ir.(*Field).appendReference
func appendFieldRef(*d2ir.Field, *d2ir.FieldReference)

//go:linkname appendEdgeRef github.com/d2lang/d2/d2ir.(*Edge).appendReference
func appendEdgeRef(*d2ir.Edge, *d2ir.EdgeReference)

//go:linkname sameRefContext github.com/d2lang/d2/d2ir.sameReferenceContext
func sameRefContext(*d2ir.RefContext, *d2ir.RefContext) bool

//go:linkname cloneASTMap github.com/d2lang/d2/d2ir.cloneASTMap
func cloneASTMap(*d2ast.Map) *d2ast.Map

//go:linkname markStructureChanged github.com/d2lang/d2/d2ir.(*Map).markStructureChanged
func markStructureChanged(*d2ir.Map)

//go:linkname appendMapField github.com/d2lang/d2/d2ir.(*Map).appendField
func appendMapField(*d2ir.Map, *d2ir.Field)

//go:linkname insertMapField github.com/d2lang/d2/d2ir.(*Map).insertField
func insertMapField(*d2ir.Map, int, *d2ir.Field)

//go:linkname removeMapField github.com/d2lang/d2/d2ir.(*Map).removeField
func removeMapField(*d2ir.Map, int) *d2ir.Field

//go:linkname appendMapEdge github.com/d2lang/d2/d2ir.(*Map).appendEdge
func appendMapEdge(*d2ir.Map, *d2ir.Edge)

//go:linkname removeMapEdge github.com/d2lang/d2/d2ir.(*Map).removeEdge
func removeMapEdge(*d2ir.Map, int) *d2ir.Edge

//go:linkname indexedField github.com/d2lang/d2/d2ir.(*Map).getFieldIndexed
func indexedField(*d2ir.Map, ...d2ast.String) *d2ir.Field

//go:linkname indexedEdges github.com/d2lang/d2/d2ir.(*Map).getEdgesIndexed
func indexedEdges(*d2ir.Map, *d2ir.EdgeID) []*d2ir.Edge

//go:linkname resolveEdge github.com/d2lang/d2/d2ir.(*EdgeID).resolve
func resolveEdge(*d2ir.EdgeID, *d2ir.Map) (*d2ir.EdgeID, *d2ir.Map, []d2ast.String, error)

//go:linkname overlayMapIndexed github.com/d2lang/d2/d2ir.overlayMapIndexed
func overlayMapIndexed(*d2ir.Map, *d2ir.Map)

//go:linkname overlayFieldIndexed github.com/d2lang/d2/d2ir.overlayFieldIndexed
func overlayFieldIndexed(*d2ir.Field, *d2ir.Field)

//go:linkname overlayEdgeMode github.com/d2lang/d2/d2ir.overlayEdge
func overlayEdgeMode(*d2ir.Edge, *d2ir.Edge, bool)

//go:linkname expandSubstitutionIndexed github.com/d2lang/d2/d2ir.expandSubstitutionIndexed
func expandSubstitutionIndexed(*d2ir.Map, *d2ir.Map, *d2ir.Field)

//go:linkname cloneImportMap github.com/d2lang/d2/d2ir.cloneImportMap
func cloneImportMap(*d2ir.Map) *d2ir.Map

//go:linkname cloneImportField github.com/d2lang/d2/d2ir.cloneImportField
func cloneImportField(*d2ir.Field) *d2ir.Field

//go:linkname canSelectImport github.com/d2lang/d2/d2ir.canSelectImport
func canSelectImport(d2ir.Node) bool

//go:linkname irMatchPattern github.com/d2lang/d2/d2ir.matchPattern
func irMatchPattern(string, []string) bool

//go:linkname irMultiGlob github.com/d2lang/d2/d2ir.(*Map).multiGlob
func irMultiGlob(*d2ir.Map, []string, func(*d2ir.Field) bool) ([]*d2ir.Field, bool)

//go:linkname irPathToField github.com/d2lang/d2/d2ir.(*Map).pathToField
func irPathToField(*d2ir.Map, *d2ir.Field) []*d2ir.Field

//go:linkname irTowardGlob github.com/d2lang/d2/d2ir.(*Map).multiGlobMatchesToward
func irTowardGlob(*d2ir.Map, *d2ir.Field, []string, func(*d2ir.Field) bool) []*d2ir.Field

type oracleReader struct {
	reader      *strings.Reader
	calls, fail int
}

func (r *oracleReader) ReadRune() (rune, int, error) {
	if r.calls == r.fail {
		return 0, 0, errors.New("oracle read failure")
	}
	r.calls++
	return r.reader.ReadRune()
}
func main() {
	if len(os.Args) > 2 && os.Args[1] == "parse-fixtures" {
		file, err := goparser.ParseFile(token.NewFileSet(), os.Args[2], nil, 0)
		if err != nil {
			panic(err)
		}
		var stringValue func(goast.Expr) (string, bool)
		stringValue = func(expr goast.Expr) (string, bool) {
			switch value := expr.(type) {
			case *goast.BasicLit:
				if value.Kind != token.STRING {
					return "", false
				}
				decoded, err := strconv.Unquote(value.Value)
				return decoded, err == nil
			case *goast.BinaryExpr:
				if value.Op != token.ADD {
					return "", false
				}
				left, ok := stringValue(value.X)
				if !ok {
					return "", false
				}
				right, ok := stringValue(value.Y)
				if !ok {
					return "", false
				}
				return left + right, true
			default:
				return "", false
			}
		}
		fixtures := []map[string]any{}
		for _, decl := range file.Decls {
			fn, ok := decl.(*goast.FuncDecl)
			if !ok {
				continue
			}
			if fn.Name.Name != "TestParse" && fn.Name.Name != "testImport" {
				continue
			}
			var body *goast.BlockStmt = fn.Body
			var targetName, targetPath string
			if fn.Name.Name == "TestParse" {
				targetName = "testCases"
				targetPath = "TestParse"
			} else {
				targetName = "tca"
				targetPath = "TestParse/import"
			}
			goast.Inspect(body, func(node goast.Node) bool {
				var values goast.Expr
				switch statement := node.(type) {
				case *goast.ValueSpec:
					for i, name := range statement.Names {
						if name.Name == targetName && i < len(statement.Values) {
							values = statement.Values[i]
							break
						}
					}
				case *goast.AssignStmt:
					for i, left := range statement.Lhs {
						if ident, ok := left.(*goast.Ident); ok && ident.Name == targetName && i < len(statement.Rhs) {
							values = statement.Rhs[i]
							break
						}
					}
				}
				literal, ok := values.(*goast.CompositeLit)
				if !ok {
					return true
				}
				for index, element := range literal.Elts {
					test, ok := element.(*goast.CompositeLit)
					if !ok {
						panic("unexpected parser fixture element")
					}
					name := ""
					source := ""
					for _, field := range test.Elts {
						pair, ok := field.(*goast.KeyValueExpr)
						if !ok {
							continue
						}
						key, ok := pair.Key.(*goast.Ident)
						if !ok {
							continue
						}
						if key.Name == "name" {
							var good bool
							name, good = stringValue(pair.Value)
							if !good {
								panic("unsupported parser fixture name")
							}
						}
						if key.Name == "text" {
							var good bool
							source, good = stringValue(pair.Value)
							if !good {
								panic("unsupported parser fixture text in " + targetPath)
							}
						}
					}
					if name == "" {
						name = fmt.Sprintf("#%02d", index)
					}
					path := "d2/testdata/d2parser/" + targetPath + "/" + name + ".d2"
					fixtures = append(fixtures, map[string]any{"name": targetPath + "/" + name, "path": path, "hex": hex.EncodeToString([]byte(source))})
				}
				return false
			})
		}
		json.NewEncoder(os.Stdout).Encode(fixtures)
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "reserved-keywords" {
		values := []string{}
		for key := range d2ast.ReservedKeywords {
			values = append(values, key)
		}
		json.NewEncoder(os.Stdout).Encode(values)
		return
	}
	if len(os.Args) > 2 && os.Args[1] == "format-fixtures" {
		file, err := goparser.ParseFile(token.NewFileSet(), os.Args[2], nil, 0)
		if err != nil {
			panic(err)
		}
		var evaluate func(goast.Expr) string
		evaluate = func(e goast.Expr) string {
			switch v := e.(type) {
			case *goast.BasicLit:
				value, err := strconv.Unquote(v.Value)
				if err != nil {
					panic(err)
				}
				return value
			case *goast.BinaryExpr:
				if v.Op != token.ADD {
					panic("unsupported fixture operation")
				}
				return evaluate(v.X) + evaluate(v.Y)
			default:
				panic("unsupported fixture expression")
			}
		}
		rows := []string{}
		goast.Inspect(file, func(n goast.Node) bool {
			if kv, ok := n.(*goast.KeyValueExpr); ok {
				if key, ok := kv.Key.(*goast.Ident); ok && key.Name == "in" {
					rows = append(rows, evaluate(kv.Value))
				}
			}
			return true
		})
		json.NewEncoder(os.Stdout).Encode(rows)
		return
	}

	if len(os.Args) > 1 && os.Args[1] == "unicode" {
		tables := map[string][][2]int{}
		for name, predicate := range map[string]func(rune) bool{"letter_or_digit": func(r rune) bool { return unicode.IsLetter(r) || unicode.IsDigit(r) }, "digit": unicode.IsDigit, "print": unicode.IsPrint} {
			ranges := [][2]int{}
			start := -1
			for r := 0; r <= unicode.MaxRune+1; r++ {
				member := r <= unicode.MaxRune && predicate(rune(r))
				if member && start < 0 {
					start = r
				}
				if !member && start >= 0 {
					ranges = append(ranges, [2]int{start, r - 1})
					start = -1
				}
			}
			tables[name] = ranges
		}
		lower := [][2]int{}
		for r := 0; r <= unicode.MaxRune; r++ {
			if mapped := unicode.ToLower(rune(r)); mapped != rune(r) {
				lower = append(lower, [2]int{r, int(mapped)})
			}
		}
		fold := [][2]int{}
		for r := 0; r <= unicode.MaxRune; r++ {
			min := rune(r)
			for next := unicode.SimpleFold(rune(r)); next != rune(r); next = unicode.SimpleFold(next) {
				if next < min {
					min = next
				}
			}
			if min != rune(r) {
				fold = append(fold, [2]int{r, int(min)})
			}
		}
		json.NewEncoder(os.Stdout).Encode(map[string]any{"version": unicode.Version, "tables": tables, "lower": lower, "fold": fold})
		return
	}

	var requests []struct {
		Hex      string
		UTF16    bool
		Fail     int
		Mode     string
		Max      int
		Depth    int
		InKey    bool `json:"in_key"`
		Edge     bool
		Spread   bool
		Options  string
		Path     string
		Parts    []string
		Specials *string
	}
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	rows := []any{}
	for _, request := range requests {
		data, err := hex.DecodeString(request.Hex)
		if err != nil {
			panic(err)
		}
		if request.Mode == "ir_pattern" {
			rows = append(rows, patternOracle(data, request.Parts))
			continue
		}
		if request.Mode == "ir_resolve" {
			rows = append(rows, irResolveOracle(data, request.Parts, request.InKey))
			continue
		}
		if request.Mode == "ir_maps" {
			rows = append(rows, irMapsOracle(data))
			continue
		}
		if request.Mode == "ast_clone" {
			m, _ := d2parser.Parse(request.Path, bytes.NewReader(data), nil)
			rows = append(rows, astCloneOracle(m, cloneASTMap(m)))
			continue
		}
		if request.Mode == "ir_refs" {
			rows = append(rows, referenceOracle(data, request.Parts))
			continue
		}
		if request.Mode == "ir_tree" || request.Mode == "ir_ast" || request.Mode == "ir_copy" || request.Mode == "ir_import" || request.Mode == "ir_glob" {
			root := &d2ir.Map{}
			current := root
			var leaf *d2ir.Field
			for i, part := range request.Parts {
				b, err := hex.DecodeString(part)
				if err != nil {
					panic(err)
				}
				var name d2ast.String = d2ast.FlatUnquotedString(string(b))
				if request.InKey {
					name = &d2ast.SingleQuotedString{Value: string(b)}
				}
				switch n := name.(type) {
				case *d2ast.UnquotedString:
					n.Range.Path = request.Path
				case *d2ast.SingleQuotedString:
					n.Range.Path = request.Path
				}
				leaf = &d2ir.Field{Name: name}
				current.Fields = append(current.Fields, leaf)
				if i < len(request.Parts)-1 {
					next := &d2ir.Map{}
					leaf.Composite = next
					current = next
				}
			}
			scalar := func(value string) *d2ir.Scalar {
				if (request.Mode == "ir_ast" || request.Mode == "ir_copy" || request.Mode == "ir_import" || request.Mode == "ir_glob") && len(data) > 0 {
					v, err := d2parser.ParseValue(string(data))
					if err != nil {
						panic(err)
					}
					return &d2ir.Scalar{Value: v.(d2ast.Scalar)}
				}
				return &d2ir.Scalar{Value: d2ast.FlatUnquotedString(value)}
			}
			if leaf != nil {
				leaf.Primary_ = scalar("leaf")
				leaf.Composite = &d2ir.Array{Values: []d2ir.Value{scalar("item"), &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("inside"), Primary_: scalar("nested")}}}}}
			}
			root.Edges = []*d2ir.Edge{{ID: &d2ir.EdgeID{}, Primary_: scalar("edge"), Map_: &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("vars"), Composite: &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("end"), Primary_: scalar("value")}}}}}}}}
			if (request.Mode == "ir_ast" || request.Mode == "ir_copy" || request.Mode == "ir_import" || request.Mode == "ir_glob") && request.Options != "" {
				key, err := d2parser.ParseMapKey(request.Options)
				if err != nil {
					panic(err)
				}
				root.Edges[0].ID = d2ir.NewEdgeIDs(key)[0]
			}
			if request.Mode == "ir_glob" {
				m := root
				for {
					var next *d2ir.Map
					if len(m.Fields) > 0 {
						next = m.Fields[0].Map()
					}
					m.Fields = append(m.Fields, &d2ir.Field{Name: d2ast.FlatUnquotedString("peer"), Primary_: scalar("sibling")})
					if next == nil {
						break
					}
					m = next
				}
			}
			root = root.Copy(nil).(*d2ir.Map)
			nodes := []d2ir.Node{root.Parent()}
			var visit func(d2ir.Node)
			visit = func(n d2ir.Node) {
				nodes = append(nodes, n)
				switch v := n.(type) {
				case *d2ir.Map:
					for _, f := range v.Fields {
						visit(f)
					}
					for _, e := range v.Edges {
						visit(e)
					}
				case *d2ir.Field:
					if v.Primary_ != nil {
						visit(v.Primary_)
					}
					if v.Composite != nil {
						visit(v.Composite)
					}
				case *d2ir.Edge:
					if v.Primary_ != nil {
						visit(v.Primary_)
					}
					if v.Map_ != nil {
						visit(v.Map_)
					}
				case *d2ir.Array:
					for _, v := range v.Values {
						visit(v)
					}
				}
			}
			visit(root)
			if request.Mode == "ir_glob" {
				rows = append(rows, globOracle(nodes))
				continue
			}
			if request.Mode == "ir_import" {
				rows = append(rows, importContextOracle(nodes, root, request.Path))
				continue
			}
			if request.Mode == "ir_copy" {
				rows = append(rows, irCopyOracle(nodes, root))
				continue
			}
			if request.Mode == "ir_ast" {
				snapshots := []any{}
				for _, n := range nodes {
					astNode := n.AST()
					scalarBytes := []string{}
					var collect func(d2ast.Node)
					collect = func(n d2ast.Node) {
						if s, ok := n.(d2ast.Scalar); ok {
							scalarBytes = append(scalarBytes, hex.EncodeToString([]byte(s.ScalarString())))
						}
						for _, child := range n.Children() {
							collect(child)
						}
					}
					collect(astNode)
					before, err := json.Marshal(astNode)
					if err != nil {
						panic(err)
					}
					formatted := d2format.Format(astNode)
					after, err := json.Marshal(astNode)
					if err != nil {
						panic(err)
					}
					var scalarAlias, nameAlias, edgeID any
					switch v := n.(type) {
					case *d2ir.Scalar:
						scalarAlias = astNode == v.Value
					case *d2ir.Field:
						nameAlias = astNode.(*d2ast.Key).Key.Path[0].Unbox() == v.Name
					case *d2ir.Edge:
						edgeID = hex.EncodeToString([]byte(v.IDString().ScalarString()))
					}
					snapshots = append(snapshots, map[string]any{"scalar_bytes": scalarBytes, "before": json.RawMessage(before), "after": json.RawMessage(after), "hex": hex.EncodeToString([]byte(formatted)), "scalar_alias": scalarAlias, "name_alias": nameAlias, "edge_id": edgeID})
				}
				rows = append(rows, snapshots)
				continue
			}
			indices := map[d2ir.Node]int{}
			for i, n := range nodes {
				indices[n] = i
			}
			index := func(n d2ir.Node) any {
				if i, ok := indices[n]; ok {
					return i
				}
				return nil
			}
			summaries := []any{}
			for _, n := range nodes {
				kind := ""
				isRoot := false
				switch v := n.(type) {
				case *d2ir.Map:
					kind = "map"
					isRoot = v.Root()
				case *d2ir.Field:
					kind = "field"
					isRoot = v.Root()
				case *d2ir.Edge:
					kind = "edge"
				case *d2ir.Scalar:
					kind = "scalar"
				case *d2ir.Array:
					kind = "array"
				}
				var rootMap any
				fields, edges := 0, 0
				if m := n.Map(); m != nil {
					rootMap = index(d2ir.RootMap(m))
					fields = m.FieldCountRecursive()
					edges = m.EdgeCountRecursive()
				}
				summaries = append(summaries, map[string]any{"kind": kind, "parent": index(n.Parent()), "primary": index(n.Primary()), "map": index(n.Map()), "parent_map": index(d2ir.ParentMap(n)), "parent_field": index(d2ir.ParentField(n)), "parent_edge": index(d2ir.ParentEdge(n)), "parent_board": index(d2ir.ParentBoard(n)), "board": d2ir.NodeBoardKind(n), "is_var": d2ir.IsVar(n), "root": isRoot, "root_map": rootMap, "fields": fields, "edges": edges})
			}
			rows = append(rows, map[string]any{"nodes": summaries, "root_reference": root.Parent().(*d2ir.Field).References[0].Context_.ScopeMap == root})
			continue
		}
		if request.Mode == "ir_edges" {
			key, err := d2parser.ParseMapKey(string(data))
			if err != nil {
				panic(err)
			}
			ids := d2ir.NewEdgeIDs(key)
			copies := []*d2ir.EdgeID{}
			aliases := [][4]bool{}
			for _, id := range ids {
				copy := id.Copy()
				copies = append(copies, copy)
				aliases = append(aliases, [4]bool{len(id.SrcPath) > 0 && id.SrcPath[0] == copy.SrcPath[0], len(id.SrcPath) > 0 && &id.SrcPath[0] == &copy.SrcPath[0], id.Index != nil && id.Index == copy.Index, id.Index != nil && key.EdgeIndex != nil && id.Index == key.EdgeIndex.Int})
			}
			var matches [][]bool
			for _, id := range ids {
				row := []bool{}
				for _, part := range request.Parts {
					b, err := hex.DecodeString(part)
					if err != nil {
						panic(err)
					}
					k, err := d2parser.ParseMapKey(string(b))
					if err != nil {
						panic(err)
					}
					for _, other := range d2ir.NewEdgeIDs(k) {
						row = append(row, id.Match(other))
					}
				}
				matches = append(matches, row)
			}
			rows = append(rows, map[string]any{"ids": ids, "copies": copies, "aliases": aliases, "matches": matches})
			continue
		}
		if request.Mode == "ir_edge_match" {
			values := []d2ast.String{}
			for _, part := range request.Parts {
				b, err := hex.DecodeString(part)
				if err != nil {
					panic(err)
				}
				values = append(values, d2ast.FlatUnquotedString(string(b)))
			}
			a := &d2ir.EdgeID{SrcPath: []d2ast.String{values[0]}, DstPath: []d2ast.String{values[1]}}
			b := &d2ir.EdgeID{SrcPath: []d2ast.String{values[2]}, DstPath: []d2ast.String{values[3]}}
			rows = append(rows, a.Match(b))
			continue
		}
		if request.Mode == "format_keypath" {
			previous := d2ast.UnquotedKeySpecials
			if request.Specials != nil {
				d2ast.UnquotedKeySpecials = *request.Specials
			}
			var values []d2ast.String
			kp := &d2ast.KeyPath{}
			for index, part := range request.Parts {
				bytes, err := hex.DecodeString(part)
				if err != nil {
					panic(err)
				}
				var value d2ast.String
				switch index % 3 {
				case 0:
					value = d2ast.FlatUnquotedString(string(bytes))
				case 1:
					value = d2ast.FlatDoubleQuotedString(string(bytes))
				case 2:
					value = &d2ast.SingleQuotedString{Value: string(bytes)}
				}
				values = append(values, value)
				kp.Path = append(kp.Path, d2ast.MakeValueBox(value).StringBox())
			}
			result := d2format.FormatKeyPath(values)
			var parts []string
			for _, part := range d2format.KeyPath(kp) {
				parts = append(parts, hex.EncodeToString([]byte(part)))
			}
			rows = append(rows, map[string]any{"hex": hex.EncodeToString([]byte(result)), "parts": parts})
			d2ast.UnquotedKeySpecials = previous
			continue
		}
		if strings.HasPrefix(request.Mode, "format_") {
			previousKey, previousValue := d2ast.UnquotedKeySpecials, d2ast.UnquotedValueSpecials
			if request.Specials != nil {
				if request.InKey {
					d2ast.UnquotedKeySpecials = *request.Specials
				} else {
					d2ast.UnquotedValueSpecials = *request.Specials
				}
			}
			var value d2ast.Value
			switch request.Mode {
			case "format_single":
				value = &d2ast.SingleQuotedString{Value: string(data)}
			case "format_double":
				value = d2ast.FlatDoubleQuotedString(string(data))
			case "format_unquoted":
				value = d2ast.FlatUnquotedString(string(data))
			case "format_block":
				value = &d2ast.BlockString{Value: string(data), Quote: request.Options, Tag: "md"}
			}
			var node d2ast.Node = value
			if request.InKey {
				node = &d2ast.KeyPath{Path: []*d2ast.StringBox{d2ast.MakeValueBox(value).StringBox()}}
			}
			formatted := d2format.Format(node)
			rows = append(rows, map[string]any{"hex": hex.EncodeToString([]byte(formatted)), "value": d2ast.MakeValueBox(value)})
			d2ast.UnquotedKeySpecials, d2ast.UnquotedValueSpecials = previousKey, previousValue
			continue
		}
		if request.Mode == "format" {
			m, err := d2parser.Parse(request.Path, bytes.NewReader(data), &d2parser.ParseOptions{UTF16Pos: request.UTF16})
			formatted := d2format.Format(m)
			display := ""
			if err != nil {
				display = err.Error()
			}
			rows = append(rows, map[string]any{"hex": hex.EncodeToString([]byte(formatted)), "map": m, "error": display})
			continue
		}
		if request.Mode == "public_key" || request.Mode == "public_mapkey" || request.Mode == "public_value" {
			var node any
			var helperErr error
			switch request.Mode {
			case "public_key":
				node, helperErr = d2parser.ParseKey(string(data))
			case "public_mapkey":
				node, helperErr = d2parser.ParseMapKey(string(data))
			case "public_value":
				var value d2ast.Value
				value, helperErr = d2parser.ParseValue(string(data))
				if value != nil {
					node = d2ast.MakeValueBox(value)
				}
			}
			display := ""
			var cause *d2parser.ParseError
			if helperErr != nil {
				display = helperErr.Error()
				errors.As(helperErr, &cause)
			}
			rows = append(rows, map[string]any{"node": node, "error": display, "cause": cause})
			continue
		}
		if request.Mode == "public_parse" {
			var initial *d2parser.ParseError
			if request.Options == "empty" {
				initial = &d2parser.ParseError{Errors: []d2ast.Error{}}
			}
			if request.Options == "prior" {
				initial = &d2parser.ParseError{Errors: []d2ast.Error{{Message: "existing diagnostic"}}}
			}
			var opts *d2parser.ParseOptions
			if request.Options != "none" {
				opts = &d2parser.ParseOptions{UTF16Pos: request.UTF16, ParseError: initial}
			}
			m, err := d2parser.Parse(request.Path, bytes.NewReader(data), opts)
			display := ""
			if err != nil {
				display = err.Error()
			}
			rows = append(rows, map[string]any{"map": m, "error": display, "structured": err, "accumulator": initial})
			continue
		}
		if request.Mode == "number" {
			value, ok := new(big.Rat).SetString(string(data))
			var rational any
			if ok {
				rational = value
			}
			rows = append(rows, map[string]any{"accepted": ok, "rational": rational})
			continue
		}
		if request.Mode == "indent" {
			a, b := splitLeadingIndent(string(data), request.Max)
			rows = append(rows, map[string]any{"indent": hex.EncodeToString([]byte(a)), "remainder": hex.EncodeToString([]byte(b)), "trim": hex.EncodeToString([]byte(trimIndent(string(data), strings.Repeat(" ", request.Max+1)))), "common": hex.EncodeToString([]byte(trimCommonIndent(string(data)))), "last": hex.EncodeToString([]byte(trimSpaceAfterLastNewline(string(data))))})
			continue
		}
		p := &parser{path: "input.d2", reader: &oracleReader{reader: strings.NewReader(string(data)), fail: request.Fail}, utf16Pos: request.UTF16, err: &ParseError{}}
		p.depth = request.Depth
		if request.Mode == "key" || request.Mode == "string" || request.Mode == "unquoted" || request.Mode == "double" || request.Mode == "sub" || request.Mode == "import" {
			p.inEdgeGroup = request.Edge
			var node any
			var patternHex []string
			switch request.Mode {
			case "key":
				node = p.parseKey()
			case "string":
				node = p.parseString(request.InKey)
			case "unquoted":
				u := p.parseUnquotedString(request.InKey)
				node = u
				if u != nil {
					for _, part := range u.Pattern {
						patternHex = append(patternHex, hex.EncodeToString([]byte(part)))
					}
				}
			case "double":
				r, eof := p.read()
				if eof || r != '"' {
					panic("expected double quote")
				}
				node = p.parseDoubleQuotedString(request.InKey)
			case "import":
				count := 1
				if request.Spread {
					count = 4
				}
				for i := 0; i < count; i++ {
					p.read()
				}
				node = p.parseImport(request.Spread)
			case "sub":
				count := 1
				if request.Spread {
					count = 4
				}
				for i := 0; i < count; i++ {
					p.read()
				}
				node = p.parseSubstitution(request.Spread)
			}
			tail := ""
			for {
				r, eof := p.read()
				if eof {
					break
				}
				tail += string(r)
			}
			row := map[string]any{"node": node, "errors": p.err, "display": p.err.Error(), "position": p.pos, "tail": tail}
			if request.Mode == "unquoted" {
				row["pattern_hex"] = patternHex
			}
			rows = append(rows, row)
			continue
		}

		if request.Mode == "file" || request.Mode == "map" || request.Mode == "array" || request.Mode == "value" || request.Mode == "mapkey" || request.Mode == "index" {
			p.inEdgeGroup = request.Edge
			var node any
			switch request.Mode {
			case "file":
				node = p.parseMap(true)
			case "map":
				p.read()
				node = p.parseMap(false)
			case "array":
				p.read()
				node = p.parseArray()
			case "value":
				node = p.parseValue()
			case "mapkey":
				node = p.parseMapKey()
			case "index":
				p.read()
				node = p.parseEdgeIndex()
			}
			tail := ""
			for {
				r, eof := p.read()
				if eof {
					break
				}
				tail += string(r)
			}
			rows = append(rows, map[string]any{"node": node, "errors": p.err, "display": p.err.Error(), "position": p.pos, "tail": tail, "depth": p.depth, "edge_group": p.inEdgeGroup})
			continue
		}

		r, eof := p.read()
		if eof || (request.Mode != "comment" && request.Mode != "block" && request.Mode != "block_comment" && r != '\'') || (request.Mode == "comment" && r != '#') || (request.Mode == "block" && r != '|') || (request.Mode == "block_comment" && r != '"') {
			panic("expected opening quote")
		}
		var node any
		var scalarHex *string
		if request.Mode == "block_comment" {
			text, eof := p.peekn(2)
			if eof || text != `""` {
				panic("expected triple quote")
			}
			p.commit()
			b := p.parseBlockComment()
			node = b
			h := hex.EncodeToString([]byte(b.Value))
			scalarHex = &h
		} else if request.Mode == "block" {
			b := p.parseBlockString()
			node = b
			h := hex.EncodeToString([]byte(b.ScalarString()))
			scalarHex = &h
		} else if request.Mode == "comment" {
			node = p.parseComment()
		} else {
			node = p.parseSingleQuotedString()
		}
		tail := ""
		for {
			r, eof := p.read()
			if eof {
				break
			}
			tail += string(r)
		}
		row := map[string]any{"node": node, "errors": p.err, "display": p.err.Error(), "position": p.pos, "tail": tail}
		if scalarHex != nil {
			if request.Mode == "block_comment" {
				row["value_hex"] = *scalarHex
			} else {
				row["scalar_hex"] = *scalarHex
			}
			row["depth"] = p.depth
		}
		rows = append(rows, row)
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}

func referenceOracle(data []byte, peers []string) any {
	key, err := d2parser.ParseMapKey(string(data))
	if err != nil {
		panic(err)
	}
	scope := &d2ast.Map{}
	scopeMap := &d2ir.Map{}
	base := &d2ir.RefContext{Key: key, Scope: scope, ScopeAST: scope, ScopeMap: scopeMap}
	if len(key.Edges) > 0 {
		base.Edge = key.Edges[0]
	}
	contexts := []*d2ir.RefContext{base, base.Copy()}
	changed := base.Copy()
	changed.Edge = &d2ast.Edge{}
	contexts = append(contexts, changed)
	changed = base.Copy()
	changed.ScopeMap = &d2ir.Map{}
	contexts = append(contexts, changed)
	changed = base.Copy()
	changed.Scope = &d2ast.Map{}
	contexts = append(contexts, changed)
	changed = base.Copy()
	changed.ScopeAST = &d2ast.Map{}
	contexts = append(contexts, changed)
	keyCopy := *key
	changed = base.Copy()
	changed.Key = &keyCopy
	contexts = append(contexts, changed)
	changed = base.Copy()
	changed.Key, err = d2parser.ParseMapKey(string(data))
	if err != nil {
		panic(err)
	}
	contexts = append(contexts, changed)
	equal, identity := [][]bool{}, [][]bool{}
	indices := []int{}
	for _, a := range contexts {
		er, ir := []bool{}, []bool{}
		for _, b := range contexts {
			er = append(er, a.Equal(b))
			ir = append(ir, sameRefContext(a, b))
		}
		equal = append(equal, er)
		identity = append(identity, ir)
		indices = append(indices, a.EdgeIndex())
	}
	peerEqual := []bool{}
	for _, p := range peers {
		b, err := hex.DecodeString(p)
		if err != nil {
			panic(err)
		}
		k, err := d2parser.ParseMapKey(string(b))
		if err != nil {
			panic(err)
		}
		c := base.Copy()
		c.Key = k
		peerEqual = append(peerEqual, base.Equal(c))
	}
	refs := []*d2ir.FieldReference{}
	add := func(path *d2ast.KeyPath, edge *d2ast.Edge) {
		if path == nil {
			return
		}
		c := base.Copy()
		c.Edge = edge
		for _, part := range path.Path {
			refs = append(refs, &d2ir.FieldReference{String: part.Unbox(), KeyPath: path, Context_: c})
		}
	}
	add(key.Key, nil)
	for _, edge := range key.Edges {
		add(edge.Src, edge)
		add(edge.Dst, edge)
	}
	add(key.EdgeKey, base.Edge)
	fieldRows := []any{}
	for _, r := range refs {
		var dest any
		if r.InEdge() {
			dest = r.EdgeDest()
		}
		fieldRows = append(fieldRows, map[string]any{"primary": r.Primary(), "path_index": r.KeyPathIndex(), "in_edge": r.InEdge(), "edge_dest": dest, "ast_identity": r.AST() == r.String, "edge_index": r.Context_.EdgeIndex()})
	}
	first := refs[len(refs)-1]
	normal := *first
	lazy := *first
	lazy.DueToGlob_ = true
	lazy.DueToLazyGlob_ = true
	copyContext := lazy
	copyContext.Context_ = lazy.Context_.Copy()
	otherMap := lazy
	otherMap.Context_ = lazy.Context_.Copy()
	otherMap.Context_.ScopeMap = &d2ir.Map{}
	otherPath := lazy
	otherPath.KeyPath = lazy.KeyPath.Copy()
	last := *first
	last.DueToGlob_ = true
	f := &d2ir.Field{}
	fieldCounts := []int{}
	fieldPrimarySteps := []any{}
	fieldIndex := func(ref d2ir.Reference) any {
		for i, r := range f.References {
			if ref == r {
				return i
			}
		}
		return nil
	}
	for _, r := range []*d2ir.FieldReference{&normal, &normal, &lazy, &lazy, &copyContext, &otherMap, &otherPath, &last} {
		appendFieldRef(f, r)
		fieldCounts = append(fieldCounts, len(f.References))
		fieldPrimarySteps = append(fieldPrimarySteps, fieldIndex(f.LastPrimaryRef()))
	}
	enormal := &d2ir.EdgeReference{Context_: base}
	elazy := *enormal
	elazy.DueToGlob_ = true
	elazy.DueToLazyGlob_ = true
	ecopy := elazy
	ecopy.Context_ = base.Copy()
	emap := elazy
	emap.Context_ = base.Copy()
	emap.Context_.ScopeMap = &d2ir.Map{}
	elast := *enormal
	elast.DueToGlob_ = true
	e := &d2ir.Edge{}
	edgeCounts := []int{}
	edgePrimarySteps := []any{}
	edgeIndex := func(ref d2ir.Reference) any {
		for i, r := range e.References {
			if ref == r {
				return i
			}
		}
		return nil
	}
	for _, r := range []*d2ir.EdgeReference{enormal, enormal, &elazy, &elazy, &ecopy, &emap, &elast} {
		appendEdgeRef(e, r)
		edgeCounts = append(edgeCounts, len(e.References))
		edgePrimarySteps = append(edgePrimarySteps, edgeIndex(e.LastPrimaryRef()))
	}
	rootRef := &d2ir.FieldReference{Context_: base}
	nestedEdge := &d2ir.Edge{References: []*d2ir.EdgeReference{enormal}, Primary_: &d2ir.Scalar{Value: d2ast.FlatUnquotedString("value")}, Map_: &d2ir.Map{}}
	owner := &d2ir.Field{Name: d2ast.FlatUnquotedString("owner"), References: []*d2ir.FieldReference{first}, Composite: &d2ir.Map{Edges: []*d2ir.Edge{nestedEdge}}}
	root := (&d2ir.Map{Fields: []*d2ir.Field{owner}}).Copy(nil).(*d2ir.Map)
	owner = root.Fields[0]
	nestedEdge = owner.Map().Edges[0]
	parentSelection := []any{}
	for _, n := range []d2ir.Node{owner, owner.Map(), nestedEdge, nestedEdge.Primary_, nestedEdge.Map_} {
		ref, primary := n.LastRef(), n.LastPrimaryRef()
		var primaryClaim any
		if primary != nil {
			primaryClaim = primary.Primary()
		}
		parentSelection = append(parentSelection, map[string]any{"last_field": ref == first, "last_edge": ref == enormal, "primary_field": primary == first, "primary_edge": primary == enormal, "primary_key": n.LastPrimaryKey() == key, "primary_claim": primaryClaim})
	}
	return map[string]any{"parent_selection": parentSelection, "field_primary_steps": fieldPrimarySteps, "edge_primary_steps": edgePrimarySteps, "fields": fieldRows, "contexts_equal": equal, "contexts_identity": identity, "context_edge_indices": indices, "peer_equal": peerEqual, "field_counts": fieldCounts, "field_last": fieldIndex(f.LastRef()), "field_last_primary": fieldIndex(f.LastPrimaryRef()), "field_primary_key": f.LastPrimaryKey() == key, "edge_counts": edgeCounts, "edge_last": edgeIndex(e.LastRef()), "edge_last_primary": edgeIndex(e.LastPrimaryRef()), "edge_primary_key": e.LastPrimaryKey() == key, "edge_reference_primary": enormal.Primary(), "edge_ast_identity": enormal.AST() == base.Edge, "root_ast_identity": rootRef.AST() == scope}
}

func astCloneOracle(src, dst d2ast.Node) any {
	before, _ := json.Marshal(dst)
	sourceBefore, _ := json.Marshal(src)
	aliases := []any{}
	var pair func(d2ast.Node, d2ast.Node)
	pair = func(a, b d2ast.Node) {
		row := map[string]any{"node": a == b}
		if v, ok := a.(d2ast.Scalar); ok {
			row["source_scalar_bytes"] = hex.EncodeToString([]byte(v.ScalarString()))
			row["scalar_bytes"] = hex.EncodeToString([]byte(b.(d2ast.Scalar).ScalarString()))
		}
		var ai, bi []d2ast.InterpolationBox
		switch a := a.(type) {
		case *d2ast.UnquotedString:
			ai = a.Value
			bi = b.(*d2ast.UnquotedString).Value
			bb := b.(*d2ast.UnquotedString)
			if len(a.Pattern) > 0 {
				row["pattern"] = &a.Pattern[0] == &bb.Pattern[0]
			}
		case *d2ast.DoubleQuotedString:
			ai = a.Value
			bi = b.(*d2ast.DoubleQuotedString).Value
		case *d2ast.Number:
			if a.Value != nil {
				row["number"] = a.Value == b.(*d2ast.Number).Value
			}
		case *d2ast.EdgeIndex:
			if a.Int != nil {
				row["index"] = a.Int == b.(*d2ast.EdgeIndex).Int
			}
		}
		if ai != nil {
			parts := []any{}
			for i, x := range ai {
				v := map[string]any{}
				if x.String != nil {
					v["string"] = x.String == bi[i].String
				}
				if x.StringRaw != nil {
					v["raw"] = x.StringRaw == bi[i].StringRaw
				}
				parts = append(parts, v)
			}
			row["parts"] = parts
		}
		aliases = append(aliases, row)
		aa, bb := a.Children(), b.Children()
		if len(aa) != len(bb) {
			panic("copy changed children")
		}
		for i := range aa {
			pair(aa[i], bb[i])
		}
	}
	pair(src, dst)
	var mutate func(d2ast.Node)
	mutate = func(n d2ast.Node) {
		for _, c := range n.Children() {
			mutate(c)
		}
		switch n := n.(type) {
		case d2ast.String:
			n.SetString("changed")
		case *d2ast.Number:
			if n.Value != nil {
				n.Value.SetInt64(73)
			}
		case *d2ast.EdgeIndex:
			if n.Int != nil {
				*n.Int = 41
			}
		}
	}
	mutate(dst)
	after, _ := json.Marshal(dst)
	sourceAfter, _ := json.Marshal(src)
	return map[string]any{"before": json.RawMessage(before), "after": json.RawMessage(after), "source_before": json.RawMessage(sourceBefore), "source_after": json.RawMessage(sourceAfter), "aliases": aliases}
}

func irCopyOracle(nodes []d2ir.Node, root *d2ir.Map) any {
	marker := d2ast.FlatUnquotedString("import")
	for _, n := range nodes {
		switch n := n.(type) {
		case *d2ir.Map:
			n.SetImportAST(marker)
			markStructureChanged(n)
		case *d2ir.Field:
			n.SetImportAST(marker)
			n.References = append(n.References, &d2ir.FieldReference{String: n.Name, Context_: &d2ir.RefContext{ScopeMap: root}})
		case *d2ir.Edge:
			n.SetImportAST(marker)
			n.References = []*d2ir.EdgeReference{{Context_: &d2ir.RefContext{ScopeMap: root}}}
		}
	}
	observations := []any{}
	for _, src := range nodes {
		for _, parent := range []d2ir.Node{nil, root} {
			dst := src.Copy(parent)
			pairs := []any{}
			var pair func(d2ir.Node, d2ir.Node, d2ir.Node)
			pair = func(a, b, expectedParent d2ir.Node) {
				parentOK := b.Parent() == expectedParent
				if expectedParent == nil {
					if m, ok := b.(*d2ir.Map); ok {
						f, ok := m.Parent().(*d2ir.Field)
						parentOK = ok && f.Parent() == nil && f.References[0].Context_.ScopeMap == m
					}
				}
				row := map[string]any{"fresh": a != b, "parent_ok": parentOK}
				switch a := a.(type) {
				case *d2ir.Scalar:
					bb := b.(*d2ir.Scalar)
					row["kind"] = "scalar"
					row["value_alias"] = a.Value == bb.Value
				case *d2ir.Field:
					bb := b.(*d2ir.Field)
					row["kind"] = "field"
					row["name_alias"] = a.Name == bb.Name
					row["import_alias"] = a.ImportAST() == bb.ImportAST()
					row["refs_nil"] = bb.References == nil
					if len(a.References) > 0 {
						row["ref_alias"] = a.References[0] == bb.References[0]
					}
				case *d2ir.Edge:
					bb := b.(*d2ir.Edge)
					row["kind"] = "edge"
					row["import_alias"] = a.ImportAST() == bb.ImportAST()
					row["refs_nil"] = bb.References == nil
					if len(a.References) > 0 {
						row["ref_alias"] = a.References[0] == bb.References[0]
					}
					if a.ID != nil {
						row["id_alias"] = a.ID == bb.ID
						if a.ID.Index != nil {
							row["index_alias"] = a.ID.Index == bb.ID.Index
						}
						endpoints := []bool{}
						for i, n := range a.ID.SrcPath {
							endpoints = append(endpoints, n == bb.ID.SrcPath[i])
						}
						for i, n := range a.ID.DstPath {
							endpoints = append(endpoints, n == bb.ID.DstPath[i])
						}
						row["endpoints"] = endpoints
					}
				case *d2ir.Array:
					row["kind"] = "array"
					row["values_nil"] = b.(*d2ir.Array).Values == nil
				case *d2ir.Map:
					bb := b.(*d2ir.Map)
					row["kind"] = "map"
					row["fields_nil"] = bb.Fields == nil
					row["edges_nil"] = bb.Edges == nil
					row["version"] = reflect.ValueOf(bb).Elem().FieldByName("structureVersion").Uint()
					row["import_alias"] = a.ImportAST() == bb.ImportAST()
				}
				pairs = append(pairs, row)
				ac, bc := irChildren(a), irChildren(b)
				if len(ac) != len(bc) {
					panic("copy changed IR children")
				}
				for i := range ac {
					pair(ac[i], bc[i], b)
				}
			}
			pair(src, dst, parent)
			observations = append(observations, map[string]any{"pairs": pairs, "ast": astCloneOracle(src.AST(), dst.AST())})
		}
	}
	return observations
}
func irChildren(n d2ir.Node) []d2ir.Node {
	out := []d2ir.Node{}
	switch n := n.(type) {
	case *d2ir.Map:
		for _, f := range n.Fields {
			out = append(out, f)
		}
		for _, e := range n.Edges {
			out = append(out, e)
		}
	case *d2ir.Field:
		if n.Primary_ != nil {
			out = append(out, n.Primary_)
		}
		if n.Composite != nil {
			out = append(out, n.Composite)
		}
	case *d2ir.Edge:
		if n.Primary_ != nil {
			out = append(out, n.Primary_)
		}
		if n.Map_ != nil {
			out = append(out, n.Map_)
		}
	case *d2ir.Array:
		for _, v := range n.Values {
			out = append(out, v)
		}
	}
	return out
}

type mapName struct {
	Hex    string
	Quoted bool
	Nil    bool
}

func (n mapName) ast() d2ast.String {
	if n.Nil {
		return nil
	}
	b, err := hex.DecodeString(n.Hex)
	if err != nil {
		panic(err)
	}
	if n.Quoted {
		return &d2ast.SingleQuotedString{Value: string(b)}
	}
	return d2ast.FlatUnquotedString(string(b))
}
func mapEdge(key string) *d2ir.EdgeID {
	k, err := d2parser.ParseMapKey(key)
	if err != nil {
		panic(err)
	}
	return d2ir.NewEdgeIDs(k)[0]
}
func irMapsOracle(data []byte) any {
	var input struct {
		Operations []struct {
			Op             string
			Map, Index, ID int
			Name           mapName
			Key            string
			Child          bool
			Path           []string
		}
		Queries  [][]mapName
		Edges    []string
		Detailed bool
	}
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	root := (&d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("scope"), Composite: &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("dive"), Composite: &d2ir.Map{}}}}}}}).Copy(nil).(*d2ir.Map)
	maps := []*d2ir.Map{root, root.Fields[0].Map(), root.Fields[0].Map().Fields[0].Map()}
	fields := []*d2ir.Field{root.Fields[0], maps[1].Fields[0]}
	edges := []*d2ir.Edge{}
	contexts := map[int]*d2ir.RefContext{}
	var created any
	fieldRefs := map[*d2ir.FieldReference]int{}
	edgeRefs := map[*d2ir.EdgeReference]int{}
	var register func(*d2ir.Map)
	var registerValue func(d2ir.Node)
	registerValue = func(n d2ir.Node) {
		switch n := n.(type) {
		case *d2ir.Map:
			register(n)
		case *d2ir.Array:
			for _, v := range n.Values {
				registerValue(v)
			}
		}
	}
	register = func(m *d2ir.Map) {
		seen := false
		for _, v := range maps {
			seen = seen || v == m
		}
		if !seen {
			maps = append(maps, m)
		}
		for _, f := range m.Fields {
			seen := false
			for _, v := range fields {
				seen = seen || v == f
			}
			if !seen {
				fields = append(fields, f)
			}
			if f.Composite != nil {
				registerValue(f.Composite)
			}
		}
		for _, e := range m.Edges {
			seen := false
			for _, v := range edges {
				seen = seen || v == e
			}
			if !seen {
				edges = append(edges, e)
			}
			if e.Map() != nil {
				register(e.Map())
			}
		}
	}
	fieldID := func(f *d2ir.Field) any {
		for i, v := range fields {
			if f == v {
				return i
			}
		}
		return nil
	}
	edgeID := func(e *d2ir.Edge) any {
		for i, v := range edges {
			if e == v {
				return i
			}
		}
		return nil
	}
	snapshots := []any{}
	snapshot := func(removed any) {
		rows := []any{}
		for _, m := range maps {
			fs, es := []any{}, []any{}
			for _, f := range m.Fields {
				fs = append(fs, fieldID(f))
			}
			for _, e := range m.Edges {
				es = append(es, edgeID(e))
			}
			linear, indexed := []any{}, []any{}
			for _, q := range input.Queries {
				path := []d2ast.String{}
				for _, n := range q {
					path = append(path, n.ast())
				}
				linear = append(linear, fieldID(m.GetField(path...)))
				indexed = append(indexed, fieldID(indexedField(m, path...)))
			}
			eq := []any{}
			for _, key := range input.Edges {
				found := []any{}
				for _, e := range indexedEdges(m, mapEdge(key)) {
					found = append(found, edgeID(e))
				}
				eq = append(eq, found)
			}
			row := map[string]any{"fields": fs, "edges": es, "version": reflect.ValueOf(m).Elem().FieldByName("structureVersion").Uint(), "linear": linear, "indexed": indexed, "edge_queries": eq}
			if input.Detailed {
				row["ast"] = m.AST()
				frs, ers := []any{}, []any{}
				for _, f := range m.Fields {
					var ids []int
					if f.References != nil {
						ids = []int{}
					}
					for _, ref := range f.References {
						label, ok := fieldRefs[ref]
						if !ok {
							label = len(fieldRefs)
							fieldRefs[ref] = label
						}
						ids = append(ids, label)
					}
					frs = append(frs, ids)
				}
				for _, e := range m.Edges {
					var ids []int
					if e.References != nil {
						ids = []int{}
					}
					for _, ref := range e.References {
						label, ok := edgeRefs[ref]
						if !ok {
							label = len(edgeRefs)
							edgeRefs[ref] = label
						}
						ids = append(ids, label)
					}
					ers = append(ers, ids)
				}
				row["field_refs"] = frs
				row["edge_refs"] = ers
				linearEdges := []any{}
				for _, key := range input.Edges {
					var ids []any
					for _, e := range m.GetEdges(mapEdge(key), nil, nil) {
						ids = append(ids, edgeID(e))
					}
					linearEdges = append(linearEdges, ids)
				}
				row["linear_edges"] = linearEdges
			}
			rows = append(rows, row)
		}
		entry := map[string]any{"maps": rows, "removed": removed}
		if input.Detailed {
			entry["created"] = created
		}
		snapshots = append(snapshots, entry)
	}
	snapshot(nil)
	for _, op := range input.Operations {
		m := maps[op.Map]
		var removed any
		created = nil
		switch op.Op {
		case "append", "insert", "direct_append":
			f := &d2ir.Field{Name: op.Name.ast()}
			if op.Child {
				f.Composite = &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("target")}}}
			}
			f = f.Copy(m).(*d2ir.Field)
			fields = append(fields, f)
			if op.Child {
				maps = append(maps, f.Map())
				fields = append(fields, f.Map().Fields[0])
			}
			if op.Op == "append" {
				appendMapField(m, f)
			} else if op.Op == "insert" {
				insertMapField(m, op.Index, f)
			} else {
				m.Fields = append(m.Fields, f)
			}
		case "remove":
			removed = fieldID(removeMapField(m, op.Index))
		case "rename":
			fields[op.ID].Name = op.Name.ast()
		case "swap":
			m.Fields[0], m.Fields[len(m.Fields)-1] = m.Fields[len(m.Fields)-1], m.Fields[0]
		case "append_edge":
			var id *d2ir.EdgeID
			if op.Key != "" {
				id = mapEdge(op.Key)
			}
			e := (&d2ir.Edge{ID: id}).Copy(m).(*d2ir.Edge)
			edges = append(edges, e)
			appendMapEdge(m, e)
		case "remove_edge":
			removed = edgeID(removeMapEdge(m, op.Index))
		case "reedge":
			edges[op.ID].ID = mapEdge(op.Key)
		case "swap_edges":
			m.Edges[0], m.Edges[len(m.Edges)-1] = m.Edges[len(m.Edges)-1], m.Edges[0]
		case "mark":
			markStructureChanged(m)
		case "ref_field", "ref_edge":
			ctx := contexts[op.Index]
			if ctx == nil {
				ctx = &d2ir.RefContext{ScopeMap: root}
				contexts[op.Index] = ctx
			}
			if op.Name.Nil {
				ctx = nil
			}
			if op.Op == "ref_field" {
				f := fields[op.ID]
				f.References = append(f.References, &d2ir.FieldReference{String: f.Name, Context_: ctx})
			} else {
				e := edges[op.ID]
				e.References = append(e.References, &d2ir.EdgeReference{Context_: ctx})
			}
		case "field_primary", "edge_primary":
			value, err := d2parser.ParseValue(op.Key)
			if err != nil {
				panic(err)
			}
			scalar := &d2ir.Scalar{Value: value.(d2ast.Scalar)}
			if op.Op == "field_primary" {
				f := fields[op.ID]
				f.Primary_ = scalar.Copy(f).(*d2ir.Scalar)
			} else {
				e := edges[op.ID]
				e.Primary_ = scalar.Copy(e).(*d2ir.Scalar)
			}
		case "field_array":
			f := fields[op.ID]
			a := &d2ir.Array{}
			if op.Key != "" {
				value, err := d2parser.ParseValue(op.Key)
				if err != nil {
					panic(err)
				}
				a.Values = append(a.Values, &d2ir.Scalar{Value: value.(d2ast.Scalar)})
			}
			if op.Child {
				a.Values = append(a.Values, &d2ir.Map{Fields: []*d2ir.Field{{Name: d2ast.FlatUnquotedString("inside")}}})
			}
			f.Composite = a.Copy(f).(*d2ir.Array)
			registerValue(f.Composite)
		case "field_map", "edge_map":
			value := &d2ir.Map{}
			if op.Child {
				value.Fields = []*d2ir.Field{{Name: d2ast.FlatUnquotedString("inside")}}
			}
			if op.Op == "field_map" {
				f := fields[op.ID]
				f.Composite = value.Copy(f).(*d2ir.Map)
				register(f.Map())
			} else {
				e := edges[op.ID]
				e.Map_ = value.Copy(e).(*d2ir.Map)
				register(e.Map())
			}
		case "merge_map":
			if op.Child {
				overlayMapIndexed(m, maps[op.ID])
			} else {
				d2ir.OverlayMap(m, maps[op.ID])
			}
			register(m)
		case "merge_field":
			if op.Child {
				overlayFieldIndexed(fields[op.ID], fields[op.Index])
			} else {
				d2ir.OverlayField(fields[op.ID], fields[op.Index])
			}
			register(m)
		case "merge_edge":
			if op.Child {
				overlayEdgeMode(edges[op.ID], edges[op.Index], true)
			} else {
				d2ir.OverlayEdge(edges[op.ID], edges[op.Index])
			}
			register(m)
		case "expand":
			if op.Child {
				expandSubstitutionIndexed(m, maps[op.ID], fields[op.Index])
			} else {
				d2ir.ExpandSubstitution(m, maps[op.ID], fields[op.Index])
			}
			register(m)
		case "replace_context":
			contexts[op.Index] = &d2ir.RefContext{ScopeMap: root}
		case "delete_edge":
			var id *d2ir.EdgeID
			if op.Key != "" {
				id = mapEdge(op.Key)
			}
			removed = edgeID(m.DeleteEdge(id))
		case "delete_field":
			path := []string{}
			for _, h := range op.Path {
				b, _ := hex.DecodeString(h)
				path = append(path, string(b))
			}
			removed = fieldID(m.DeleteField(path...))
		case "base", "base_nil":
			var parent d2ir.Node
			if op.Child {
				parent = root
			}
			if op.Op == "base_nil" {
				m = nil
			}
			copy := m.CopyBase(parent)
			created = len(maps)
			register(copy)
		default:
			panic(op.Op)
		}
		snapshot(removed)
	}
	return snapshots
}

func irResolveOracle(data []byte, parts []string, quoted bool) (result any) {
	defer func() {
		if recover() != nil {
			result = map[string]any{"panic": true}
		}
	}()
	root := &d2ir.Map{}
	m := root
	for _, part := range parts {
		b, _ := hex.DecodeString(part)
		var name d2ast.String = d2ast.FlatUnquotedString(string(b))
		if quoted {
			name = &d2ast.SingleQuotedString{Value: string(b)}
		}
		next := &d2ir.Map{}
		m.Fields = []*d2ir.Field{{Name: name, Composite: next}}
		m = next
	}
	root = root.Copy(nil).(*d2ir.Map)
	maps := []*d2ir.Map{root}
	names := []d2ast.String{root.Parent().(*d2ir.Field).Name}
	m = root
	for len(m.Fields) > 0 {
		names = append(names, m.Fields[0].Name)
		m = m.Fields[0].Map()
		maps = append(maps, m)
	}
	id := mapEdge(string(data))
	resolved, rm, common, err := resolveEdge(id, m)
	if err != nil {
		return map[string]any{"error": err.Error()}
	}
	index := 0
	for i, m := range maps {
		if m == rm {
			index = i
		}
	}
	owners := func(path []d2ast.String) []string {
		out := []string{}
		for _, n := range path {
			owner := "unknown"
			for i, v := range id.SrcPath {
				if n == v {
					owner = "s" + strconv.Itoa(i)
				}
			}
			for i, v := range id.DstPath {
				if n == v {
					owner = "d" + strconv.Itoa(i)
				}
			}
			for i, v := range names {
				if n == v {
					owner = "n" + strconv.Itoa(i)
				}
			}
			out = append(out, owner)
		}
		return out
	}
	raw := []string{}
	for _, path := range [][]d2ast.String{resolved.SrcPath, resolved.DstPath, common} {
		for _, n := range path {
			raw = append(raw, hex.EncodeToString([]byte(n.ScalarString())))
		}
	}
	var indexAlias any
	if id.Index != nil {
		indexAlias = id.Index == resolved.Index
	}
	return map[string]any{"id": resolved, "original": id, "map": index, "common": common, "owners": []any{owners(resolved.SrcPath), owners(resolved.DstPath), owners(common)}, "index_alias": indexAlias, "raw": raw}
}

// Diagnostic access to private glob records. Reflection obtains the actual
// pinned field types; no source package or release implementation is patched.
func privateValue(v reflect.Value) reflect.Value {
	return reflect.NewAt(v.Type(), unsafe.Pointer(v.UnsafeAddr())).Elem()
}
func globField(g reflect.Value, name string) reflect.Value {
	return privateValue(g.Elem().FieldByName(name))
}
func mapGlobs(m *d2ir.Map) reflect.Value {
	return privateValue(reflect.ValueOf(m).Elem().FieldByName("globs"))
}
func importContextOracle(nodes []d2ir.Node, root *d2ir.Map, config string) any {
	flags := strings.Split(config, ":")
	maps := []*d2ir.Map{}
	for _, n := range nodes {
		if m, ok := n.(*d2ir.Map); ok {
			maps = append(maps, m)
		}
	}
	external := (&d2ir.Map{}).Copy(nil).(*d2ir.Map)
	key, _ := d2parser.ParseMapKey("a -> b")
	scope, scopeAST := &d2ast.Map{}, &d2ast.Map{}
	contexts := []*d2ir.RefContext{}
	for _, m := range []*d2ir.Map{root, maps[len(maps)-1], external, nil, maps[len(maps)-1]} {
		contexts = append(contexts, &d2ir.RefContext{Key: key, Edge: key.Edges[0], Scope: scope, ScopeAST: scopeAST, ScopeMap: m})
	}
	for i, n := range nodes {
		switch n := n.(type) {
		case *d2ir.Field:
			n.References = nil
			if i%3 == 1 {
				n.References = []*d2ir.FieldReference{}
			}
			if i%3 == 2 || i == 0 {
				var ctx *d2ir.RefContext
				if i%6 < 5 {
					ctx = contexts[i%6]
				}
				ref := &d2ir.FieldReference{String: n.Name, KeyPath: key.Edges[0].Src, Context_: ctx, DueToLazyGlob_: true}
				n.References = []*d2ir.FieldReference{ref, ref}
			}
		case *d2ir.Edge:
			ref := &d2ir.EdgeReference{Context_: contexts[i%5], DueToLazyGlob_: true}
			n.References = []*d2ir.EdgeReference{ref, ref}
		}
	}
	typ := mapGlobs(root).Type().Elem().Elem()
	globs := []reflect.Value{reflect.New(typ), reflect.New(typ), reflect.New(typ)}
	for i, g := range globs {
		globField(g, "refctx").Set(reflect.ValueOf(contexts[[]int{0, 1, 4}[i]]))
		globField(g, "root").Set(globs[[]int{1, 0, 1}[i]])
		for j, name := range []string{"appliedFields", "appliedEdges"} {
			v := globField(g, name)
			if (i+j)%3 != 2 {
				v.Set(reflect.MakeMap(v.Type()))
				if (i+j)%3 == 0 {
					v.SetMapIndex(reflect.ValueOf("a"), reflect.Zero(v.Type().Elem()))
					v.SetMapIndex(reflect.ValueOf(string([]byte{255})), reflect.Zero(v.Type().Elem()))
				}
			}
		}
	}
	for i, m := range maps {
		v := mapGlobs(m)
		switch flags[0] {
		case "none":
		case "empty":
			v.Set(reflect.MakeSlice(v.Type(), 0, 0))
		case "nil":
			v.Set(reflect.MakeSlice(v.Type(), 1, 1))
		case "shared":
			v.Set(reflect.MakeSlice(v.Type(), 4, 4))
			v.Index(0).Set(globs[i%3])
			v.Index(1).Set(globs[2])
			v.Index(3).Set(globs[i%3])
		}
	}
	var source d2ir.Node = root
	if flags[2] == "field" {
		source = nodes[0]
		if len(root.Fields) > 0 {
			source = root.Fields[0]
		}
	}
	selected := canSelectImport(source)
	var copy d2ir.Node
	if flags[1] == "ordinary" {
		copy = source.Copy(nil)
	} else {
		switch n := source.(type) {
		case *d2ir.Map:
			copy = cloneImportMap(n)
		case *d2ir.Field:
			copy = cloneImportField(n)
		}
	}
	copied := []d2ir.Node{}
	if m, ok := copy.(*d2ir.Map); ok {
		copied = append(copied, m.Parent())
	}
	var visit func(d2ir.Node)
	visit = func(n d2ir.Node) {
		copied = append(copied, n)
		for _, c := range irChildren(n) {
			visit(c)
		}
	}
	visit(copy)
	nodeName := func(n d2ir.Node) any {
		if n == nil || reflect.ValueOf(n).IsNil() {
			return nil
		}
		for i, v := range nodes {
			if n == v {
				return "s" + strconv.Itoa(i)
			}
		}
		for i, v := range copied {
			if n == v {
				return "c" + strconv.Itoa(i)
			}
		}
		return "external"
	}
	ctxs := []*d2ir.RefContext{}
	refs := []any{}
	gs := []reflect.Value{}
	ctxID := func(c *d2ir.RefContext) any {
		if c == nil {
			return nil
		}
		for i, v := range ctxs {
			if c == v {
				return i
			}
		}
		ctxs = append(ctxs, c)
		return len(ctxs) - 1
	}
	refID := func(r any) int {
		for i, v := range refs {
			if r == v {
				return i
			}
		}
		refs = append(refs, r)
		return len(refs) - 1
	}
	globID := func(g reflect.Value) any {
		if g.IsNil() {
			return nil
		}
		for i, v := range gs {
			if g.Pointer() == v.Pointer() {
				return i
			}
		}
		cell := reflect.New(g.Type()).Elem()
		cell.Set(g)
		gs = append(gs, cell)
		return len(gs) - 1
	}
	snapshot := func() any {
		ns := []any{}
		for _, n := range append(append([]d2ir.Node{}, nodes...), copied...) {
			row := map[string]any{"id": nodeName(n)}
			switch n := n.(type) {
			case *d2ir.Map:
				v := mapGlobs(n)
				var ids []any
				if !v.IsNil() {
					ids = []any{}
				}
				for i := 0; i < v.Len(); i++ {
					ids = append(ids, globID(v.Index(i)))
				}
				row["globs"] = ids
			case *d2ir.Field:
				var ids []int
				if n.References != nil {
					ids = []int{}
				}
				for _, r := range n.References {
					ids = append(ids, refID(r))
					ctxID(r.Context_)
				}
				row["refs"] = ids
			case *d2ir.Edge:
				var ids []int
				if n.References != nil {
					ids = []int{}
				}
				for _, r := range n.References {
					ids = append(ids, refID(r))
					ctxID(r.Context_)
				}
				row["refs"] = ids
			}
			ns = append(ns, row)
		}
		globRows := []any{}
		for i := 0; i < len(gs); i++ {
			g := gs[i]
			row := map[string]any{"root": globID(globField(g, "root")), "context": ctxID(globField(g, "refctx").Interface().(*d2ir.RefContext))}
			for _, name := range []string{"appliedFields", "appliedEdges"} {
				v := globField(g, name)
				var keys []string
				if !v.IsNil() {
					keys = []string{}
				}
				for _, k := range v.MapKeys() {
					keys = append(keys, hex.EncodeToString([]byte(k.String())))
				}
				sort.Strings(keys)
				row[name] = keys
			}
			globRows = append(globRows, row)
		}
		rr := []any{}
		for _, r := range refs {
			switch r := r.(type) {
			case *d2ir.FieldReference:
				rr = append(rr, map[string]any{"kind": "field", "context": ctxID(r.Context_), "glob": r.DueToGlob_, "lazy": r.DueToLazyGlob_, "path_alias": r.KeyPath == key.Edges[0].Src})
			case *d2ir.EdgeReference:
				rr = append(rr, map[string]any{"kind": "edge", "context": ctxID(r.Context_), "glob": r.DueToGlob_, "lazy": r.DueToLazyGlob_})
			}
		}
		cc := []any{}
		for _, c := range ctxs {
			cc = append(cc, map[string]any{"map": nodeName(c.ScopeMap), "key": c.Key == key, "edge": c.Edge == key.Edges[0], "scope": c.Scope == scope, "scope_ast": c.ScopeAST == scopeAST})
		}
		return map[string]any{"nodes": ns, "refs": rr, "contexts": cc, "globs": globRows}
	}
	before := snapshot()
	seen := map[uintptr]bool{}
	var mutateGlob func(reflect.Value)
	mutateGlob = func(g reflect.Value) {
		if g.IsNil() || seen[g.Pointer()] {
			return
		}
		seen[g.Pointer()] = true
		v := globField(g, "appliedFields")
		if v.IsNil() {
			v.Set(reflect.MakeMap(v.Type()))
		}
		v.SetMapIndex(reflect.ValueOf("changed"), reflect.Zero(v.Type().Elem()))
		if c := globField(g, "refctx").Interface().(*d2ir.RefContext); c != nil {
			c.ScopeMap = nil
		}
		mutateGlob(globField(g, "root"))
	}
	for _, n := range copied {
		switch n := n.(type) {
		case *d2ir.Field:
			if len(n.References) > 0 {
				n.References[0].DueToGlob_ = true
				if c := n.References[0].Context_; c != nil {
					c.ScopeMap = nil
				}
			}
		case *d2ir.Edge:
			if len(n.References) > 0 {
				n.References[0].DueToGlob_ = true
				if c := n.References[0].Context_; c != nil {
					c.ScopeMap = nil
				}
			}
		case *d2ir.Map:
			v := mapGlobs(n)
			for i := 0; i < v.Len(); i++ {
				mutateGlob(v.Index(i))
			}
			if v.Len() > 0 {
				v.Index(0).Set(reflect.Zero(v.Type().Elem()))
			}
		}
	}
	return map[string]any{"selected": selected, "copy_selected": canSelectImport(copy), "before": before, "after": snapshot()}
}

func patternOracle(data []byte, parts []string) (result any) {
	defer func() {
		if recover() != nil {
			result = map[string]any{"panic": true}
		}
	}()
	pattern := []string{}
	for _, p := range parts {
		b, _ := hex.DecodeString(p)
		pattern = append(pattern, string(b))
	}
	return map[string]any{"match": irMatchPattern(string(data), pattern)}
}
func globOracle(nodes []d2ir.Node) any {
	index := func(n d2ir.Node) any {
		for i, v := range nodes {
			if n == v {
				return i
			}
		}
		return nil
	}
	ids := func(fields []*d2ir.Field) any {
		if fields == nil {
			return nil
		}
		out := []any{}
		for _, f := range fields {
			out = append(out, index(f))
		}
		return out
	}
	patterns := [][]string{{"*"}, {"*", "", "*"}, {"*", "", "*", "", "*"}}
	globs := []any{}
	for _, n := range nodes {
		m, ok := n.(*d2ir.Map)
		if !ok {
			continue
		}
		rows := []any{}
		for _, p := range patterns {
			matches, multi := irMultiGlob(m, p, nil)
			toward := []any{}
			for _, n := range nodes {
				if f, ok := n.(*d2ir.Field); ok {
					toward = append(toward, map[string]any{"field": index(f), "path": ids(irPathToField(m, f)), "matches": ids(irTowardGlob(m, f, p, nil))})
				}
			}
			rows = append(rows, map[string]any{"matches": ids(matches), "multi": multi, "toward": toward})
		}
		globs = append(globs, map[string]any{"map": index(m), "patterns": rows})
	}
	describe := func(path []d2ast.String) any {
		var values []any
		if path != nil {
			values = []any{}
		}
		for _, part := range path {
			var owner any
			for i, n := range nodes {
				if f, ok := n.(*d2ir.Field); ok && part == f.Name {
					owner = i
				}
			}
			values = append(values, map[string]any{"ast": part, "raw": hex.EncodeToString([]byte(part.ScalarString())), "owner": owner})
		}
		return values
	}
	paths := []any{}
	for _, n := range nodes {
		relative := []any{describe(d2ir.RelIDA(nil, n))}
		for _, p := range nodes {
			relative = append(relative, describe(d2ir.RelIDA(p, n)))
		}
		paths = append(paths, map[string]any{"ida": describe(d2ir.IDA(n)), "board": describe(d2ir.BoardIDA(n)), "relative": relative})
	}
	return map[string]any{"globs": globs, "paths": paths}
}
