// SPDX-License-Identifier: MPL-2.0
// Development-only overlay: calls the original recursive compiler primitive.
package d2ir

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2format"
	"github.com/d2lang/d2/d2parser"
	"io"
	"io/fs"
	"sort"
	"strings"
	"testing/fstest"
)

type OracleFieldOperation struct {
	Op        string `json:"op"`
	Key       string `json:"key"`
	Map       string `json:"map"`
	Path      string `json:"path"`
	Value     string `json:"value"`
	Array     bool   `json:"array"`
	Create    bool   `json:"create"`
	Indexed   bool   `json:"indexed"`
	Start     int    `json:"start"`
	NoContext bool   `json:"no_context"`
	Context   *int   `json:"context"`
	Glob      bool   `json:"glob"`
	Enter     bool   `json:"enter"`
	Stack     []int  `json:"stack"`
	Lazy      bool   `json:"lazy"`
	Target    string `json:"target"`
}

func OracleFieldScenario(ops []OracleFieldOperation) []any {
	root := &Map{}
	root.initRoot()
	c := &compiler{err: &d2parser.ParseError{}, globContextStack: [][]*globContext{nil}}
	contexts := []*RefContext{}
	paths := []*d2ast.KeyPath{}
	lookup := func(path string) *Field {
		kp, err := d2parser.ParseKey(path)
		if err != nil {
			panic(err)
		}
		return root.GetField(kp.IDA()...)
	}
	name := func(n Node) string { return d2format.FormatKeyPath(IDA(n)) }
	names := func(fields []*Field) any {
		if fields == nil {
			return nil
		}
		out := []string{}
		for _, f := range fields {
			out = append(out, name(f))
		}
		return out
	}
	ctxID := func(ctx *RefContext) any {
		for i, v := range contexts {
			if ctx == v {
				return i
			}
		}
		return nil
	}
	pathID := func(kp *d2ast.KeyPath) any {
		for i, v := range paths {
			if kp == v {
				return i
			}
		}
		return nil
	}
	rows := []any{}
	for _, op := range ops {
		var selected, created []*Field
		var err error
		if op.Op == "set" {
			f := lookup(op.Path)
			if f == nil {
				panic("missing set field")
			}
			if op.Array {
				f.Composite = &Array{parent: f}
			} else {
				v, e := d2parser.ParseValue(op.Value)
				if e != nil {
					panic(e)
				}
				f.Primary_ = &Scalar{parent: f, Value: v.(d2ast.Scalar)}
			}
		} else {
			m := root
			if op.Map != "" {
				m = lookup(op.Map).Map()
				if m == nil {
					panic("missing query map")
				}
			}
			var ctx *RefContext
			var kp *d2ast.KeyPath
			if op.Context != nil {
				ctx = contexts[*op.Context]
				kp = paths[*op.Context]
			} else {
				key, e := d2parser.ParseMapKey(op.Key)
				if e != nil {
					panic(e)
				}
				ctx = &RefContext{Key: key, ScopeMap: m}
				switch op.Path {
				case "src":
					kp = key.Edges[0].Src
				case "dst":
					kp = key.Edges[0].Dst
				case "edge":
					kp = key.EdgeKey
				default:
					kp = key.Key
				}
				contexts = append(contexts, ctx)
				paths = append(paths, kp)
			}
			c.globRefContextStack = nil
			for _, id := range op.Stack {
				c.globRefContextStack = append(c.globRefContextStack, contexts[id])
			}
			if op.Enter {
				c.globRefContextStack = append(c.globRefContextStack, ctx)
			}
			var gctx *globContext
			if op.Glob {
				gctx = c.ensureGlobContext(ctx)
			}
			c.lazyGlobBeingApplied = op.Lazy
			c.lazyGlobTarget = nil
			if op.Target != "" {
				c.lazyGlobTarget = lookup(op.Target)
			}
			refctx := ctx
			if op.NoContext {
				refctx = nil
			}
			err = m.ensureField(op.Start, kp, refctx, op.Create, gctx, c, op.Indexed, &selected, &created)
		}
		nodes := []any{}
		var visit func(Node)
		visit = func(n Node) {
			row := map[string]any{"path": name(n)}
			switch n := n.(type) {
			case *Map:
				row["kind"] = "map"
				row["version"] = n.structureVersion
				nodes = append(nodes, row)
				for _, f := range n.Fields {
					visit(f)
				}
				for _, e := range n.Edges {
					visit(e)
				}
				return
			case *Field:
				row["kind"] = "field"
				refs := []any{}
				for _, r := range n.References {
					var s any
					if r.String != nil {
						s = r.String
					}
					refs = append(refs, map[string]any{"context": ctxID(r.Context_), "path": pathID(r.KeyPath), "string": s, "glob": r.DueToGlob_, "lazy": r.DueToLazyGlob_})
				}
				row["refs"] = refs
				var owner any
				for i, p := range paths {
					for j, s := range p.Path {
						if s.Unbox() == n.Name {
							owner = []int{i, j}
						}
					}
				}
				row["name_owner"] = owner
				nodes = append(nodes, row)
				if n.Primary_ != nil {
					visit(n.Primary_)
				}
				if n.Composite != nil {
					visit(n.Composite)
				}
				return
			case *Array:
				row["kind"] = "array"
				nodes = append(nodes, row)
				for _, v := range n.Values {
					visit(v)
				}
				return
			case *Scalar:
				row["kind"] = "scalar"
			default:
				panic(fmt.Sprintf("unexpected %T", n))
			}
			nodes = append(nodes, row)
		}
		visit(root)
		globs := []any{}
		for _, g := range c.globContexts() {
			fields := []string{}
			for key := range g.appliedFields {
				fields = append(fields, key)
			}
			sort.Strings(fields)
			globs = append(globs, map[string]any{"context": ctxID(g.refctx), "fields": fields})
		}
		var errorValue any
		if err != nil {
			errorValue = err
		}
		rows = append(rows, map[string]any{"selected": names(selected), "created": names(created), "error": errorValue, "ast": root.AST(), "nodes": nodes, "globs": globs})
	}
	return rows
}

