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
    #[serde(default)]
    in_key: bool,
    #[serde(default)]
    edge: bool,
    #[serde(default)]
    spread: bool,
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
    let rows:Vec<_>=requests.into_iter().map(|r| {
        let bytes:Vec<u8>=r.hex.as_bytes().as_chunks::<2>().0.iter().map(|h|u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        let mut p=Parser::new("input.d2".into(),Reader{inner:Utf8Reader::new(&bytes),calls:0,fail:r.fail},r.utf16);
        p.in_edge_group=r.edge;
        let mut pattern_hex=None;
        let node=match r.mode.as_str() {
            "key"=>json!(p.parse_key()),"string"=>json!(p.parse_string(r.in_key)),
            "unquoted"=>{let u=p.parse_unquoted_string(r.in_key);pattern_hex=u.as_ref().and_then(|u|u.pattern.as_ref()).map(|parts|parts.iter().map(|part|part.read().as_bytes().iter().map(|v|format!("{v:02x}")).collect::<String>()).collect::<Vec<_>>());json!(u)},
            "double"=>{assert_eq!(p.read(),Some('"'));json!(p.parse_double_quoted_string(r.in_key))},
            "import"=>{for _ in 0..if r.spread {4}else{1}{p.read();}json!(p.parse_import(r.spread))},
            "sub"=>{for _ in 0..if r.spread {4}else{1}{p.read();}json!(p.parse_substitution(r.spread))},
            _=>panic!("unknown mode"),
        };
        let mut tail=String::new();while let Some(r)=p.read(){tail.push(r);}
        let mut row=json!({"node":node,"errors":p.errors,"display":p.errors.to_string(),"position":p.position,"tail":tail});if r.mode=="unquoted" {row["pattern_hex"]=json!(pattern_hex);}
        row
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
