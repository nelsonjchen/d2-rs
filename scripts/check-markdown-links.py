#!/usr/bin/env python3
"""Goldmark oracle fixture generator (recipe `goldmark-link`, tools/fixtures/goldmark-markdown.toml).

Builds tools/markdown-link-oracle/main.go against the pinned Goldmark module, captures
its results for the requests below and writes them to the staged recipe
output. Run through `uv run --no-project python scripts/regen-fixtures.py
--only goldmark-link`; `cargo test -p d2-rs-text --features oracle-fixtures`
performs the native comparison (docs/FIXTURES.md).
"""
import json
import subprocess
from references import ROOT
from fixture_recipe import write_request_fixture
from goldmark_oracle import build, prepare_module

harness = ROOT / 'tools/markdown-link-oracle/main.go'
module, trace_dep = prepare_module('markdown-link-oracle', harness, copy_goldmark=True)
dep = trace_dep  # verified byte-identical to the pinned module
bridge=module/'oracle_links.go.txt'
bridge.write_text('package parser\nimport "github.com/yuin/goldmark/text"\nimport "github.com/yuin/goldmark/ast"\nimport "fmt"\nimport "bytes"\nimport "regexp/syntax"\nimport "reflect"\nimport "github.com/yuin/goldmark/util"\nfunc OracleLinkValue(r text.Reader,title bool)([]byte,bool){if title{return parseLinkTitle(r)};return parseLinkDestination(r)}\nfunc OracleReference(r text.Reader)(*ast.LinkReferenceDefinition,int,int){n,s,e:=parseLinkReferenceDefinition(r,NewContext());if n==nil{return nil,s,e};return n.(*ast.LinkReferenceDefinition),s,e}\n')

with bridge.open('a') as out: out.write("""
func OracleCodeBlock(source []byte,segments []text.Segment)any{
 r:=text.NewReader(source);pc:=NewContext();b:=NewCodeBlockParser();r.SetPosition(0,segments[0]);n,state:=b.Open(ast.NewDocument(),r,pc)
 snap:=func()any{line,pos:=r.Position();var lines any;var nodepos any;if n!=nil{lines=append([]text.Segment{},n.Lines().Sliced(0,n.Lines().Len())...);nodepos=n.Pos()};return map[string]any{"state":int(state),"line":line,"position":pos,"lines":lines,"nodepos":nodepos}}
 steps:=[]any{snap()}
 if n!=nil{for i,segment:=range segments[1:]{r.SetPosition(i+1,segment);state=b.Continue(n,r,pc);steps=append(steps,snap());if state==Close{break}};b.Close(n,r,pc);steps=append(steps,snap())}
 return map[string]any{"steps":steps,"source":fmt.Sprintf("%x",source)}
}
func OracleFenceBlock(source []byte,segments []text.Segment)any{
 r:=text.NewReader(source);pc:=NewContext();b:=NewFencedCodeBlockParser();r.SetPosition(0,segments[0]);value,_:=r.PeekLine();_,offset:=util.IndentWidth(value,r.LineOffset());if offset>=len(value){offset=-1};pc.SetBlockOffset(offset);n,state:=b.Open(ast.NewDocument(),r,pc)
 snap:=func()any{var info any;if n!=nil{if v:=n.(*ast.FencedCodeBlock).Info;v!=nil{info=v.Segment}};line,pos:=r.Position();var lines any;var nodepos any;if n!=nil{lines=append([]text.Segment{},n.Lines().Sliced(0,n.Lines().Len())...);nodepos=n.Pos()};return map[string]any{"info":info,"context":pc.Get(fencedCodeBlockInfoKey)!=nil,"state":int(state),"line":line,"position":pos,"lines":lines,"nodepos":nodepos}}
 steps:=[]any{snap()}
 if n!=nil{for i,segment:=range segments[1:]{r.SetPosition(i+1,segment);state=b.Continue(n,r,pc);steps=append(steps,snap());if state==Close{break}};b.Close(n,r,pc);steps=append(steps,snap())}
 return map[string]any{"steps":steps,"source":fmt.Sprintf("%x",source)}
}
func OracleSetextLifecycle(source []byte,segments []text.Segment,mode string)any{
 root:=ast.NewDocument();p:=ast.NewParagraph();p.Lines().Append(segments[0]);p.SetBlankPreviousLines(true);root.AppendChild(root,p)
 pc:=NewContext();pc.SetOpenedBlocks([]Block{{Node:p}});r:=text.NewReader(source);r.SetPosition(1,segments[1]);b:=NewSetextHeadingParser();n,state:=b.Open(root,r,pc)
 if n!=nil{root.AppendChild(root,n);if mode!="setextlife"{p.Lines().Clear()};if mode=="setextnext" || mode=="setextnextempty"{next:=ast.NewParagraph();if mode=="setextnext"{next.Lines().Append(segments[0])};root.AppendChild(root,next)};b.Close(n,r,pc)}
 nodes:=[]any{};for v:=root.FirstChild();v!=nil;v=v.NextSibling(){nodes=append(nodes,map[string]any{"kind":v.Kind().String(),"lines":append([]text.Segment{},v.Lines().Sliced(0,v.Lines().Len())...),"pos":v.Pos(),"blank":v.HasBlankPreviousLines()})}
 return map[string]any{"nodes":nodes,"state":int(state),"temporary":pc.Get(temporaryParagraphKey)!=nil,"paragraph_attached":p.Parent()!=nil}
}
func OracleATX(source []byte,segment text.Segment)any{
 r:=text.NewReader(source);r.SetPosition(0,segment);pc:=NewContext();line,_:=r.PeekLine();_,offset:=util.IndentWidth(line,r.LineOffset());pc.SetBlockOffset(offset)
 n,state:=NewATXHeadingParser().Open(ast.NewDocument(),r,pc);var lines any;var level any;if n!=nil{lines=append([]text.Segment{},n.Lines().Sliced(0,n.Lines().Len())...);level=n.(*ast.Heading).Level};sl,sp:=r.Position()
 return map[string]any{"lines":lines,"level":level,"state":int(state),"line":sl,"position":sp}
}
func OracleListItem(source []byte)any{
	match,kind:=parseListItem(source)
	return map[string]any{"match":match,"kind":int(kind),"offset":calcListOffset(source,match)}
}
func OracleSetext(source []byte)any{mark,ok:=matchesSetextHeadingBar(source);if !ok{return nil};return int(mark)}
func OracleSimpleBlock(source []byte,segment text.Segment,kind string)any{
 r:=text.NewReader(source);r.SetPosition(0,segment)
 var b BlockParser=NewThematicBreakParser();if kind=="blockquote"{b=NewBlockquoteParser()}
 n,state:=b.Open(ast.NewDocument(),r,NewContext())
 var node any;if n!=nil{node=n.Kind().String()}
 line,pos:=r.Position();value,_:=r.PeekLine();var peek any;if value!=nil{peek=fmt.Sprintf("%x",value)}
 return map[string]any{"node":node,"state":int(state),"line":line,"position":pos,"peek":peek,"offset":r.LineOffset()}
}
func OracleHTMLBlock(source []byte,segments []text.Segment)any{
	r:=text.NewReader(source);pc:=NewContext();b:=NewHTMLBlockParser();root:=ast.NewDocument();steps:=[]any{}
	var node ast.Node
	snap:=func(state State)any{line,pos:=r.Position();var typ any;var lines any;var closure any;if node!=nil{h:=node.(*ast.HTMLBlock);typ=int(h.HTMLBlockType);lines=append([]text.Segment{},h.Lines().Sliced(0,h.Lines().Len())...);if h.HasClosure(){closure=h.ClosureLine}};return map[string]any{"type":typ,"lines":lines,"closure":closure,"state":int(state),"line":line,"position":pos}}
	if len(segments)>0{r.SetPosition(0,segments[0]);n,state:=b.Open(root,r,pc);node=n;if n!=nil{root.AppendChild(root,n)};steps=append(steps,snap(state));if n!=nil{for i,segment:=range segments[1:]{r.SetPosition(i+1,segment);state=b.Continue(n,r,pc);steps=append(steps,snap(state));if state==Close{break}};b.Close(n,r,pc)}}
	return map[string]any{"steps":steps,"source":fmt.Sprintf("%x",source)}
}
func OracleLabelStates(segments []text.Segment, removes []byte) any {
 pc:=NewContext()
 nodes:=[]*linkLabelState{}
 for i,seg:=range segments {nodes=append(nodes,newLinkLabelState(seg,i%2==1))}
 id:=func(n *linkLabelState)any{if n==nil{return nil};for i,v:=range nodes{if v==n{return i}};panic("unregistered")}
 snap:=func()any{
  var head *linkLabelState
  if v:=pc.Get(linkLabelStateKey);v!=nil{head=v.(*linkLabelState)}
  values:=[]any{}
  for _,n:=range nodes{values=append(values,map[string]any{"previous":id(n.Prev),"next":id(n.Next),"first":id(n.First),"last":id(n.Last),"length":linkLabelStateLength(n)})}
  return map[string]any{"head":id(head),"nodes":values}
 }
 rounds:=[]any{snap()}
 for _,n:=range nodes{pushLinkLabelState(pc,n);rounds=append(rounds,snap())}
 for _,i:=range removes{removeLinkLabelState(pc,nodes[int(i)]);rounds=append(rounds,snap())}
 return rounds
}
""")