// Unresolved spread placeholders have nil names/keys. Observe these explicitly
// instead of calling the formatter on nil and losing the compiler's real error.
func oracleNodePath(n Node) any {
	path := IDA(n)
	for _, part := range path {
		if part == nil {
			return nil
		}
	}
	return d2format.FormatKeyPath(path)
}
func oracleKey(k *d2ast.Key) any {
	if k == nil {
		return nil
	}
	return d2format.Format(k)
}

// OracleCompileFields retains internal state even when public Compile returns
// a nil map. The production compiler functions themselves remain unchanged.
func OracleCompileFields(input, path string) any {
	return oracleCompileFields(input, path, nil, false, nil)
}
func OracleCompileImports(input, path string, files map[string]string, utf16 bool) (out any) {
	phase := "parse"
	defer func() {
		if recovered := recover(); recovered != nil {
			out = map[string]any{"panic": true, "phase": phase}
		}
	}()
	return oracleCompileFields(input, path, files, utf16, &phase)
}
func oracleCompileFields(input, path string, files map[string]string, utf16 bool, phase *string) any {
	source, parseErr := d2parser.Parse(path, strings.NewReader(input), &d2parser.ParseOptions{UTF16Pos: utf16})
	if parseErr != nil {
		return map[string]any{"parse_error": parseErr.Error()}
	}
	c := &compiler{err: &d2parser.ParseError{}, seenImports: map[string]struct{}{}, parsedImports: map[string]*d2ast.Map{}, importTemplates: map[string]*importTemplate{}}
	defer func() { delete(oracleCompilerTraces, c) }()
	c.utf16Pos = utf16
	if files != nil {
		fsys := fstest.MapFS{}
		for path, text := range files {
			fsys[path] = &fstest.MapFile{Data: []byte(text)}
		}
		c.fs = fsys
		c.importStack = []string{path}
	}
	root := &Map{}
	root.initRoot()
	root.parent.(*Field).References[0].Context_.Scope = source
	root.parent.(*Field).References[0].Context_.ScopeAST = source
	if phase != nil {
		*phase = "compile-map"
	}
	c.compileMap(root, source, source)
	if phase != nil {
		*phase = "substitutions"
	}
	c.compileSubstitutions(root, nil)
	if phase != nil {
		*phase = "classes"
	}
	c.overlayClasses(root)
	if phase != nil {
		*phase = "cleanup"
	}
	root.removeSuspendedFields()
	if phase != nil {
		*phase = "observe"
	}
	errors := []any{}
	for _, err := range c.err.Errors {
		errors = append(errors, err)
	}
	nodes := []any{}
	var visit func(Node)
	visit = func(n Node) {
		if scalar, ok := n.(*Scalar); ok && scalar == nil {
			nodes = append(nodes, map[string]any{"path": nil, "kind": "scalar", "nil": true})
			return
		}
		row := map[string]any{"path": oracleNodePath(n)}
		switch n := n.(type) {
		case *Map:
			row["version"] = n.structureVersion
			row["kind"] = "map"
			globs := []any{}
			for _, g := range n.globs {
				fields := []string{}
				for f := range g.appliedFields {
					fields = append(fields, f)
				}
				sort.Strings(fields)
				edges := []string{}
				for e := range g.appliedEdges {
					edges = append(edges, e)
				}
				sort.Strings(edges)
				globs = append(globs, map[string]any{"edges": edges, "key": d2format.Format(g.refctx.Key), "scope": oracleScopeName(g.refctx.ScopeMap), "fields": fields})
			}
			row["globs"] = globs
			nodes = append(nodes, row)
			for _, f := range n.Fields {
				visit(f)
			}
			for _, e := range n.Edges {
				visit(e)
			}
			return
		case *Field:
			row["kind"] = "field"
			row["suspended"] = n.suspended
			refs := []any{}
			for _, r := range n.References {
				refs = append(refs, map[string]any{"key": oracleKey(r.Context_.Key), "string": r.String, "glob": r.DueToGlob_, "lazy": r.DueToLazyGlob_, "scope": oracleScopeName(r.Context_.ScopeMap)})
			}
			row["refs"] = refs
			nodes = append(nodes, row)
			if n.Primary_ != nil {
				visit(n.Primary_)
			}
			if n.Composite != nil {
				visit(n.Composite)
			}
			return
		case *Edge:
			row["kind"] = "edge"
			row["id"] = n.ID
			row["suspended"] = n.suspended
			refs := []any{}
			for _, r := range n.References {
				refs = append(refs, map[string]any{"key": oracleKey(r.Context_.Key), "edge": r.Context_.Edge, "glob": r.DueToGlob_, "lazy": r.DueToLazyGlob_, "scope": oracleScopeName(r.Context_.ScopeMap)})
			}
			row["refs"] = refs
			nodes = append(nodes, row)
			if n.Primary_ != nil {
				visit(n.Primary_)
			}
			if n.Map_ != nil {
				visit(n.Map_)
			}
			return
		case *Array:
			row["kind"] = "array"
			nodes = append(nodes, row)
			for _, v := range n.Values {
				visit(v)
			}
			return
		case *Scalar:
			row["kind"] = "scalar"
		default:
			panic(fmt.Sprintf("unexpected %T", n))
		}
		nodes = append(nodes, row)
	}
	visit(root)
	var projected any
	if files != nil {
		projected = oracleImportAST(root)
	} else {
		projected = root.AST()
	}
	out := map[string]any{"ast": projected, "errors": errors, "nodes": nodes}
	if files != nil {
		parsed := []string{}
		for k := range c.parsedImports {
			parsed = append(parsed, k)
		}
		sort.Strings(parsed)
		templates := []string{}
		for k := range c.importTemplates {
			templates = append(templates, k)
		}
		sort.Strings(templates)
		seen := []string{}
		for k := range c.seenImports {
			seen = append(seen, k)
		}
		sort.Strings(seen)
		out["parsed"] = parsed
		out["templates"] = templates
		out["seen"] = seen
		out["imports"] = c.imports
	}
	if oracleTraceEnabled {
		trace := oracleCompilerTraces[c]
		if trace == nil {
			trace = []any{}
		}
		out["trace"] = trace
		delete(oracleCompilerTraces, c)
	}
	return out
}

