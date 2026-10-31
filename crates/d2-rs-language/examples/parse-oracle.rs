// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::ast::Error;
use d2_rs_language::{ParseOptions, parse, parser::ParseError};
use serde::Deserialize;
use serde_json::json;
#[derive(Deserialize)]
struct Request {
    hex: String,
    mode: String,
    utf16: bool,
    options: String,
    path: String,
}
fn main() {
    let requests: Vec<Request> = serde_json::from_reader(std::io::stdin()).unwrap();
    let rows:Vec<_>=requests.into_iter().map(|r|{
        let bytes:Vec<u8>=r.hex.as_bytes().as_chunks::<2>().0.iter().map(|h|u8::from_str_radix(std::str::from_utf8(h).unwrap(),16).unwrap()).collect();
        if r.mode != "public_parse" {
            let value = match r.mode.as_str() {
                "public_key" => d2_rs_language::parse_key(&bytes).map(|n|json!(n)),
                "public_mapkey" => d2_rs_language::parse_map_key(&bytes).map(|n|json!(n)),
                "public_value" => d2_rs_language::parse_value(&bytes).map(|n|json!(d2_rs_core::ast::ValueBox::from_child(Some(n)))),
                _=>panic!("unknown mode"),
            };
            return match value {
                Ok(node)=>json!({"node":node,"error":"","cause":null}),
                Err(error)=>json!({"node":null,"error":error.to_string(),"cause":error.cause}),
            };
        }
        let mut initial=match r.options.as_str(){
            "empty"=>Some(ParseError{errors:Some(Vec::new())}),
            "prior"=>Some(ParseError{errors:Some(vec![Error{message:"existing diagnostic".into(),..Default::default()}])}),
            _=>None,
        };
        let options=if r.options=="none"{None}else{Some(ParseOptions{utf16_positions:r.utf16,errors:initial.as_mut()})};
        let output=parse(r.path,&bytes,options);
        let display=output.error.as_ref().map_or_else(String::new,ToString::to_string);
        json!({"map":output.map,"error":display,"structured":output.error,"accumulator":initial})
    }).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