with bridge.open('a') as out:out.write("""
func OracleLinkBottoms(ops []byte) any {
 pc:=NewContext()
 d1,d2:=&Delimiter{},&Delimiter{}
 rounds:=[]any{}
 pop:=func(){
  n:=popLinkBottom(pc)
  var value any
  if n==nil {value="absent"} else if n==(*Delimiter)(nil){value="typed_nil"} else if n==d1{value=0}else if n==d2{value=1}else{panic("unknown")}
  rounds=append(rounds,value)
 }
 for _,op:=range ops{
  switch op{
   case 0:pc.(*parseContext).lastDelimiter=nil;pushLinkBottom(pc)
   case 1:pc.(*parseContext).lastDelimiter=d1;pushLinkBottom(pc)
   case 2:pc.(*parseContext).lastDelimiter=d2;pushLinkBottom(pc)
   case 3:pop()
   case 4:pc.Set(linkBottom,nil)
  }
 }
 for range len(ops)+1{pop()}
 return rounds
}
""")

with bridge.open('a') as out:out.write("""
func OracleLinkOpenClose(source []byte,segments []text.Segment) any {
 pc:=NewContext()
 root:=ast.NewParagraph()
 lines:=text.NewSegments();lines.AppendAll(segments)
 r:=text.NewBlockReader(source,lines)
 labels:=[]*linkLabelState{}
 snap:=func()any{
  children:=[]any{}
  for n:=root.FirstChild();n!=nil;n=n.NextSibling(){
   switch v:=n.(type){
   case *linkLabelState:children=append(children,map[string]any{"kind":"label","segment":v.Segment,"image":v.IsImage})
   case *ast.Text:children=append(children,map[string]any{"kind":"text","segment":v.Segment})
   }
  }
  attached:=[]bool{}
  for _,v:=range labels{attached=append(attached,v.Parent()!=nil)}
  line,pos:=r.Position()
  return map[string]any{"children":children,"attached":attached,"count":root.ChildCount(),"head":pc.Get(linkLabelStateKey)!=nil,"line":line,"position":pos}
 }
 rounds:=[]any{snap()}
 for {
  line,_:=r.PeekLine()
  if line==nil{break}
  if len(line)>0 && (line[0]=='[' || line[0]=='!'){
   n:=defaultLinkParser.Parse(root,r,pc)
   if n!=nil{root.AppendChild(root,n);labels=append(labels,n.(*linkLabelState))}else{r.Advance(1)}
  }else{r.Advance(1)}
  rounds=append(rounds,snap())
 }
 defaultLinkParser.CloseBlock(root,r,pc);rounds=append(rounds,snap())
 defaultLinkParser.CloseBlock(root,r,pc);rounds=append(rounds,snap())
 return rounds
}
""")

