// SPDX-License-Identifier: MPL-2.0
//! Native replay of original resource encoding and transport lifetime calls.
use base64::Engine as _;
use d2_rs_core::CancellationToken;
use d2_rs_scenebuild::{
    ImageHttpClient, ImageHttpResponse, ImageKind, ImageLocalFiles, ImageResolveError,
    ImageResource, NativeImageHttpClient, RasterImageResolver, ResolvedImageAsset,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn uri(op: &Value) -> Value {
    if op["nil"] == true {
        // Rust represents the absent Resource as Option, not a nil receiver.
        return json!({"data":null,"error":"imageasset: nil resource"});
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(op["data"].as_str().unwrap())
        .unwrap();
    let r = ResolvedImageAsset {
        resource: ImageResource {
            kind: ImageKind::Svg,
            mime_type: op["mime"].as_str().unwrap().into(),
            data: Arc::from(bytes),
            pixel_width: 0,
            pixel_height: 0,
            decoded_bytes: 0,
        },
        fetched_bytes: 0,
        decompressed_bytes: 0,
    };
    let token = CancellationToken::new();
    if op["canceled"] == true {
        token.cancel();
    }
    match r.data_uri(Some(&token)) {
        Ok(mut bytes) => {
            let got = base64::engine::general_purpose::STANDARD.encode(&bytes);
            if !bytes.is_empty() {
                bytes[0] ^= 255;
            }
            // The returned allocation must not alias the resource.
            assert_eq!(
                base64::engine::general_purpose::STANDARD.encode(r.data_uri(Some(&token)).unwrap()),
                got
            );
            json!({"data":got,"error":""})
        }
        Err(e) => json!({"data":null,"error":e.message}),
    }
}
#[derive(Debug, Default)]
struct Tracking(Arc<AtomicUsize>);
impl ImageHttpClient for Tracking {
    fn close_idle_connections(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn get(
        &self,
        _: &str,
        _: i64,
        _: Option<&CancellationToken>,
    ) -> Result<ImageHttpResponse, ImageResolveError> {
        panic!("transport lifetime replay must not fetch")
    }
}
fn replay(case: &Value) -> Value {
    let mut sessions = HashMap::new();
    let mut observations = Vec::new();
    for op in case["operations"].as_array().unwrap() {
        let id = op["id"].as_u64().unwrap_or(0);
        observations.push(match op["op"].as_str().unwrap() {
            "data-uri" => uri(op),
            "new" => {
                let mut r = RasterImageResolver::new(
                    Path::new("."),
                    serde_json::from_value(op["limits"].clone()).unwrap(),
                    Arc::new(ImageLocalFiles::default()),
                )
                .unwrap();
                let mut tracker = None;
                // Go's trusted custom RoundTripper corresponds to the native
                // caller-owned client boundary. A default std Transport under
                // public policy gets an independent native default client;
                // this finite source witness supplies no configured hooks.
                let isolated = if op["transport"] == "tracking" && op["allow_private"] == true {
                    let client = Arc::new(Tracking::default());
                    tracker = Some(client.0.clone());
                    let shared = client.clone();
                    let isolated = !Arc::ptr_eq(&client, &shared);
                    r = r.with_http_client(shared);
                    isolated
                } else if op["transport"] == "standard" {
                    let caller: Arc<dyn ImageHttpClient> =
                        Arc::new(NativeImageHttpClient::default());
                    let isolated: Box<dyn ImageHttpClient> =
                        Box::new(NativeImageHttpClient::default());
                    let distinct = !std::ptr::eq(caller.as_ref(), isolated.as_ref());
                    r = r.with_owned_http_client(isolated);
                    distinct
                } else {
                    assert_eq!(op["transport"], "default");
                    true
                };
                let got = json!({"error":"", "owned":r.owns_http_transport(), "isolated":isolated});
                sessions.insert(id, (r, tracker));
                got
            }
            "close" => {
                if let Some((r, tracker)) = sessions.get(&id) {
                    r.close_idle_connections();
                    match tracker {
                        Some(count) => json!({"closed":count.load(Ordering::SeqCst)}),
                        None => json!({}),
                    }
                } else {
                    assert_eq!(id, 0);
                    // Option::None is the Rust equivalent of a nil receiver.
                    json!({})
                }
            }
            other => panic!("unknown operation {other}"),
        });
    }
    json!(observations)
}
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    println!("{}", json!(cases.iter().map(replay).collect::<Vec<_>>()));
}
