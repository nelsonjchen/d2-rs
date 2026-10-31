// SPDX-License-Identifier: MPL-2.0
//! Replay source HTTP environments through the native policy-owned client.
use d2_rs_scenebuild::{netpolicy::BlockedAddress, netpolicy_client::*};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};
struct FixtureResolver {
    answers: BTreeMap<String, Vec<String>>,
    calls: Arc<Mutex<Vec<Value>>>,
}
impl reqwest::dns::Resolve for FixtureResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push(json!({"network":"ip","host":host}));
        let result = self.answers.get(&host).cloned();
        Box::pin(async move {
            let answers = result.ok_or("unexpected test hostname")?;
            let addresses: Vec<SocketAddr> = answers
                .iter()
                .map(|s| SocketAddr::new(s.parse().unwrap(), 0))
                .collect();
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
struct Custom(Arc<AtomicBool>);
impl RoundTripper for Custom {
    fn round_trip(&self, _: reqwest::Request) -> HttpFuture<'_> {
        self.0.store(true, Ordering::SeqCst);
        Box::pin(async { Err("stopped by test".into()) })
    }
}
fn run(case: &Value) -> Value {
    let called = Arc::new(AtomicBool::new(false));
    let lookups = Arc::new(Mutex::new(Vec::<Value>::new()));
    let dials = Arc::new(Mutex::new(Vec::<String>::new()));
    let port = Arc::new(AtomicU16::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(Mutex::new(Vec::<String>::new()));
    let server = case["server"].as_str().unwrap();
    let server_address = if server.is_empty() {
        None
    } else {
        Some(server.parse::<SocketAddr>().unwrap())
    };
    let server_thread = server_address.map(|address| {
        let listener = TcpListener::bind(address).unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = done.clone();
        let hits = hits.clone();
        let routes = case["routes"].clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 65536 {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                hits.lock().unwrap().push(path.to_owned());
                let route = &routes[path];
                let body = route["body"].as_str().unwrap_or("missing fixture route");
                let code = route["status"].as_u64().unwrap_or(404);
                let location = route["location"].as_str().unwrap_or("");
                let mut response = format!(
                    "HTTP/1.1 {code} Test\r\nConnection: close\r\nContent-Length: {}\r\n",
                    body.len()
                );
                if !location.is_empty() {
                    response.push_str(&format!("Location: {location}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(body);
                stream.write_all(response.as_bytes()).unwrap();
            }
        })
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut client = None;
    let mut output = Vec::new();
    runtime.block_on(async {
        for operation in case["operations"].as_array().unwrap() {
            match operation["op"].as_str().unwrap() {
                "new" => {
                    let custom:Arc<dyn RoundTripper>=Arc::new(Custom(called.clone()));
                    let transport=match operation["transport"].as_str().unwrap() {
                        "custom" => Transport::Custom(custom.clone()),
                        "standard" => {
                            let called=called.clone();
                            Transport::Standard(Arc::new(move |builder| {
                                // An arbitrary caller transport hook must never run under public-only policy.
                                called.store(true,Ordering::SeqCst);
                                builder.proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
                            }))
                        },
                        "default" => Transport::Default,
                        other => panic!("{other}"),
                    };
                    let options=HttpClientOptions{transport, ..Default::default()};
                    let before=match &options.transport {Transport::Standard(hook)=>Some(hook.clone()),_=>None};
                    let io=if operation["injected"]==true {
                        let resolver=FixtureResolver{answers:serde_json::from_value(case["answers"].clone()).unwrap(),calls:lookups.clone()};
                        let dials=dials.clone();let port=port.clone();
                        Some(ClientIo {resolver:Arc::new(resolver),dial:Arc::new(move |address| {
                            dials.lock().unwrap().push(SocketAddr::new(address.ip(),port.load(Ordering::SeqCst)).to_string());
                            Ok(server_address.expect("injected policy dial server"))
                        })})
                    } else { None };
                    let result=new_http_client_with_io(Some(&options), Policy{allow_private_networks:operation["allow_private"]==true}, io);
                    let preserved=match (&options.transport,before) {(Transport::Standard(after),Some(before))=>Arc::ptr_eq(after,&before),_=>true};
                    output.push(json!({"accepted":result.is_ok(),"custom_preserved":result.as_ref().is_ok_and(|c|c.uses_custom_transport(&custom)),"base_preserved":preserved}));
                    client=result.ok();
                },
                "dial" => {
                    let cancel=d2_rs_core::CancellationToken::new(); cancel.cancel();
                    let result=client.as_ref().unwrap().get(&format!("http://{}/",operation["address"].as_str().unwrap()),Some(&cancel)).await;
                    output.push(json!({"error_kind":if result.unwrap_err()=="context canceled" {"cancelled"}else{"other"}}));
                },
                "get" => {
                    let url=operation["url"].as_str().unwrap();
                    port.store(reqwest::Url::parse(url).unwrap().port_or_known_default().unwrap(),Ordering::SeqCst);
                    let result=client.as_ref().unwrap().get(url,None).await;
                    let (kind,body)=match result {
                        Ok(response)=>("",Some(String::from_utf8(response.bytes().await.unwrap().to_vec()).unwrap())),
                        Err(error)=>(if error==BlockedAddress.to_string(){"blocked"}else{"other"},None),
                    };
                    let mut got=json!({"error_kind":kind,"lookups":*lookups.lock().unwrap(),"dials":*dials.lock().unwrap(),"hits":*hits.lock().unwrap()});
                    if let Some(body)=body {got["body"]=json!(body);}
                    output.push(got);
                },
                "hooks" => output.push(json!({"called":called.load(Ordering::SeqCst)})),
                other => panic!("{other}"),
            }
        }
    });
    done.store(true, Ordering::SeqCst);
    if let Some(thread) = server_thread {
        thread.join().unwrap();
    }
    json!(output)
}
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let results: Vec<Value> = request["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            if request["baseline"] == true {
                json!({"unsupported":"native caller-client ownership API unavailable before L24"})
            } else {
                run(case)
            }
        })
        .collect();
    println!("{}", json!(results));
}
