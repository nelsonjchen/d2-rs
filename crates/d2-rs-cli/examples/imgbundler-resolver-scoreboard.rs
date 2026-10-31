// SPDX-License-Identifier: MPL-2.0
//! Raw test I/O at the imageasset boundary; all resolution, MIME probing,
//! budgets, worker scheduling and replacement run in the native libraries.
use base64::Engine as _;
use d2_rs_cli::imgbundler::{self, Budget, Options};
use d2_rs_core::CancellationToken;
use d2_rs_scenebuild::{
    ImageFileReader, ImageHttpClient, ImageHttpResponse, ImageMemoryCache, ImageResolveError,
    RasterImageResolver,
};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Files(Value);
impl ImageFileReader for Files {
    fn read(
        &self,
        path: &Path,
        _: i64,
        _: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, ImageResolveError> {
        Ok(base64::engine::general_purpose::STANDARD
            .decode(
                self.0[path.to_str().unwrap()]
                    .as_str()
                    .expect("captured local input"),
            )
            .unwrap())
    }
}
#[derive(Debug)]
struct Http {
    request: Value,
    calls: AtomicUsize,
}
impl ImageHttpClient for Http {
    fn get(
        &self,
        url: &str,
        _: i64,
        _: Option<&CancellationToken>,
    ) -> Result<ImageHttpResponse, ImageResolveError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let response = self.request["responses"]
            .get(url)
            .unwrap_or(&self.request["default_response"]);
        assert!(!response.is_null(), "uncaptured HTTP request {url}");
        Ok(ImageHttpResponse {
            status: response["status"].as_u64().unwrap() as u16,
            content_length: response["length"].as_i64().unwrap(),
            headers: response["headers"]
                .as_object()
                .unwrap()
                .iter()
                .flat_map(|(name, values)| {
                    values
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(move |v| (name.clone(), v.as_str().unwrap().to_owned()))
                })
                .collect(),
            body: base64::engine::general_purpose::STANDARD
                .decode(response["body"].as_str().unwrap())
                .unwrap(),
        })
    }
}
fn execute(request: Value) -> Value {
    let http = Arc::new(Http {
        request: request.clone(),
        calls: AtomicUsize::new(0),
    });
    let mut resolver = RasterImageResolver::new(
        Path::new(request["base_dir"].as_str().unwrap()),
        serde_json::from_value(request["limits"].clone()).unwrap(),
        Arc::new(Files(request["files"].clone())),
    )
    .unwrap()
    .with_http_client(http.clone());
    if request["legacy_cache"] == true {
        // The deprecated wrapper cache is created for this invocation only.
        resolver = resolver
            .with_shared_cache(
                Arc::new(ImageMemoryCache::new(4096, 512 << 20).unwrap()),
                "imgbundler/imageasset/v2/default-http",
                true,
            )
            .unwrap();
    }
    let source = request["source"].as_str().unwrap().as_bytes();
    let options = Options {
        local: request["local"] == true,
        remote: request["remote"] == true,
    };
    let budget = request["budget"].as_i64().unwrap();
    let out = match request["op"].as_str().unwrap() {
        "resolver-replacement" => {
            let mut budget = Budget::new(budget);
            let result = imgbundler::resolve_replacement(
                &resolver,
                request["href"].as_str().unwrap(),
                &mut budget,
                None,
            );
            match result {
                Ok(out) => {
                    json!({"replacement":String::from_utf8(out).unwrap(),"used":budget.used,"error":""})
                }
                Err(e) => json!({"replacement":"","used":budget.used,"error":e}),
            }
        }
        op => {
            let (out, error) = if op == "resolver-bundle" {
                imgbundler::bundle_with_resolver(source, options, Some(&resolver), None)
            } else {
                let (matches, hrefs) =
                    imgbundler::find_image_elements(source, options, &|| Ok(())).unwrap();
                imgbundler::run_resolver_workers(
                    source,
                    &matches,
                    &hrefs,
                    &resolver,
                    budget,
                    request["workers"].as_u64().unwrap() as usize,
                    None,
                )
            };
            let mut out =
                json!({"output":String::from_utf8(out).unwrap(),"error":error.unwrap_or_default()});
            let calls = http.calls.load(Ordering::Relaxed);
            if op == "resolver-workers" && request["workers"] == 16 {
                out["requests_within_worker_limit"] = json!(calls <= 16)
            } else {
                out["requests"] = json!(calls)
            }
            out
        }
    };
    out
}

fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let out = if let Value::Array(requests) = request {
        Value::Array(requests.into_iter().map(execute).collect())
    } else {
        execute(request)
    };
    println!("{out}");
}
