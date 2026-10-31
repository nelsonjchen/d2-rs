// SPDX-License-Identifier: MPL-2.0
use d2_rs_language::parser::*;
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Request {
    hex: String,
    utf16: bool,
    fail: i64,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    depth: usize,
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
    let rows:Vec<_>=requests.into_iter().map(|request| {
        let bytes:Vec<u8>=request.hex.as_bytes().as_chunks::<2>().0.iter().map(|h|u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        let reader=Reader {inner:Utf8Reader::new(&bytes),calls:0,fail:request.fail};
        let mut parser=Parser::new("input.d2".into(),reader,request.utf16);
        parser.depth=request.depth;
        assert_eq!(parser.read(),Some(if request.mode == "block_comment" { '"' } else if request.mode == "block" { '|' } else if request.mode == "comment" { '#' } else { '\'' }));
        let mut scalar_hex=None;
        let node=if request.mode == "block_comment" {
 let (text,eof)=parser.peek_n(2);assert!(!eof);assert_eq!(text,"\"\"");parser.commit();
 let b=parser.parse_block_comment();scalar_hex=Some(b.value.as_bytes().iter().map(|v|format!("{v:02x}")).collect::<String>());json!(b)
 } else if request.mode == "block" {let b=parser.parse_block_string();scalar_hex=Some(b.value.as_bytes().iter().map(|v|format!("{v:02x}")).collect::<String>());json!(b)} else if request.mode == "comment" {json!(parser.parse_comment())} else {json!(parser.parse_single_quoted_string())};
        let mut tail=String::new();while let Some(r)=parser.read(){tail.push(r);}
        let mut row=json!({"node":node,"errors":parser.errors,"display":parser.errors.to_string(),"position":parser.position,"tail":tail});
        if let Some(hex)=scalar_hex {row[if request.mode == "block_comment" {"value_hex"} else {"scalar_hex"}]=json!(hex);row["depth"]=json!(parser.depth);}
        row
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