with bridge.open('a') as out:out.write("""
func OracleParseLinks(source []byte,segments []text.Segment) any {
 pc:=NewContext()
 for _,label:=range []string{"x","a","strasse"}{pc.AddReference(NewReference([]byte(label),[]byte("destination"),[]byte("title")))}
 root:=ast.NewParagraph()
 lines:=text.NewSegments();lines.AppendAll(segments)
 r:=text.NewBlockReader(source,lines)
 value:=func(v []byte)any{if v==nil{return nil};return fmt.Sprintf("%x",v)}
 var tree func(ast.Node)any
 tree=func(n ast.Node)any{
  children:=[]any{}
  for c:=n.FirstChild();c!=nil;c=c.NextSibling(){children=append(children,tree(c))}
  data:=map[string]any{"children":children,"count":n.ChildCount()}
  switch v:=n.(type){
  case *ast.Paragraph:data["kind"]="root"
  case *ast.Text:data["kind"]="text";data["segment"]=v.Segment;data["raw"]=v.IsRaw();data["soft"]=v.SoftLineBreak();data["hard"]=v.HardLineBreak()
  case *ast.CodeSpan:data["kind"]="code"
  case *linkLabelState:data["kind"]="label";data["segment"]=v.Segment;data["image"]=v.IsImage
  case *Delimiter:data["kind"]="delimiter";data["segment"]=v.Segment;data["length"]=v.Length;data["char"]=v.Char
  case *ast.Emphasis:data["kind"]="emphasis";data["level"]=v.Level;data["pos"]=v.Pos()
  case *ast.AutoLink:data["kind"]="auto";data["type"]=int(v.AutoLinkType);data["label"]=value(v.Label(source));data["url"]=value(v.URL(source))
  case *ast.RawHTML:data["kind"]="html";data["segments"]=v.Segments.Sliced(0,v.Segments.Len())
  case *ast.Link:
   data["kind"]="link";data["destination"]=value(v.Destination);data["title"]=value(v.Title);data["pos"]=v.Pos()
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  case *ast.Image:
   data["kind"]="image";data["destination"]=value(v.Destination);data["title"]=value(v.Title);data["pos"]=v.Pos()
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  default:panic("unhandled node")
  }
  return data
 }
 snap:=func()any{line,pos:=r.Position();return map[string]any{"tree":tree(root),"line":line,"position":pos}}
 rounds:=[]any{snap()}
 for {
  line,seg:=r.PeekLine();if line==nil{break}
  l,pos:=r.Position()
  var n ast.Node
  if len(line)>0 && (line[0]=='[' || line[0]=='!' || line[0]==']'){n=defaultLinkParser.Parse(root,r,pc)}else if len(line)>0 && (line[0]=='*'||line[0]=='_'){n=NewEmphasisParser().Parse(root,r,pc)}else if len(line)>0 && line[0]==96{n=NewCodeSpanParser().Parse(root,r,pc)}else if len(line)>0 && line[0]==60{n=NewAutoLinkParser().Parse(root,r,pc);if n==nil{n=NewRawHTMLParser().Parse(root,r,pc)}}
  if n==nil{r.SetPosition(l,pos);r.Advance(1);ast.MergeOrAppendTextSegment(root,text.NewSegment(seg.Start,seg.Start+1))}else{root.AppendChild(root,n)}
  rounds=append(rounds,snap())
 }
 ProcessDelimiters(nil,pc);rounds=append(rounds,snap())
 defaultLinkParser.CloseBlock(root,r,pc);rounds=append(rounds,snap())
 return rounds
}
""")
with bridge.open('a') as out:out.write('func OracleTagPrefix(source []byte,closing bool)int{reg:=openTagRegexp;if closing{reg=closeTagRegexp};v:=reg.FindIndex(source);if v==nil{return -1};return v[1]}\n')
with bridge.open('a') as out:out.write('func OracleTagProgram(closing bool)any{reg:=openTagRegexp;if closing{reg=closeTagRegexp};r,e:=syntax.Parse(reg.String(),syntax.Perl);if e!=nil{panic(e)};p,e:=syntax.Compile(r.Simplify());if e!=nil{panic(e)};return map[string]any{"program":p,"onepass":!reflect.ValueOf(reg).Elem().FieldByName("onepass").IsNil(),"pattern":reg.String()}}\n')

with bridge.open('a') as out:out.write("""
type oracleTagReader struct{text.Reader;Events []any}
func(r *oracleTagReader)ReadRune()(rune,int,error){
 v,n,e:=r.Reader.ReadRune();line,pos:=r.Reader.Position()
 r.Events=append(r.Events,map[string]any{"rune":int(v),"size":n,"eof":e!=nil,"line":line,"position":pos})
 return v,n,e
}
func OracleTagMatch(source []byte,segments []text.Segment,closing bool)any{
 reg:=openTagRegexp;if closing{reg=closeTagRegexp}
 plainSource:=append([]byte(nil),source...)
 lines:=text.NewSegments();lines.AppendAll(segments)
 r:=text.NewBlockReader(source,lines)
 oldline,oldpos:=r.Position()
 observed:=&oracleTagReader{Reader:r,Events:[]any{}}
 match:=reg.FindReaderSubmatchIndex(observed)
 beforeLine,beforePos:=r.Position()
 r.SetPosition(oldline,oldpos)
 var end any
 if match!=nil{end=match[1];r.Advance(match[1]-match[0])}
 finalLine,finalPos:=r.Position()
 plain:=text.NewBlockReader(plainSource,lines)
 ok:=plain.Match(reg)
 pl,pp:=plain.Position()
 equal:=ok==(match!=nil)&&pl==finalLine&&pp==finalPos&&bytes.Equal(plainSource,source)
 return map[string]any{"end":end,"events":observed.Events,"before_line":beforeLine,"before_position":beforePos,"line":finalLine,"position":finalPos,"source":fmt.Sprintf("%x",source),"plain_equal":equal}
}
""")

with bridge.open('a') as out:out.write("""
func OracleParsePrepared(source []byte,node ast.Node,pc Context){
 p:=NewParser().(*parser);for _,v:=range DefaultInlineParsers(){p.addInlineParser(v,nil)}
 p.parseBlock(text.NewBlockReader(source,nil),node,pc)
}
func OracleInlineBlock(source []byte,segments []text.Segment,escaped,raw bool,extra InlineParser)any{
 p:=NewParser().(*parser)
 for _,v:=range DefaultInlineParsers(){p.addInlineParser(v,nil)}
 if extra!=nil{p.addInlineParser(util.Prioritized(extra,500),nil)}
 p.escapedSpace=escaped
 pc:=NewContext()
 for _,label:=range []string{"x","a","strasse"}{pc.AddReference(NewReference([]byte(label),[]byte("destination"),[]byte("title")))}
 var root ast.Node=ast.NewParagraph()
 if raw{root=ast.NewCodeBlock()}
 lines:=text.NewSegments();lines.AppendAll(segments);root.SetLines(lines)
 r:=text.NewBlockReader(source,nil)
 p.parseBlock(r,root,pc)
 value:=func(v []byte)any{if v==nil{return nil};return fmt.Sprintf("%x",v)}
 var tree func(ast.Node)any
 tree=func(n ast.Node)any{
  children:=[]any{}
  for c:=n.FirstChild();c!=nil;c=c.NextSibling(){children=append(children,tree(c))}
  data:=map[string]any{"kind":n.Kind().String(),"children":children,"count":n.ChildCount(),"pos":n.Pos()}
  if n==root{data["kind"]="root"}
  switch v:=n.(type){
  case *ast.Text:data["segment"]=v.Segment;data["raw"]=v.IsRaw();data["soft"]=v.SoftLineBreak();data["hard"]=v.HardLineBreak()
  case *ast.Emphasis:data["level"]=v.Level
  case *ast.Link:
   data["destination"]=value(v.Destination);data["title"]=value(v.Title)
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  case *ast.Image:
   data["destination"]=value(v.Destination);data["title"]=value(v.Title)
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  case *ast.AutoLink:data["type"]=int(v.AutoLinkType);data["label"]=value(v.Label(source));data["url"]=value(v.URL(source))
  case *ast.RawHTML:data["segments"]=v.Segments.Sliced(0,v.Segments.Len())
  }
  return data
 }
 line,pos:=r.Position()
 return map[string]any{"tree":tree(root),"line":line,"position":pos,"source":fmt.Sprintf("%x",source)}
}
""")

