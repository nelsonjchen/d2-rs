// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the pinned watch access API through its socket writer.
use super::*;
use serde_json::{Value, json};

#[test]
#[ignore = "executed explicitly by cli-watch-scoreboard.py"]
fn replay() {
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_WATCH_REQUEST").unwrap()).unwrap(),
    ).unwrap();
    let mut results = Vec::new();
    let mut previous_token = String::new();
    for input in inputs {
        if input["operation"] == "token" {
            let access = WatchAccess::new(12345).unwrap();
            results.push(json!({"error":"", "length":access.token.len(),
                "url_safe":!access.token.contains(['+', '/', '=']),
                "distinct":access.token != previous_token}));
            previous_token = access.token;
            continue;
        }
        let access = WatchAccess {
            token: input["token"].as_str().unwrap().to_owned(),
            cookie_name: input["cookie_name"].as_str().unwrap().to_owned(),
        };
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        let mut headers = BTreeMap::new();
        if let Some(cookie) = input["cookie"].as_str().filter(|s| !s.is_empty()) {
            headers.insert("cookie".to_owned(), cookie.to_owned());
        }
        let allowed = access.authorize(&mut stream, input["target"].as_str().unwrap(), &headers);
        drop(stream);
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let mut code = 200;
        let mut result_headers = serde_json::Map::new();
        let mut body = "";
        if !response.is_empty() {
            let (head, content) = response.split_once("\r\n\r\n").unwrap();
            body = content;
            let mut lines = head.split("\r\n");
            code = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
            for line in lines {
                let (name, value) = line.split_once(": ").unwrap();
                // httptest.ResponseRecorder observes the application headers;
                // socket framing is checked separately here, not normalized.
                if name == "Content-Length" {
                    assert_eq!(value.parse::<usize>().unwrap(), content.len());
                } else if name == "Connection" {
                    assert_eq!(value, "close");
                } else {
                    assert!(result_headers.insert(name.to_owned(), json!([value])).is_none());
                }
            }
        }
        results.push(json!({"allowed":allowed,"code":code,"headers":result_headers,"body":body}));
    }
    std::fs::write(std::env::var_os("D2RS_WATCH_RESPONSE").unwrap(), serde_json::to_vec(&results).unwrap()).unwrap();
}
