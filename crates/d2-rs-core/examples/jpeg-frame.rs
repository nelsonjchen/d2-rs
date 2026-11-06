// SPDX-License-Identifier: MPL-2.0
use serde_json::{Value, json};
use std::io;
fn main() {
    let inputs: Vec<Value> = serde_json::from_reader(io::stdin().lock()).unwrap();
    let outputs:Vec<Value>=inputs.into_iter().map(|input|{let data:Vec<u8>=input["Data"].as_array().unwrap().iter().map(|v|v.as_u64().unwrap() as u8).collect();match d2_rs_core::jpeg_decode::decode(&data){Ok(image)=>json!({"Width":image.width,"Height":image.height,"Error":"","Pixels":image.rgba}),Err(error)=>json!({"Width":0,"Height":0,"Error":error,"Pixels":[]})}}).collect();
    serde_json::to_writer(io::stdout().lock(), &outputs).unwrap();
}