with bridge.open('a') as out:out.write("""
func OracleReferenceParagraph(source []byte,segments []text.Segment,mutate bool)any{
 pc:=NewContext()
 root:=ast.NewDocument();paragraph:=ast.NewParagraph()
 lines:=text.NewSegments();lines.AppendAll(segments);paragraph.SetLines(lines);paragraph.SetBlankPreviousLines(true)
 root.AppendChild(root,paragraph)
 r:=text.NewReader(source)
 LinkReferenceParagraphTransformer.Transform(paragraph,r,pc)
 value:=func(v []byte)any{if v==nil{return nil};return fmt.Sprintf("%x",v)}
 var tree func(ast.Node)any
 tree=func(n ast.Node)any{
  children:=[]any{};for c:=n.FirstChild();c!=nil;c=c.NextSibling(){children=append(children,tree(c))}
  data:=map[string]any{"kind":n.Kind().String(),"children":children,"count":n.ChildCount(),"pos":n.Pos()}
  if n==root{data["kind"]="root"}
  switch v:=n.(type){
  case *ast.Paragraph:data["lines"]=append([]text.Segment{},v.Lines().Sliced(0,v.Lines().Len())...);data["blank"]=v.HasBlankPreviousLines()
  case *ast.LinkReferenceDefinition:data["lines"]=append([]text.Segment{},v.Lines().Sliced(0,v.Lines().Len())...);data["blank"]=v.HasBlankPreviousLines();data["label"]=value(v.Label);data["destination"]=value(v.Destination);data["title"]=value(v.Title)
  case *ast.Text:data["segment"]=v.Segment;data["raw"]=v.IsRaw();data["soft"]=v.SoftLineBreak();data["hard"]=v.HardLineBreak()
  case *ast.Emphasis:data["level"]=v.Level
  case *ast.Link:
   data["destination"]=value(v.Destination);data["title"]=value(v.Title)
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  case *ast.Image:
   data["destination"]=value(v.Destination);data["title"]=value(v.Title)
   var ref any;if v.Reference!=nil{ref=map[string]any{"type":int(v.Reference.Type),"value":value(v.Reference.Value)}};data["reference"]=ref
  case *ast.AutoLink:data["type"]=int(v.AutoLinkType);data["label"]=value(v.Label(source));data["url"]=value(v.URL(source))
  case *ast.RawHTML:data["segments"]=v.Segments.Sliced(0,v.Segments.Len())
  }
  return data
 }
 keys:=[]string{"x","a","renamed","strasse"}
 for n:=root.FirstChild();n!=nil;n=n.NextSibling(){if v,ok:=n.(*ast.LinkReferenceDefinition);ok{keys=append(keys,util.ToLinkReference(v.Label))}}
 snap:=func()any{
  refs:=[]any{}
  for _,key:=range keys{v,ok:=pc.Reference(key);var item any;if ok{item=map[string]any{"label":value(v.Label()),"destination":value(v.Destination()),"title":value(v.Title())}};refs=append(refs,item)}
  return map[string]any{"tree":tree(root),"lookups":refs,"source":fmt.Sprintf("%x",source)}
 }
 stages:=[]any{snap()}
 if mutate{for n:=root.FirstChild();n!=nil;n=n.NextSibling(){if v,ok:=n.(*ast.LinkReferenceDefinition);ok{v.Label=[]byte("renamed");v.Destination=[]byte("changed");v.Title=nil;break}}}
 stages=append(stages,snap())
 if paragraph.Parent()!=nil{
  NewParagraphParser().Close(paragraph,r,pc)
  p:=NewParser().(*parser);for _,v:=range DefaultInlineParsers(){p.addInlineParser(v,nil)}
  p.parseBlock(text.NewBlockReader(source,nil),paragraph,pc)
 }
 stages=append(stages,snap())
 return stages
}
""")