var oracleTraceEnabled = false
var oracleCompilerTraces = map[*compiler][]any{}

func oracleCompilerEvent(c *compiler, phase string, refctx *RefContext) {
	name := func(n Node) string { return hex.EncodeToString([]byte(d2format.FormatKeyPath(IDA(n)))) }
	var target any
	if c.lazyGlobTarget != nil {
		target = name(c.lazyGlobTarget)
	}
	queue := []string{}
	for _, f := range c.lazyGlobWorklist {
		queue = append(queue, name(f))
	}
	post := []string{}
	for _, f := range c.lazyPostTargets {
		post = append(post, name(f))
	}
	var queued any
	if c.lazyGlobQueued != nil {
		queued = len(c.lazyGlobQueued)
	}
	oracleCompilerTraces[c] = append(oracleCompilerTraces[c], map[string]any{"phase": phase, "key_range": refctx.Key.Range, "scope": name(refctx.ScopeMap), "target": target, "lazy": c.lazyGlobBeingApplied, "applying": c.applyingLazyWorklist, "stack": len(c.globRefContextStack), "version": refctx.ScopeMap.structureVersion, "queue": queue, "post": post, "queued": queued})
}

type OracleImportOperation struct {
	Op      string `json:"op"`
	File    string `json:"file"`
	Pre     string `json:"pre"`
	Located bool   `json:"located"`
	Hex     string `json:"hex"`
}
type OracleImportRequest struct {
	Files map[string]string       `json:"files"`
	UTF16 bool                    `json:"utf16"`
	Ops   []OracleImportOperation `json:"ops"`
}
type oracleImportFS struct {
	files fstest.MapFS
	opens map[string]int
}

