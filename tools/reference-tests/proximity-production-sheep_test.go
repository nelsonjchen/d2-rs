package proximity_test

import (
    "context"
    "encoding/json"
    "fmt"
    "sort"
    "testing"
    "github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
    "github.com/d2lang/d2/d2layouts/d2talalayout/internal/proximity"
    "github.com/d2lang/d2/lib/geo"
)

func TestProductionSheepControls(t *testing.T) {
    for _, name := range []string{"ordered-duplicates", "current-leaf", "invalid-orientation", "empty-uncle"} {
        graph := layoutgraph.NewGraph()
        add := func(id int64, name string, parent *layoutgraph.Node) *layoutgraph.Node {
            node := layoutgraph.NewNode(layoutgraph.EntityID(id), 200, 120)
            node.D2ID = &name
            node.Container = parent
            graph.AddNewNodeToContainer(parent, node)
            return node
        }
        local := add(0,"local",nil)
        uncle := add(1,"uncle",nil)
        first := add(2,"first",local)
        second := add(3,"second",local)
        cousin := add(4,"cousin",uncle)
        local.SetContainer(true)
        uncle.SetContainer(true)
        graph.Containers[local] = []*layoutgraph.Node{second,first}
        a := &layoutgraph.EdgeAbduction{OriginallyFrom:first,OriginallyTo:cousin,CurrentFrom:local,CurrentTo:uncle}
        b := &layoutgraph.EdgeAbduction{OriginallyFrom:second,OriginallyTo:cousin,CurrentFrom:local,CurrentTo:uncle}
        records := []*layoutgraph.EdgeAbduction{a,b,a}
        if name == "current-leaf" { a.CurrentTo=cousin; records=records[:2] }
        grouped, cousins, err := proximity.GroupSheep(context.Background(),graph,local,records)
        if err != nil { t.Fatal(err) }
        uncles := make([]*layoutgraph.Node,0,len(grouped))
        for uncle := range grouped { uncles=append(uncles,uncle) }
        sort.Slice(uncles,func(i,j int) bool{return uncles[i].ID<uncles[j].ID})
        groups := make([]any,0,len(uncles))
        for _, uncle := range uncles {
            nodes := make([]any,0,len(grouped[uncle]))
            for _, node := range grouped[uncle] {
                names := make([]string,0,len(cousins[uncle][node]))
                for _, cousin := range cousins[uncle][node] {names=append(names,cousin.DebugID())}
                nodes=append(nodes,map[string]any{"node":node.DebugID(),"cousins":names})
            }
            groups=append(groups,map[string]any{"uncle":uncle.DebugID(),"nodes":nodes})
        }
        if name == "invalid-orientation" { cousin.TopLeft=&geo.Point{}; cousin.HerdAssignment=layoutgraph.NewHerdAssignment() }
        if name == "empty-uncle" {graph.Containers[uncle]=nil}
        var message any
        if err := proximity.AssignHerds(context.Background(),graph,local,records); err != nil {message=err.Error()}
        encoded,err := json.Marshal(map[string]any{"case":name,"groups":groups,"error":message})
        if err != nil {t.Fatal(err)}
        fmt.Printf("PRODUCTION_SHEEP_CONTROL %s\n",encoded)
    }
}