table_bridge=module/'oracle_table.go.txt'
table_bridge.write_text("""
package extension
import("fmt";gast "github.com/yuin/goldmark/ast";"github.com/yuin/goldmark/text";"github.com/yuin/goldmark/parser";"github.com/yuin/goldmark/extension/ast")
func OracleTableParagraph(source []byte,segments []text.Segment,inline bool)any{
 root:=gast.NewDocument();p:=gast.NewParagraph();for _,s:=range segments{p.Lines().Append(s)};root.AppendChild(root,p)
 pc:=parser.NewContext();defaultTableParagraphTransformer.Transform(p,text.NewReader(source),pc)
 if inline {
  gast.Walk(root,func(n gast.Node,entering bool)(gast.WalkStatus,error){if entering && n.Kind()==ast.KindTableCell{parser.OracleParsePrepared(source,n,pc)};return gast.WalkContinue,nil})
  defaultTableASTTransformer.Transform(root,text.NewReader(source),pc)
 }
 var snapshot func(gast.Node)any
 snapshot=func(n gast.Node)any{
  children:=[]any{};for c:=n.FirstChild();c!=nil;c=c.NextSibling(){children=append(children,snapshot(c))}
  result:=map[string]any{"kind":n.Kind().String(),"pos":n.Pos(),"count":n.ChildCount(),"children":children}
  switch v:=n.(type){
  case *gast.Text:result["segment"]=v.Segment;result["raw"]=v.IsRaw();result["soft"]=v.SoftLineBreak();result["hard"]=v.HardLineBreak()
  case *gast.Paragraph:result["lines"]=append([]text.Segment{},v.Lines().Sliced(0,v.Lines().Len())...)
  case *ast.Table:result["alignments"]=v.Alignments
  case *ast.TableHeader:result["alignments"]=v.Alignments
  case *ast.TableRow:result["alignments"]=v.Alignments
  case *ast.TableCell:result["alignment"]=int(v.Alignment);result["lines"]=append([]text.Segment{},v.Lines().Sliced(0,v.Lines().Len())...)
  }
  return result
 }
 escaped:=[]any{};if v:=pc.Get(escapedPipeCellListKey);v!=nil{for _,e:=range v.([]*escapedPipeCell){escaped=append(escaped,map[string]any{"cell":snapshot(e.Cell),"positions":e.Pos,"transformed":e.Transformed})}}
 return map[string]any{"tree":snapshot(root),"paragraph":snapshot(p),"attached":p.Parent()!=nil,"escaped":escaped,"source":fmt.Sprintf("%x",source)}
}
func OracleTable(source []byte,segment text.Segment,alignmentValues []int,kind string)any{
 reader:=text.NewReader(source)
 if kind=="tabledelimiter"{return defaultTableParagraphTransformer.parseDelimiter(segment,reader)}
 var alignments []ast.Alignment
 if alignmentValues!=nil{alignments=make([]ast.Alignment,len(alignmentValues));for i,v:=range alignmentValues{alignments[i]=ast.Alignment(v)}}
 pc:=parser.NewContext()
 row:=defaultTableParagraphTransformer.parseRow(segment,alignments,kind=="tablerowheader",reader,pc)
 cells:=[]any{};ids:=map[*ast.TableCell]int{}
 for n:=row.FirstChild();n!=nil;n=n.NextSibling(){
  c:=n.(*ast.TableCell);ids[c]=len(cells)
  cells=append(cells,map[string]any{"alignment":int(c.Alignment),"pos":c.Pos(),"lines":append([]text.Segment{},c.Lines().Sliced(0,c.Lines().Len())...)})
 }
 var escaped any
 if v:=pc.Get(escapedPipeCellListKey);v!=nil{
  entries:=[]any{}
  for _,entry:=range v.([]*escapedPipeCell){entries=append(entries,map[string]any{"cell":ids[entry.Cell],"positions":entry.Pos,"transformed":entry.Transformed})}
  escaped=entries
 }
 return map[string]any{"cells":cells,"alignments":row.Alignments,"pos":row.Pos(),"count":row.ChildCount(),"escaped":escaped,"source":fmt.Sprintf("%x",source)}
}
""")
binary = build(module, overlay={str(trace_dep/'parser/oracle_links.go'):str(bridge),str(trace_dep/'extension/oracle_table.go'):str(table_bridge)})
from itertools import product
requests=[]
inputs=[b'',b' ',b'\t\n',b'url',b'url tail',b'url)',b')',b'()',b'(a(b)c)',b'(unclosed',b'<url>',b'<>',b'<a b>',b'<a\nb>',b'<unclosed',b'<a\\>b>',b'"title"',b"'title'",b'(title)',b'""',b'"a\n b"',b'"a\n\n b"',b'(a(b)c)',b'"a\\"b"',b'"${x}"',b'"`code`"',b'\n  <url>',b'  "title"', '😀(é)'.encode(),b'\xff',b'url\r\n',b'"a\r\nb"']
for byte in range(256):
    inputs += [b'<a\\'+bytes([byte])+b'b>',b'"a\\'+bytes([byte])+b'b"',b'a'+bytes([byte])+b'b']
for data,kind,padding,force in product(inputs,['destination','title'],[0,2],[False,True]):
    whole=[{'Start':0,'Stop':len(data),'Padding':padding,'ForceNewline':force}] if data else []
    lines=[];offset=0
    # Split only on LF, matching the upstream reader's source segmentation.
    chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:lines.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    for segments in [whole,lines]:requests.append({'Source':data.hex(),'Kind':kind,'Segments':segments})

reference_inputs = [b'', b'[x]: url', b'[x]: <>', b'[x]: url "title"', b'[x]: url\n"title"', b'[x]: url\n"unclosed', b'[x]: url "unclosed', b'[x]: url\n"title" trailing', b'[x]: url "title" trailing', b'[x]: url\nnext', b'[ ]: url', b'[]: url', b'[a[b]]: url', b'[x] url', b'[x]:', b'[x]:\n', b'[x]: url\n\n', b'[a\nb]: <url>', b'[x]: url ""']
for byte in range(256):
    reference_inputs += [b'[a'+bytes([byte])+b'b]: url "title"', b'[x]: a'+bytes([byte])+b'b', b'[x]: url "a'+bytes([byte])+b'b"']
for data,padding,force in product(reference_inputs,[0,2],[False,True]):
    lines=[];offset=0
    chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:lines.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'reference','Segments':lines})

# Exercise the original paragraph transformer on the helper corpus and on
# consecutive definitions, blank lines, title fallbacks and intervening text.
extract_inputs = reference_inputs + [
    a + b'\n' + b for a,b in product(
        [b'[a]: one', b'[a]: one "title"', b'[a]: one\n"unclosed', b'\n[a]: one', b'[a]: one\n"title" trailing'],
        [b'[b]: two', b'[a]: duplicate', b'paragraph', b'\n[b]: two', b'[b]: two\nnext'])
]
for data in extract_inputs:
    lines=[];offset=0
    chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:lines.append({'Start':offset,'Stop':offset+n,'Padding':0,'ForceNewline':False})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'extract','Segments':lines})

import re
fold_source=(dep/'util/unicode_case_folding.gen.go').read_text()
fold_keys=[int(v.strip(),0) for v in re.search(r'var _unicodeCaseFoldingFrom = .*?\{(.*?)\}',fold_source).group(1).split(',') if v.strip()]
normal_inputs=[b'', b'  a \t b\r\n', b'A\vB\fC', 'Straße Σςσ İ K ﬃ'.encode()]
for key in fold_keys:
    normal_inputs += [chr(key).encode(), b' \tA'+chr(key).encode()+b'\r\n B ']
for b in range(256):
    normal_inputs += [bytes([b]), b' A'+bytes([b])+b'B ', b'\xf0'+bytes([b])+b'\x80A']
for data in normal_inputs:
    requests.append({'Source':data.hex(),'Kind':'normalize','Segments':[]})

