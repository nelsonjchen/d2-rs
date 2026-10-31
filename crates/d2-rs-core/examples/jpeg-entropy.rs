// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::jpeg_entropy::Reader;
use serde_json::{Value, json};
use std::io;
fn main() {
    let inputs: Vec<Value> = serde_json::from_reader(io::stdin().lock()).unwrap();
    let mut outputs = Vec::new();
    for input in inputs {
        let data: Vec<u8> = input["Data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let mut r = Reader::new(&data);
        let table_error = r
            .dht(
                input["Length"].as_i64().unwrap() as i32,
                input["Baseline"].as_bool().unwrap(),
            )
            .err()
            .map_or(String::new(), |e| e.to_string());
        let tables:Vec<Vec<Value>>=r.tables.iter().map(|row|row.iter().map(|h|json!({"Count":h.count,"Lut":h.lut.to_vec(),"Values":h.values.to_vec(),"Min":h.min,"Max":h.max,"Indices":h.indices})).collect()).collect();
        let mut steps = Vec::new();
        for a in input["Actions"].as_array().unwrap() {
            let n = a["N"].as_i64().unwrap_or(0);
            let mut bytes = Vec::new();
            let result = match a["Op"].as_str().unwrap() {
                "symbol" => r
                    .symbol(
                        a["Tc"].as_u64().unwrap_or(0) as usize,
                        a["Th"].as_u64().unwrap_or(0) as usize,
                    )
                    .map(i64::from),
                "receive" => r.receive(n as u8).map(i64::from),
                "bits" => r.read_bits(n as i32).map(i64::from),
                "bit" => r.bit().map(i64::from),
                "byte" => r.byte().map(i64::from),
                "stuffed" => r.stuffed().map(i64::from),
                "full" => {
                    bytes.resize(n as usize, 0);
                    r.full(&mut bytes).map(|_| 0)
                }
                "undo" => {
                    r.undo_stuffed();
                    Ok(0)
                }
                _ => panic!("unknown op"),
            };
            let (value, error) = match result {
                Ok(v) => (v, String::new()),
                Err(e) => (0, e.to_string()),
            };
            use std::fmt::Write;
            let mut encoded = String::new(); // Go JSON encodes a byte slice as base64.
            // Encode the tiny result slice directly with the RFC4648 alphabet.
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            for chunk in bytes.chunks(3) {
                let v = (u32::from(chunk[0]) << 16)
                    | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                    | u32::from(*chunk.get(2).unwrap_or(&0));
                for j in 0..4 {
                    if j > chunk.len() {
                        encoded.push('=')
                    } else {
                        encoded
                            .write_char(alphabet[((v >> (18 - j * 6)) & 63) as usize] as char)
                            .unwrap();
                    }
                }
            }
            steps.push(json!({"Value":value,"Data":encoded,"Error":error,"A":r.accumulator,"M":r.mask,"N":r.bits,"I":r.start,"J":r.end,"Unread":r.unread,"Offset":r.input_offset}));
        }
        outputs.push(json!({"TableError":table_error,"Tables":tables,"Steps":steps}));
    }
    serde_json::to_writer(io::stdout().lock(), &outputs).unwrap();
}
