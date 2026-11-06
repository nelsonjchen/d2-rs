// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::parser::*;
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Request {
    hex: String,
    mode: String,
    utf16: bool,
    fail: i64,
}
struct Reader<'a> {
    inner: Utf8Reader<'a>,
    calls: i64,
    fail: i64,
}
impl RuneReader for Reader<'_> {
    fn read_rune(&mut self) -> Result<Option<char>, String> {
        if self.calls == self.fail {
            return Err("oracle read failure".into());
        }
        self.calls += 1;
        self.inner.read_rune()
    }
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows:Vec<_>=requests.into_iter().map(|r|{
        let bytes:Vec<u8>=r.hex.as_bytes().as_chunks::<2>().0.iter().map(|h|u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        let mut p=Parser::new("input.d2".into(),Reader{inner:Utf8Reader::new(&bytes),calls:0,fail:r.fail},r.utf16);
        let node=match r.mode.as_str(){
            "file"=>json!(p.parse_map(true)),"map"=>{p.read();json!(p.parse_map(false))},
            "array"=>{p.read();json!(p.parse_array())},"value"=>json!(p.parse_value()),
            "mapkey"=>json!(p.parse_map_key()),"index"=>{p.read();json!(p.parse_edge_index())},
            _=>panic!("unknown mode"),
        };
        let mut tail=String::new();while let Some(r)=p.read(){tail.push(r);}
        json!({"node":node,"errors":p.errors,"display":p.errors.to_string(),"position":p.position,"tail":tail,"depth":p.depth,"edge_group":p.in_edge_group})
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
