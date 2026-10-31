package main
import ("context"; "encoding/json"; "net/http"; "net/http/httptest"; "os"; "sync/atomic")
func main() {
 var requests atomic.Int32
 server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){ requests.Add(1) }))
 defer server.Close()
 ctx,cancel := context.WithCancel(context.Background()); cancel()
 req,_ := http.NewRequestWithContext(ctx,"GET",server.URL+"/cancelled.svg",nil)
 _,err := (&http.Client{}).Do(req)
 json.NewEncoder(os.Stdout).Encode(map[string]any{"error":err.Error(),"requests":requests.Load(),"url":server.URL+"/cancelled.svg"})
}
