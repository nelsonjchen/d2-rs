// SPDX-License-Identifier: MPL-2.0
//! JSON adapter for the owning CLI imgbundler algorithms. Resource data URIs
//! are explicit fixture inputs; this tier makes no resolver/policy claim.
use d2_rs_cli::imgbundler::{self, Budget, ImageMatch, Options};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let source = request["source"].as_str().unwrap_or("").as_bytes();
    let context = || {
        if request["canceled"] == true {
            Err("context canceled".into())
        } else {
            Ok(())
        }
    };
    let options = Options {
        local: request["local"] == true,
        remote: request["remote"] == true,
    };
    let out = match request["op"].as_str().unwrap() {
        "scan" => match imgbundler::find_image_elements(source, options, &context) {
            Ok((matches, hrefs)) => {
                json!({"matches":matches.iter().map(|m| [m.start,m.end,m.href_start,m.href_end]).collect::<Vec<_>>(),"hrefs":hrefs,"error":""})
            }
            Err(error) => json!({"matches":[],"hrefs":[],"error":error}),
        },
        "apply" => {
            let matches = request["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| ImageMatch {
                    start: m[0].as_u64().unwrap() as usize,
                    end: m[1].as_u64().unwrap() as usize,
                    href_start: m[2].as_u64().unwrap() as usize,
                    href_end: m[3].as_u64().unwrap() as usize,
                })
                .collect::<Vec<_>>();
            let replacements: BTreeMap<String, Vec<u8>> = request["replacements"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().as_bytes().to_vec()))
                .collect();
            let result = imgbundler::apply_replacements(
                source,
                &matches,
                &replacements,
                request["limit"].as_i64().unwrap(),
                &context,
            );
            match result {
                Ok(out) => json!({"output":String::from_utf8(out).unwrap(),"error":""}),
                Err(error) => json!({"output":String::from_utf8_lossy(source),"error":error}),
            }
        }
        "reserve" => {
            let mut budget = Budget {
                used: request["used"].as_i64().unwrap(),
                limit: request["limit"].as_i64().unwrap(),
                exhausted: request["sticky"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            };
            let error = budget
                .reserve(request["count"].as_i64().unwrap())
                .err()
                .unwrap_or_default();
            json!({"used":budget.used,"error":error})
        }
        "bundle" => {
            // Fixtures provide the already-resolved SVG resource. Native
            // scanner, cumulative budget and replacement run without Go.
            let result = (|| -> Result<Vec<u8>, String> {
                let (matches, hrefs) = imgbundler::find_image_elements(source, options, &context)?;
                let mut replacements = BTreeMap::new();
                let mut budget = Budget::new(imgbundler::MAX_BUNDLED_BYTES);
                for href in hrefs {
                    let uri = request["resources"][&href]
                        .as_str()
                        .ok_or("fixture resource missing")?;
                    replacements.insert(href, imgbundler::replacement(uri, &mut budget)?);
                }
                imgbundler::apply_replacements(
                    source,
                    &matches,
                    &replacements,
                    imgbundler::MAX_OUTPUT_BYTES,
                    &context,
                )
            })();
            match result {
                Ok(out) => json!({"output":String::from_utf8(out).unwrap(),"error":""}),
                Err(error) => {
                    json!({"output":String::from_utf8_lossy(source),"error":format!("{}failed to bundle images: {error}",request["prefix"].as_str().unwrap())})
                }
            }
        }
        _ => json!({"unsupported":true}),
    };
    println!("{out}");
}