func (f *oracleImportFS) Open(name string) (fs.File, error) {
	f.opens[name]++
	if _, ok := f.files[name]; !ok {
		return nil, fmt.Errorf("missing %s", name)
	}
	return f.files.Open(name)
}
func OracleImportLoading(r OracleImportRequest) any {
	source := &oracleImportFS{files: fstest.MapFS{}, opens: map[string]int{}}
	for name, data := range r.Files {
		b, e := hex.DecodeString(data)
		if e != nil {
			panic(e)
		}
		source.files[name] = &fstest.MapFile{Data: b}
	}
	c := &compiler{fs: source, err: &d2parser.ParseError{}, parsedImports: map[string]*d2ast.Map{}, utf16Pos: r.UTF16}
	var working *d2ast.Map
	out := []any{}
	for _, op := range r.Ops {
		var resolved any
		var openError any
		ok := true
		switch op.Op {
		case "push":
			imp := &d2ast.Import{Pre: op.Pre, Path: []*d2ast.StringBox{d2ast.RawStringBox(op.File, true)}}
			if op.Located {
				imp.Range = d2ast.MakeRange("fixture.d2,0:0:0-0:1:1")
			}
			p, success := c.pushImportStack(imp)
			ok = success
			if ok {
				resolved = p
			}
		case "pop":
			c.popImportStack()
		case "load":
			var e error
			// No import node: it only positions a variable-expansion budget error,
			// which these small inputs cannot reach.
			working, e, ok = c.loadImportAST(op.File, nil, c.err)
			if e != nil {
				openError = e.Error()
			}
		case "write":
			b, e := hex.DecodeString(op.Hex)
			if e != nil {
				panic(e)
			}
			source.files[op.File] = &fstest.MapFile{Data: b}
		case "error":
			c.err.Errors = append(c.err.Errors, d2ast.Error{Message: "existing error"})
		case "clear-errors":
			c.err.Errors = nil
		case "mutate":
			if working != nil && len(working.Nodes) > 0 && working.Nodes[0].MapKey != nil {
				k := working.Nodes[0].MapKey
				if k.Key != nil && len(k.Key.Path) > 0 {
					k.Key.Path[0] = d2ast.RawStringBox("changed", true)
				}
				if k.Value.UnquotedString != nil {
					k.Value.UnquotedString.SetString("changed")
				}
			}
		default:
			panic(op.Op)
		}
		cache := []string{}
		for name := range c.parsedImports {
			cache = append(cache, name)
		}
		sort.Strings(cache)
		stack := append([]string{}, c.importStack...)
		// Marshal snapshots now: later mutation must not alter prior observations.
		row := map[string]any{"path": resolved, "ok": ok, "open_error": openError, "stack": stack, "cache": cache, "errors": c.err, "working": working, "opens": source.opens}
		b, e := json.Marshal(row)
		if e != nil {
			panic(e)
		}
		var copied any
		json.Unmarshal(b, &copied)
		out = append(out, copied)
	}
	return out
}

type oracleSingleStreamFS struct{ reader io.Reader }

func (f *oracleSingleStreamFS) Open(string) (fs.File, error) {
	return &oracleSingleStreamFile{Reader: f.reader}, nil
}

type oracleSingleStreamFile struct{ io.Reader }

func (f *oracleSingleStreamFile) Stat() (fs.FileInfo, error) { return nil, nil }
func (f *oracleSingleStreamFile) Close() error               { return nil }
func OracleLoadStream(reader io.Reader, seed, utf16 bool) any {
	accumulator := &d2parser.ParseError{}
	if seed {
		accumulator.Errors = append(accumulator.Errors, d2ast.Error{Message: "existing error"})
	}
	c := &compiler{fs: &oracleSingleStreamFS{reader: reader}, parsedImports: map[string]*d2ast.Map{}, utf16Pos: utf16}
	working, e, ok := c.loadImportAST("stream.d2", nil, accumulator)
	var openError any
	if e != nil {
		openError = e.Error()
	}
	return map[string]any{"working": working, "open_error": openError, "ok": ok, "accumulator": accumulator, "cached": len(c.parsedImports) > 0}
}

func oracleScopeName(m *Map) any {
	if m == nil {
		return nil
	}
	return d2format.FormatKeyPath(IDA(m))
}

func oracleImportAST(root *Map) (out any) {
	defer func() {
		if recovered := recover(); recovered != nil {
			out = map[string]any{"panic": true}
		}
	}()
	return root.AST()
}