registry_inputs=[b'',b'A\na\nA',b' a \na\nA',b'\na\n', 'Straße\nSTRASSE\nstraße'.encode(), 'Σ\nς\nσ'.encode()]
for byte in range(256):
    registry_inputs.append(b' A'+bytes([byte])+b'B \na'+bytes([byte])+b'b\nmissing')
for key in fold_keys:
    label=chr(key).encode()
    registry_inputs.append(label+b'\n'+label+b'\nA')
for data in registry_inputs:
    requests.append({'Source':data.hex(),'Kind':'registry','Segments':[]})

from itertools import permutations
for n in range(1,7):
    segments=[{'Start':i*7,'Stop':i*7+1+i%2,'Padding':0,'ForceNewline':False} for i in range(n)]
    for order in permutations(range(n)):
        requests.append({'Source':bytes(order).hex(),'Kind':'labels','Segments':segments})

for n in range(6):
    for ops in product(range(5),repeat=n):
        requests.append({'Source':bytes(ops).hex(),'Kind':'bottoms','Segments':[]})

opening_inputs=[b'',b'[',b'![',b'!',b'![a[b',b'[a\n![b',b'!![a',b'[x]']
for n in range(5):
    opening_inputs += [bytes(v) for v in product(b'[!ab',repeat=n)]
for data in opening_inputs:
    for padding in [0,2]:
        lines=[];offset=0
        chunks=data.split(b'\n')
        for i,chunk in enumerate(chunks):
            n=len(chunk)+(i<len(chunks)-1)
            if n:lines.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':False})
            offset+=n
        requests.append({'Source':data.hex(),'Kind':'openclose','Segments':lines})

parse_inputs=[b'',b'[x]',b'[x]()',b'[x](url)',b'![x](url)',b'[x][a]',b'[x][]',b'[x][missing]',b'[x](bad',b'[x][',b'[[x](u)](v)',b'[![x](u)](v)',b'![a [x](u)](v)',b'[x](u "t")',b'[x](u \'t\')',b'[x](u (t))',b'[x](u "bad)',b'[x](<>)',b'[x](u\n"title")', 'Straße [Straße]'.encode(), b'['+b'a'*1000+b'](u)']
for label,dest,tail in product([b'x',b'a',b'no',b'![x]',b'[x]',b''],[b'',b'u',b'<a b>',b'(a)',b'u "t"',b'u "bad'],[b'',b' tail',b'[a]']):
    parse_inputs.append(b'['+label+b']('+dest+b')'+tail)
for n in range(5):
    parse_inputs += [bytes(v) for v in product(b'[!]',repeat=n)]

for mark,label,dest in product([b'*',b'**',b'_',b'__'],[b'x',b'[a]',b'![a](v)'],[b'(u)',b'[a]',b'']):
    parse_inputs += [mark+b'['+mark+label+mark+b']'+dest+mark, b'['+mark+label+mark+b']'+dest, mark+b'['+label+b']'+dest+mark]

tick=bytes([96])
for content,opener,closer in product([b'[x]',b'[x](u)',b'*a*',b'![x](u)',b' a ',b' ',b'a\nb',b'a'+tick+b'b',b''],[tick,tick*2,tick*3],[b'',tick,tick*2,tick*3]):
    code=opener+content+closer
    parse_inputs += [code,b'['+code+b'](u)',b'*'+code+b'*']

auto_inputs=[b'',b'a@b',b'a@b.c',b'a@-b',b'a@b-',b'a@b..c',b'mailto:a@b',b'http:x',b'a:x',b'ab:',b'foo+.-:bar',b'http:a b',b'a@b:foo']
for byte in range(256):
    auto_inputs += [bytes([byte])+b'a:x', b'ab:'+bytes([byte])+b'x',b'a'+bytes([byte])+b'@b',b'a@b'+bytes([byte])+b'c']
for n in [0,1,2,31,32,33,34,61,62,63,64,65]:
    auto_inputs += [b'a'*n+b':x',b'a@'+b'b'*n,b'a@b'+b'-'*n+b'c',b'a@'+b'b'*n+b'.c',b'a@b'+b'-'*n]
for data in auto_inputs:
    requests.append({'Source':data.hex(),'Kind':'autoindex','Segments':[]})
    parse_inputs.append(b'<'+data+b'>')
for data in [b'<a@b>',b'<http:x>',b'<a@b->']:
    parse_inputs += [b'['+data+b'](u)',b'*'+data+b'*',bytes([96])+data+bytes([96])]

for opener,closer in [(b'<!--',b'-->'),(b'<?',b'?>'),(b'<!DOCTYPE ',b'>'),(b'<![CDATA[',b']]>')]:
    for body,end in product([b'',b'x',b'[x](u)',b'*a*',b'a\nb',b'--',b'$'+b'{x}',b'\xff'],[closer,b'',closer+b'tail']):
        data=opener+body+end
        parse_inputs += [data,b'['+data+b'](u)',b'*'+data+b'*']
parse_inputs += [b'<!-->',b'<!--->',b'<!--a--b-->',b'<!lower>',b'<!A>',b'<![CDATA[x]>']
for data in parse_inputs:
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':0,'ForceNewline':False})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'parse','Segments':segments})

raw_inputs=[b'',b'<!-->',b'<!--->',b'<!--',b'<?',b'<!A',b'<!lower>',b'<![CDATA[',b'<!--a\nb-->',b'<?a\nb?>']
for byte in range(256):
    for opener,closer in [(b'<!--',b'-->'),(b'<?',b'?>'),(b'<!A ',b'>'),(b'<![CDATA[',b']]>')]:
        raw_inputs += [opener+b'a'+bytes([byte])+b'b'+closer,opener+bytes([byte])]
for data,padding,force in product(raw_inputs,[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'rawdelimited','Segments':segments})

tag_inputs=[b'',b'<a>',b'</a>',b'<a/>',b'<a b=c>',b'<a b="c">',b"<a b='c'>",b'<a\r>',b'<a\r\n>',b'<a b>',b'<a b=>',b'<1>',b'<a-b>',b'<a:b>',b'<a b=c/>tail']
for byte in range(256):
    tag_inputs += [b'<'+bytes([byte])+b'a>',b'<a'+bytes([byte])+b'>',b'<a '+bytes([byte])+b'=x>',b'<a b='+bytes([byte])+b'>',b'<a b="x'+bytes([byte])+b'y">']
for name,space,attribute,ending in product([b'a',b'A-b',b'1a'],[b' ',b'\r',b'\r\n',b'\n\n',b'\t'],[b'b',b'b=c',b'b = "x y"',b"b='x\ny'",b'b =',b'b=c d=e',b':_=x'],[b'>',b'/>',b' >',b'\r>',b'']):
    tag_inputs.append(b'<'+name+space+attribute+ending)
