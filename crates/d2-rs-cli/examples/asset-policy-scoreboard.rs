// SPDX-License-Identifier: MPL-2.0
//! Real socket/filesystem replay. No expected bytes or decisions enter requests.
// The replay drives the rooted local-file policy, which is Unix-only.
#[cfg(not(unix))]
fn main() {
    eprintln!("asset-policy-scoreboard needs a Unix host");
    std::process::exit(2);
}

#[cfg(unix)]
use base64::Engine as _;
#[cfg(unix)]
use d2_rs_scenebuild::{ImageLocalFiles, LocalFileError};
#[cfg(unix)]
use serde_json::{Value, json};
#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::path::{Component, Path, PathBuf};
#[cfg(unix)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
fn body(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value.as_str().unwrap())
        .unwrap()
}
#[cfg(unix)]
fn denied_error<T>(result: &Result<T, LocalFileError>) -> Value {
    match result {
        Ok(_) => json!({"error":"", "denied":false}),
        Err(e) => json!({"error":e.message, "denied":e.denied}),
    }
}
#[cfg(unix)]
fn close_file(file: std::fs::File) -> String {
    use std::os::fd::IntoRawFd;
    // SAFETY: one owned descriptor is consumed exactly once.
    if unsafe { libc::close(file.into_raw_fd()) } == 0 {
        String::new()
    } else {
        std::io::Error::last_os_error().to_string()
    }
}
#[cfg(unix)]
fn localfile(case: &Value, fixture_root: &Path) -> Value {
    let mut policies = HashMap::from([(0, ImageLocalFiles::default())]);
    let mut files = HashMap::new();
    let mut keys = Vec::<String>::new();
    let mut observations = Vec::new();
    let mut fixtures = Vec::new();
    let path_for = |op: &Value, field: &str| {
        let path = PathBuf::from(op[field].as_str().unwrap());
        assert!(path.is_absolute() && path.starts_with(fixture_root) && path != fixture_root);
        assert!(!path.components().any(|c| c == Component::ParentDir));
        path
    };
    let ops = case["operations"].as_array().unwrap();
    let mut index = 0;
    while index < ops.len() {
        let op = &ops[index];
        let id = op["id"].as_u64().unwrap_or(0);
        let got = match op["op"].as_str().unwrap() {
            "mkdir" => {
                use std::os::unix::fs::PermissionsExt;
                let path = path_for(op, "path");
                assert!(!path.exists() && !path.is_symlink());
                if fixtures.is_empty() {
                    fixtures.push(path.parent().unwrap().to_owned());
                }
                std::fs::create_dir_all(&path).unwrap();
                std::fs::set_permissions(
                    path,
                    std::fs::Permissions::from_mode(op["mode"].as_u64().unwrap() as u32),
                )
                .unwrap();
                json!({"error":""})
            }
            "write" => {
                use std::os::unix::fs::OpenOptionsExt;
                let path = path_for(op, "path");
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(op["mode"].as_u64().unwrap() as u32)
                    .open(path)
                    .unwrap()
                    .write_all(&body(&op["body"]))
                    .unwrap();
                json!({"error":""})
            }
            "symlink" => {
                std::os::unix::fs::symlink(op["target"].as_str().unwrap(), path_for(op, "path"))
                    .unwrap();
                json!({"error":""})
            }
            "rename" => {
                std::fs::rename(path_for(op, "old"), path_for(op, "path")).unwrap();
                json!({"error":""})
            }
            "fifo" => {
                use std::os::unix::ffi::OsStrExt;
                let path = path_for(op, "path");
                let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                // SAFETY: name is nul-terminated and live for mkfifo.
                assert_eq!(
                    unsafe {
                        libc::mkfifo(name.as_ptr(), op["mode"].as_u64().unwrap() as libc::mode_t)
                    },
                    0
                );
                json!({"error":""})
            }
            "rooted" => {
                let result = ImageLocalFiles::rooted(&path_for(op, "path"));
                let error = result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                if let Ok(policy) = result {
                    policies.insert(id, policy);
                }
                json!({"error":error})
            }
            "unrestricted" => {
                policies.insert(id, ImageLocalFiles::Unrestricted);
                json!({})
            }
            "open" => {
                let start = Instant::now();
                let result = policies[&id].open(Path::new(op["path"].as_str().unwrap()));
                assert!(start.elapsed() < Duration::from_secs(2), "Open blocked");
                let got = denied_error(&result);
                if let Ok(file) = result {
                    files.insert(op["file"].as_u64().unwrap(), file);
                }
                got
            }
            "read" => {
                let mut data = vec![0; op["size"].as_u64().unwrap() as usize];
                let n = files
                    .get_mut(&op["file"].as_u64().unwrap())
                    .unwrap()
                    .read(&mut data)
                    .unwrap();
                data.truncate(n);
                json!({"data":base64::engine::general_purpose::STANDARD.encode(data),"error":if n==0{"EOF"}else{""}})
            }
            "file-close" => {
                json!({"error":close_file(files.remove(&op["file"].as_u64().unwrap()).unwrap())})
            }
            "cache-key" => {
                let result =
                    policies[&id].policy_cache_key(Path::new(op["path"].as_str().unwrap()));
                let mut got = denied_error(&result);
                let key = result.unwrap_or_default();
                got["equal_prior"] =
                    json!(keys.iter().map(|prior| prior == &key).collect::<Vec<_>>());
                keys.push(key);
                got
            }
            "close" => {
                let end = ops[index..]
                    .iter()
                    .take_while(|next| next["op"] == "close" && next["id"] == id)
                    .count()
                    + index;
                // Reproduce the shared-copy close contention rather than merely
                // serializing the 32 goroutine calls in the source test.
                let results = std::thread::scope(|scope| {
                    let handles:Vec<_>=(index..end).map(|_| {
                        let copy=policies[&id].clone();
                        scope.spawn(move||json!({"error":copy.try_close().err().map(|e|e.message).unwrap_or_default()}))
                    }).collect();
                    handles
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .collect::<Vec<_>>()
                });
                observations.extend(results);
                index = end;
                continue;
            }
            other => panic!("unknown operation {other}"),
        };
        observations.push(got);
        index += 1;
    }
    drop(files);
    drop(policies);
    for path in fixtures {
        assert!(path.starts_with(fixture_root) && path != fixture_root);
        std::fs::remove_dir_all(path).unwrap();
    }
    json!({"observations":observations,"raw_cache_keys":keys})
}
#[cfg(unix)]
fn network(cases: &[Value]) -> Vec<Value> {
    let first = &cases[0]["request"];
    let source = first["source"].as_str().unwrap();
    let (_, hrefs) = d2_rs_cli::imgbundler::find_image_elements(
        source.as_bytes(),
        d2_rs_cli::imgbundler::Options {
            local: false,
            remote: true,
        },
        &|| Ok(()),
    )
    .unwrap();
    let url = reqwest::Url::parse(&hrefs[0]).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let listener = std::net::TcpListener::bind(("127.0.0.1", url.port().unwrap())).unwrap();
    listener.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (server_hits, server_stop) = (hits.clone(), stop.clone());
    let png = body(&first["body"]);
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 4096];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = stream.read(&mut buffer).unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&buffer[..n]);
                        assert!(request.len() < 65536);
                    }
                    assert!(request.starts_with(b"GET /private HTTP/1.1\r\n"));
                    server_hits.fetch_add(1, Ordering::SeqCst);
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",png.len()).unwrap();
                    stream.write_all(&png).unwrap();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let out=cases.iter().map(|case|{
        let r=&case["request"];assert_eq!(r["body"],first["body"]);
        let before=hits.load(Ordering::SeqCst);
        let (out,error)=d2_rs_cli::imgbundler::bundle_remote_with_policy(r["source"].as_str().unwrap().as_bytes(),r["cache"]==true,r["allow_private"]==true,None);
        json!({"output":String::from_utf8(out).unwrap(),"error":error.unwrap_or_default(),"hits":hits.load(Ordering::SeqCst)-before})
    }).collect();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    out
}
#[cfg(unix)]
fn main() {
    let request: Value =
        serde_json::from_slice(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let cases = request["cases"].as_array().unwrap();
    let out = match request["tier"].as_str().unwrap() {
        "network" => network(cases),
        "localfile" => cases
            .iter()
            .map(|c| localfile(c, Path::new(request["fixture_root"].as_str().unwrap())))
            .collect(),
        _ => panic!("unknown tier"),
    };
    println!("{}", json!(out));
}
