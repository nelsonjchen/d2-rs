// SPDX-License-Identifier: MPL-2.0
//! Replay source HTTP operations against a real loopback endpoint at the captured
//! address. No expected resources, policy decisions or errors enter this process.
use base64::Engine as _;
use d2_rs_scenebuild::{
    ImageKind, ImageLocalFiles, ImageMemoryCache, NativeImageHttpClient, RasterImageResolver,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn main() {
    let input: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let fixture = &input["fixture"];
    let url = url::Url::parse(fixture["url"].as_str().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let server = TcpListener::bind(("127.0.0.1", url.port().unwrap())).unwrap();
    server.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server_hits = hits.clone();
    let server_stop = stop.clone();
    let body = base64::engine::general_purpose::STANDARD
        .decode(fixture["body"].as_str().unwrap())
        .unwrap();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(45);
        while !server_stop.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "server deadline");
            match server.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        header.push(byte[0]);
                        assert!(header.len() < 8192);
                    }
                    assert!(header.starts_with(b"GET /private HTTP/1.1\r\n"));
                    server_hits.fetch_add(1, Ordering::SeqCst);
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                    stream.write_all(&body).unwrap();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
    });
    let mut rows = Vec::new();
    for case in input["cases"].as_array().unwrap() {
        let mut sessions = HashMap::new();
        let mut caches = HashMap::<u64, Arc<ImageMemoryCache>>::new();
        let mut session_caches = HashMap::new();
        let mut got = Vec::new();
        for op in case["operations"].as_array().unwrap() {
            let id = op["id"].as_u64().unwrap();
            got.push(match op["op"].as_str().unwrap(){
    "new"=>{
     let private=op["private"].as_bool().unwrap();
     let mut result=RasterImageResolver::new(Path::new(""),serde_json::from_value(op["limits"].clone()).unwrap(),Arc::new(ImageLocalFiles::default())).map(|r|r.with_http_client(Arc::new(NativeImageHttpClient{allow_private_networks:private})));
     let cache_id=op["cache"].as_u64().unwrap();if cache_id!=0{let cache=caches.entry(cache_id).or_insert_with(||Arc::new(ImageMemoryCache::new(op["cache_entries"].as_i64().unwrap(),op["cache_bytes"].as_i64().unwrap()).unwrap()));result=result.and_then(|r|r.with_shared_cache(cache.clone(),op["namespace"].as_str().unwrap(),private));session_caches.insert(id,cache.clone());}
     let error=result.as_ref().err().map(ToString::to_string).unwrap_or_default();if let Ok(r)=result{sessions.insert(id,r);}json!({"error":error})
    },
    "resolve"=>{
     let source=op["source"].as_str().unwrap();assert!(source.starts_with(fixture["url"].as_str().unwrap()));
     let result=sessions[&id].resolve_with_context(source,None);
     let mut value=match result{
      Ok(r)=>json!({"error":"","unavailable":false,"resource":{"kind":match r.resource.kind{ImageKind::Raster=>"raster".to_owned(),ImageKind::Svg=>"svg".to_owned(),ImageKind::Other(k)=>format!("Kind({k})")},"mime":r.resource.mime_type,"width":r.resource.pixel_width,"height":r.resource.pixel_height,"decoded":r.resource.decoded_bytes,"data":base64::engine::general_purpose::STANDARD.encode(&r.resource.data)}}),
      Err(e)=>json!({"error":e.message,"unavailable":e.unavailable,"resource":null})};
     value["hits"]=json!(hits.load(Ordering::SeqCst));if let Some(cache)=session_caches.get(&id){value["cache_entries"]=json!(cache.usage().entries);value["cache_bytes"]=json!(cache.usage().encoded);}value
    },other=>panic!("unknown op {other}")
   });
        }
        rows.push(got);
    }
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    println!("{}", json!(rows));
}