for data,kind in product(tag_inputs,['tagopen','tagclose']):
    if kind=='tagclose' and data.startswith(b'<') and not data.startswith(b'</'):data=b'</'+data[1:]
    requests.append({'Source':data.hex(),'Kind':kind,'Segments':[]})

reader_tags=tag_inputs[:15]+[b'<a>tail',b'</a>tail',b'<a x="hi\nthere">tail',b'<a x="unterminated',b'<a x="'+bytes.fromhex('efbfbd')+b'">',b'not a tag']
reader_tags += [b'<a x="'+bytes([v])+b'">tail' for v in range(256)]
reader_tags += [b'</'+v[1:] for v in reader_tags if v.startswith(b'<') and not v.startswith(b'</')]
for data,closing,padding,force in product(reader_tags,[False,True],[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'tagreadclose' if closing else 'tagreadopen','Segments':segments})

raw_tag_inputs=tag_inputs+[b'</'+v[1:] for v in tag_inputs if v.startswith(b'<') and not v.startswith(b'</')]
for data,padding,force in product(raw_tag_inputs,[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':'rawtag','Segments':segments})

inline_inputs=[b'',b'a',b'a\n',b'a \n',b'a  \n',b'a\\\n',b'a\\\r\n',b'a  \r\n',b'a\\\\\n',b'\n',b'\r\n',b'\\*x*',b'\\[x]',b'~~x~~',b'[~~x~~](u)',b'**a**',b'<a x="y">z</a>',bytes([96])+b'a'+bytes([96]),b'a\n\nb']
inline_inputs += parse_inputs[:60]
for first,ending,second in product([b'a',b'*a*',b'[x]',bytes([96])+b'a'+bytes([96]),b'<a@b>',b'\\'],[b'\n',b' \n',b'  \n',b'\\\n',b'\\\\\n',b'\r\n',b'  \r\n'],[b'b',b'*b*',b'[a]',b'',b'~~b~~']):
    inline_inputs.append(first+ending+second)
for data,kind,padding,force in product(inline_inputs,['inline','inlineescaped','inlinestrike','inlineraw'],[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':kind,'Segments':segments})

paragraph_inputs=[b'',b'a',b' a ',b' a\n b ',b'a\n\nb',b' \t\r\n',b'\v',b'\f',b'[x]: u\ntext',b'  a  \n  b  ',b'a\r\nb\r\n']
for byte in range(256):
    paragraph_inputs += [b' '+bytes([byte])+b' \n next ',bytes([byte])]
for data,kind,padding,force in product(paragraph_inputs,['paragraph','paragraphclear'],[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':kind,'Segments':segments})

reference_paragraph_inputs=[b'',b'[x]: u',b'[x]: u\n[x]',b'[x]: u\n[x]: v\n[x]',b' [x]: <u> "t"\n [x][]',b'[x]: u\n"bad\n[x]',b'[x]: u\n"title" trailing\n[x]',b'[x]: u\n[a]: v\n[x][a]',b'normal [x]', '[Straße]: u\n[STRASSE]'.encode()]
for label,destination,tail in product([b'x',b'X',b' x ',b'a'],[b'u',b'<>',b'u "title"',b'u (title)',b'u "bad'],[b'',b'\n[x]',b'\n[x]: duplicate\n[x]',b'\ntext [x](v)',b'\n![x]']):
    reference_paragraph_inputs.append(b'['+label+b']: '+destination+tail)
for data,kind,padding,force in product(reference_paragraph_inputs,['refparagraph','refparagraphmutate'],[0,2],[False,True]):
    segments=[];offset=0;chunks=data.split(b'\n')
    for i,chunk in enumerate(chunks):
        n=len(chunk)+(i<len(chunks)-1)
        if n:segments.append({'Start':offset,'Stop':offset+n,'Padding':padding,'ForceNewline':force})
        offset+=n
    requests.append({'Source':data.hex(),'Kind':kind,'Segments':segments})

table_inputs=[b'',b'-',b'---',b'---\n',b'|---|',b':--|--:|:--:',b'  --- | ---',b'    --- | ---',b'||',b'a|b|c',b'| a | b |',b' a \n',b'a\\|b|c',bytes([96])+b'a\\|b'+bytes([96])+b'|c',b'a|',b'|a',b'|',b'a||b']
for byte in range(256):
    table_inputs += [b'--'+bytes([byte])+b'--|:--:',b'a'+bytes([byte])+b'|b']
for data,padding,force in product(table_inputs,[0,2],[False,True]):
    segment={'Start':0,'Stop':len(data),'Padding':padding,'ForceNewline':force}
    requests.append({'Source':data.hex(),'Kind':'tabledelimiter','Segments':[segment]})
    for kind,alignments in product(['tablerow','tablerowheader'],[None,[],[1],[1,2,3]]):
        requests.append({'Source':data.hex(),'Kind':kind,'Segments':[segment],'Alignments':alignments})
for prefix,header,delimiter,body in product([b'',b'intro\n'],[b'a|b\n',b'a|b|c\n',b'|a|b|\n',bytes([96])+b'a\\|b'+bytes([96])+b'|c\n'],[b'-|-\n',b':-|-:\n',b'---\n',b'bad\n-|-\n'],[b'',b'c|d\n',b'c\n',b'c|d|e\n']):
    data=prefix+header+delimiter+body
    for padding,force in product([0,2],[False,True]):
        segments=[];offset=0
        for line in data.splitlines(keepends=True):
            segments.append({'Start':offset,'Stop':offset+len(line),'Padding':padding,'ForceNewline':force});offset+=len(line)
        requests.append({'Source':data.hex(),'Kind':'tableparagraph','Segments':segments})
for cell,other,body in product([b'a',bytes([96])+b'a\\|b'+bytes([96]),bytes([96])+b'a\\|b\\|c'+bytes([96]),b'a\\|b',bytes([96])+b'\\|'+bytes([96]),bytes([96])+b'unclosed\\|b'],[b'd',b'd\\|e'],[b'',b'f|g\n']):
    data=cell+b'|'+other+b'\n-|-\n'+body
    for padding,force in product([0,2],[False,True]):
        segments=[];offset=0
        for line in data.splitlines(keepends=True):
            segments.append({'Start':offset,'Stop':offset+len(line),'Padding':padding,'ForceNewline':force});offset+=len(line)
        requests.append({'Source':data.hex(),'Kind':'tableinline','Segments':segments})
block_inputs=[b'',b'>',b'>\n',b'>\ttext\n',b' > text',b'    >x',b'---',b'* * *',b'___\n',b'-_*',b'\t---']
for byte in range(256):block_inputs += [b'>'+bytes([byte])+b'x\n',b'-'+bytes([byte])+b'--\n']
for data,padding,offset,kind in product(block_inputs,[0,1,3],[0,1,2],['blockquote','thematic']):
    source=b'x'*offset+data
    requests.append({'Source':source.hex(),'Kind':kind,'Segments':[{'Start':offset,'Stop':len(source),'Padding':padding,'ForceNewline':False}]})
code_inputs=[b'',b'x',b'    x',b'\tx',b'   \tx',b'    \tx',b'    x\n\n    y\n\nplain',b'    x\n  \n\ty',b'    x\n    ']
for byte in range(256):code_inputs += [b'    '+bytes([byte])+b'\n',b'    x\n'+bytes([byte])+b'   y\n']
for data,padding,offset in product(code_inputs,[0,1,3],[0,1,2]):
    source=b'x'*offset+data;segments=[];start=offset
    for line in data.splitlines(keepends=True):
        segments.append({'Start':start,'Stop':start+len(line),'Padding':padding,'ForceNewline':False});start+=len(line)
    if not segments:segments=[{'Start':offset,'Stop':offset,'Padding':padding,'ForceNewline':False}]
    requests.append({'Source':source.hex(),'Kind':'codeblock','Segments':segments})
for mark,indent,info,body in product([bytes([96])*3,b'~~~',b'~~'],[b'',b' ',b'   ',b'\t'],[b'',b' rust',b' x'+bytes([96])+b'y'],[b'body\n',b'\tbody\n',b'  body\n',b'\n',b'    \n']):
    data=indent+mark+info+b'\n'+body+mark+b'\n'
    for padding in [0,1,3]:
        segments=[];offset=0
        for line in data.splitlines(keepends=True):
            segments.append({'Start':offset,'Stop':offset+len(line),'Padding':padding,'ForceNewline':False});offset+=len(line)
        requests.append({'Source':data.hex(),'Kind':'fenceblock','Segments':segments})
for byte in range(256):
    for prefix in [b'',b' ',b'   ',b'    ',b'\t']:
        for data in [b'=='+bytes([byte]),b'--'+bytes([byte]),bytes([byte])+b'--',b'---\n']:
            requests.append({'Source':(prefix+data).hex(),'Kind':'setextbar','Segments':[]})
for count,tail,padding in product(range(1,9),[b'',b'\n',b' title',b' title ###',b' ### ',b'x',b'\ttext #',b'  a#',b'  a ## x'],[0,1,3]):
    data=b'#'*count+tail
    requests.append({'Source':data.hex(),'Kind':'atx','Segments':[{'Start':0,'Stop':len(data),'Padding':padding,'ForceNewline':False}]})
for byte,padding,offset in product(range(256),[0,1,3],[0,2]):
    for data in [b'#'+bytes([byte])+b'title',b'## title'+bytes([byte])+b'###',b'### '+bytes([byte])+b' #']:
        source=b'x'*offset+data
        requests.append({'Source':source.hex(),'Kind':'atx','Segments':[{'Start':offset,'Stop':len(source),'Padding':padding,'ForceNewline':False}]})
list_inputs=[b'',b'-',b'* x',b'+\t',b'1.',b'1. x',b'0) x',b'123456789. x',b'1234567890. x',b'    - x',b'\t- x',b'-\n',b'-  x\n',b'-\t x\n',b'1) x\n',b'1)    x\n']
for byte in range(256):
    list_inputs += [b'-'+bytes([byte]), b'1.'+bytes([byte]), b'  - '+bytes([byte])+b'\n']
for data in list_inputs:
    requests.append({'Source':data.hex(),'Kind':'listitem','Segments':[]})
html_inputs=[b'<!-- one\n two -->\n',b'<?pi\nmore ?>\n',b'<!DOCTYPE html>\n',b'<![CDATA[\nbody\n]]>\n',b'<script>\nbody\n</script>\n',b'<div>\nbody\n\n',b'<custom>\nbody\n\n',b'<x>\nbody\n',b'<x class>\n',b'<x class="v">\n',b'<x class=\'v\'>\n',b'<x class=v>\n',b'<x a=\"\">\n',b'<x a=\'\'>\n',b'<x a = v>\n',b'<x a=bad> trailing\n',b'<x a=`bad`>\n',b'<x a=>\n',b'<x a b>\n',b'<x/>\n',b'<x />   \n',b'</ x>\n',b'</x class>\n',b'<div\tclass>\n',b'<div\x00>\n',b'<scriptish>\n',b'<script /x>\n',b'<SCRIPT>\n',b'<SCRIPT>\n</ScRiPt>\n',b'<div> trailing\n',b'    <x>\n',b'   <x>\n',b'    <div>\n',b'<!A\x00>\n',b'<!a>\n']
for byte in range(256):
    html_inputs += [b'<x'+bytes([byte])+b'>\n', b'<div '+bytes([byte])+b'>\n', b'<!--'+bytes([byte])+b'-->\n']
for data,padding in product(html_inputs,[0,1,3]):
    segments=[];offset=0
    for line in data.splitlines(keepends=True):
        segments.append({'Start':offset,'Stop':offset+len(line),'Padding':padding,'ForceNewline':False});offset+=len(line)
    requests.append({'Source':data.hex(),'Kind':'htmlblock','Segments':segments})
for bar,padding,mode in product([b'---\n',b'===\n',b' - \n',b' ===',b'bad',b'    ---'],[0,1,3],['setextlife','setextempty','setextnext','setextnextempty']):
    data=b'title\n'+bar
    requests.append({'Source':data.hex(),'Kind':mode,'Segments':[{'Start':0,'Stop':6,'Padding':0,'ForceNewline':False},{'Start':6,'Stop':len(data),'Padding':padding,'ForceNewline':False}]})
requests=list({json.dumps(q,sort_keys=True):q for q in requests}.values())
results = json.loads(subprocess.run([str(binary)], input=json.dumps(requests).encode(), stdout=subprocess.PIPE, check=True).stdout)
assert len(results) == len(requests)
write_request_fixture('crates/d2-rs-text/tests/fixtures/link.json', requests, results)
print(f'{len(requests)} Goldmark link-value requests captured')
