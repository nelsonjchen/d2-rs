// SPDX-License-Identifier: MPL-2.0
//! Original public-address and validated-dial contracts; inputs contain no decisions.
use d2_rs_scenebuild::netpolicy;
use serde_json::{Value, json};
use std::net::IpAddr;

#[path = "fixtures/netpolicy-before.rs"]
mod before;

fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let out: Vec<Value> = request["cases"].as_array().unwrap().iter().map(|case| {
        if request["baseline"] == true && case["name"] != "TestNonPublicAddressRanges" {
            return json!({"unsupported":"native validated dial API unavailable before L23"});
        }
        let out: Vec<Value> = case["operations"].as_array().unwrap().iter().map(|op| {
            if op["op"] == "public" {
                let address: IpAddr = op["address"].as_str().unwrap().parse().unwrap();
                return json!(if request["baseline"] == true { before::is_public_address(address) } else {netpolicy::is_public_address(address)});
            }
            assert_eq!(op["op"], "dial");
            let (host, port) = op["address"].as_str().unwrap().rsplit_once(':').unwrap();
            let mut lookups = Vec::new();
            let mut dials = Vec::new();
            let result: Result<(),String> = netpolicy::validated_dial(op["network"].as_str().unwrap(), host, port, None, |network, host| {
                lookups.push(json!({"network":network,"host":host}));
                if !op["lookup_error"].as_str().unwrap().is_empty() { return Err(op["lookup_error"].as_str().unwrap().into()); }
                Ok(op["answers"].as_array().unwrap().iter().map(|v|v.as_str().unwrap().parse().unwrap()).collect())
            }, |network,address| {
                dials.push(json!({"network":network,"address":address}));
                Err(op["dial_error"].as_str().unwrap().to_owned())
            });
            let error=result.unwrap_err();
            json!({"blocked": error == netpolicy::BlockedAddress.to_string(), "error":error,"lookups":lookups,"dials":dials})
        }).collect();
        json!(out)
    }).collect();
    println!("{}", json!(out));
}
