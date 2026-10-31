// SPDX-License-Identifier: MPL-2.0
// Oracle for crates/d2-rs-render/tests/fixtures/legend-connections-source-v090.json
// (docs/FIXTURES.md): legend connection icon scenes for every arrowhead x
// stroke width x dash/animation mode through public d2scenebuild.Build.
// Harvested from .reference/legend-connections-20261016/source-module.
// Overlaid into d2renderers/d2scenebuild as an external test (public APIs only); inert unless
// D2RS_RECIPE_OUTPUT is set.
package d2scenebuild_test
import("context";"testing";"encoding/json";"fmt";"os";"strings";"github.com/d2lang/d2/d2renderers/d2scene";"github.com/d2lang/d2/d2renderers/d2scenebuild";"github.com/d2lang/d2/d2target")
func TestD2RSLegendConnections(t *testing.T) {
	outputPath := os.Getenv("D2RS_RECIPE_OUTPUT")
	if outputPath == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	stdout, err := os.Create(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	defer stdout.Close()
rows:=[]any{};for _,kind:=range []d2target.Arrowhead{d2target.NoArrowhead,d2target.TriangleArrowhead,d2target.ArrowArrowhead,d2target.UnfilledTriangleArrowhead,d2target.DiamondArrowhead,d2target.FilledDiamondArrowhead,d2target.CircleArrowhead,d2target.FilledCircleArrowhead,d2target.CrossArrowhead,d2target.BoxArrowhead,d2target.FilledBoxArrowhead,d2target.LineArrowhead,d2target.CfOne,d2target.CfMany,d2target.CfOneRequired,d2target.CfManyRequired}{for _,width:=range []int{0,1,2,4}{for mode:=0;mode<6;mode++{
 d:=d2target.NewDiagram();d.Root.Fill="#ffffff";d.Root.Stroke="none";s:=*d2target.BaseShape();s.ID="host";s.Type=d2target.ShapeRectangle;s.Pos=d2target.Point{X:10,Y:20};s.Width=100;s.Height=80;d.Shapes=[]d2target.Shape{s};c:=*d2target.BaseConnection();c.ID="edge";c.Label="edge";c.Stroke="B1";c.StrokeWidth=width;c.StrokeDash=float64((mode%3)*2);c.Opacity=1;c.Animated=mode>=3;if mode%3!=0{c.SrcArrow=kind};if mode%3!=1{c.DstArrow=kind};d.Legend=&d2target.Legend{Connections:[]d2target.Connection{c}};pad:=int64(0);doc,err:=d2scenebuild.Build(context.Background(),d,d2scenebuild.Options{Pad:&pad});if err!=nil{panic(err)}
 nodes:=[]any{};var walk func(*d2scene.Node);walk=func(n *d2scene.Node){if strings.HasPrefix(n.ID,"legend:connection:0"){samples:=[]any{};for _,track:=range n.Animations {for _,t:=range []int64{0,1,33333333,123456789,int64(track.Duration)-1,int64(track.Duration),int64(track.Duration)+1}{local:=t%int64(track.Duration);progress:=float64(local)/float64(track.Duration);a:=track.Keyframes[0].Value.Number;z:=track.Keyframes[len(track.Keyframes)-1].Value.Number;value:=a+(z-a)*progress;samples=append(samples,map[string]any{"time_ns":t,"value":value})}};nodes=append(nodes,map[string]any{"dash_samples":samples,"id":n.ID,"transform":n.Transform,"opacity":n.Opacity,"primitive":n.Primitive,"animations":n.Animations})};for _,child:=range n.Children{walk(child)}};walk(doc.Root);rows=append(rows,map[string]any{"name":fmt.Sprintf("%s-w%d-mode%d",kind,width,mode),"connection":c,"nodes":nodes})
 }}};enc:=json.NewEncoder(stdout);enc.SetIndent("","  ");if err:=enc.Encode(rows);err!=nil{panic(err)}}
