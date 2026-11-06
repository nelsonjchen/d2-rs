// SPDX-License-Identifier: MPL-2.0
//! The cancellable request/body boundary of Go imageasset.loadHTTP, with the
//! lib/netpolicy public-only network policy (D2 master 6a7b48378).
use d2_rs_core::CancellationToken;
use std::cell::Cell;
use std::error::Error as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Keep transport availability distinct from fatal policy/limit/context failures.
#[derive(Debug)]
pub(super) struct FetchError {
    pub unavailable: bool,
    pub message: String,
}
impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            unavailable: false,
            message,
        }
    }
}
impl FetchError {
    fn unavailable(message: String) -> Self {
        Self {
            unavailable: true,
            message,
        }
    }
}

// netpolicy.Policy{AllowPrivateNetworks} as threaded by d2cli.Run through
// netpolicy.WithPolicy: false (the zero, public-only policy) or true. Unit
// tests serve fixtures from loopback listeners and run CLI dispatch in
// parallel, so the process policy is fixed permissive under cfg(test);
// policy tests pass the policy to `fetch_policy` explicitly, and the
// CLI-visible policy is scored by scripts/cli-diff-scoreboard.py.
static ALLOW_PRIVATE_NETWORK: AtomicBool = AtomicBool::new(cfg!(test));

pub(super) fn set_allow_private_network(allow: bool) {
    if !cfg!(test) {
        ALLOW_PRIVATE_NETWORK.store(allow, Ordering::Release);
    }
}

pub(super) fn allow_private_network() -> bool {
    ALLOW_PRIVATE_NETWORK.load(Ordering::Acquire)
}

use d2_rs_scenebuild::netpolicy::{BlockedAddress, host_is_blocked, public_client_builder};
fn blocked_by_policy(error: &reqwest::Error) -> bool {
    let mut cause = error.source();
    while let Some(inner) = cause {
        if inner.downcast_ref::<BlockedAddress>().is_some() {
            return true;
        }
        cause = inner.source();
    }
    false
}

#[cfg(test)]
pub(super) fn fetch(
    url: &str,
    timeout: Duration,
    cancellation: Option<CancellationToken>,
) -> Result<(Vec<u8>, String, Option<String>), String> {
    fetch_policy(url, timeout, cancellation, false, allow_private_network())
        .map_err(|error| error.message)
}

pub(super) fn fetch_raster(
    url: &str,
    timeout: Duration,
    cancellation: Option<CancellationToken>,
) -> Result<(Vec<u8>, String, Option<String>), FetchError> {
    fetch_policy(url, timeout, cancellation, true, allow_private_network())
}

tokio::task_local! {
    /// The URL a request currently targets, updated on each redirect.
    static REDIRECT_URL: Arc<Mutex<String>>;
}

/// The process's I/O runtime. Like Go's shared imageasset client transport,
/// it outlives each request so idle keep-alive connections are reused.
/// Dropping a request's futures closes its transport; an OS name lookup may
/// still finish inside getaddrinfo on the blocking pool without delaying the
/// caller, whose context has already ended.
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: std::sync::OnceLock<Result<tokio::runtime::Runtime, String>> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// One client (connection pool) per network policy, as imageasset.New builds
/// one HTTP client per resolver.
fn shared_client(allow_private: bool) -> Result<reqwest::Client, String> {
    static PUBLIC: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    static PRIVATE: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    let cell = if allow_private { &PRIVATE } else { &PUBLIC };
    cell.get_or_init(|| {
        // The 30s dial ceiling of net.Dialer in netpolicy.NewHTTPClient and
        // http.DefaultTransport; each request carries its own `timeout`.
        if allow_private {
            reqwest::Client::builder()
        } else {
            public_client_builder()
        }
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let _ = REDIRECT_URL.try_with(|url| *url.lock().unwrap() = attempt.url().to_string());
            // Every redirect dials a new destination through the policy.
            if !allow_private && host_is_blocked(attempt.url()) {
                attempt.error(BlockedAddress)
            } else if attempt.previous().len() >= 10 {
                // Go counts the initial request in the ten-entry history.
                attempt.error("stopped after 10 redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())
    })
    .clone()
}

fn fetch_policy(
    url: &str,
    timeout: Duration,
    cancellation: Option<CancellationToken>,
    raster: bool,
    allow_private: bool,
) -> Result<(Vec<u8>, String, Option<String>), FetchError> {
    let runtime =
        runtime().map_err(|error| format!("could not fetch remote asset {url:?}: {error}"))?;
    let result = runtime.block_on(fetch_async(
        url,
        timeout,
        cancellation.clone(),
        raster,
        allow_private,
    ));
    // Caller cancellation wins even when a response/error became ready together.
    if raster {
        if let Some(cause) = cancellation.and_then(|token| token.error()) {
            return Err(cause.to_string().into());
        }
    }
    result
}

async fn fetch_async(
    url: &str,
    timeout: Duration,
    cancellation: Option<CancellationToken>,
    raster: bool,
    allow_private: bool,
) -> Result<(Vec<u8>, String, Option<String>), FetchError> {
    let reading_body = Cell::new(false);
    let current_url = Arc::new(Mutex::new(url.to_owned()));
    let request = REDIRECT_URL.scope(Arc::clone(&current_url), async {
        let client = shared_client(allow_private)
            .map_err(|error| format!("could not fetch remote asset {url:?}: {error}"))?;
        if !allow_private && reqwest::Url::parse(url).is_ok_and(|parsed| host_is_blocked(&parsed)) {
            // The validated dialer rejects the literal destination; net/http
            // reports a transport failure without a response.
            return Err(FetchError::unavailable("HTTP request failed".to_owned()));
        }
        let mut builder = client.get(url)
            .timeout(timeout)
            .header("User-Agent", if raster {"Mozilla/5.0 (compatible; D2 imageasset)"} else {"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"})
            .header("Accept", "image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8")
            .header("Accept-Encoding", "gzip, deflate, br");
        if !raster {
            builder = builder
                .header("Accept-Language", "en-US,en;q=0.9")
                .header("DNT", "1")
                .header("Upgrade-Insecure-Requests", "1")
                .header("Sec-Fetch-Dest", "image")
                .header("Sec-Fetch-Mode", "no-cors")
                .header("Sec-Fetch-Site", "cross-site");
        }
        let mut response = builder.send().await.map_err(|error| {
            if raster {
                let message = "HTTP request failed".to_owned();
                if blocked_by_policy(&error) {
                    FetchError::unavailable(message)
                } else if error.is_redirect() || error.is_builder() {
                    FetchError::from(message)
                } else {
                    FetchError::unavailable(message)
                }
            } else {
                FetchError::from(if error.is_timeout() {
                    format!(
                        "Get {:?}: context deadline exceeded",
                        current_url.lock().unwrap()
                    )
                } else if error.is_redirect() {
                    format!(
                        "Get {:?}: stopped after 10 redirects",
                        current_url.lock().unwrap()
                    )
                } else {
                    format!("could not fetch remote asset {url:?}: {error}")
                })
            }
        })?;
        reading_body.set(true);
        if response.status().as_u16() != 200 {
            let code = response.status().as_u16();
            if raster {
                return Err(FetchError::unavailable(format!(
                    "expected status 200 but got status code {code}"
                )));
            }
            let reason = response
                .extensions()
                .get::<hyper::ext::ReasonPhrase>()
                .map(|reason| reason.as_bytes())
                .unwrap_or_else(|| {
                    response
                        .status()
                        .canonical_reason()
                        .unwrap_or("")
                        .as_bytes()
                });
            let reason = String::from_utf8_lossy(reason);
            let status = if reason.is_empty() {
                code.to_string()
            } else {
                format!("{code} {reason}")
            };
            return Err(format!("expected status 200 but got {code} {status}").into());
        }
        if raster
            && response
                .content_length()
                .is_some_and(|length| length > super::MAX_ASSET_BYTES)
        {
            return Err(format!(
                "fetched bytes {} exceeds limit {}",
                response.content_length().unwrap_or_default(),
                super::MAX_ASSET_BYTES
            )
            .into());
        }
        let (encoding, content_type) = if raster {
            raster_headers(response.headers())?
        } else {
            let encoding = response
                .headers()
                .get("Content-Encoding")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let content_type = response
                .headers()
                .get("Content-Type")
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            (encoding, content_type)
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            let message = body_error(url, error);
            if raster {
                FetchError::unavailable(message)
            } else {
                FetchError::from(message)
            }
        })? {
            let total = bytes.len().saturating_add(chunk.len()) as u64;
            if total > super::MAX_ASSET_BYTES {
                return Err(if raster {
                    format!(
                        "fetched bytes {total} exceeds limit {}",
                        super::MAX_ASSET_BYTES
                    )
                } else {
                    "http: request body too large".to_owned()
                }
                .into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((bytes, encoding, content_type))
    });
    // The synchronous document/watch API owns the token. Polling only bridges
    // that token to this private I/O runtime; expiry keeps its original cause.
    // Selecting cancellation drops the whole request, including partial bodies.
    tokio::select! {
        result = request => result,
        cause = cancelled(cancellation) => {
            Err(FetchError::from(if reading_body.get() { cause } else { format!("Get {:?}: {cause}", current_url.lock().unwrap()) }))
        }
    }
}

// Source imageasset.selectContentType/parseContentEncodings. Multiple header
// fields must be validated before reading a body, not silently reduced to one.
fn raster_headers(
    headers: &reqwest::header::HeaderMap,
) -> Result<(String, Option<String>), FetchError> {
    let mut content_type = None;
    let mut total = 0usize;
    for value in headers.get_all("Content-Type") {
        total += value.as_bytes().len();
        if total > 1024 {
            return Err("Content-Type header bytes exceeds limit 1024"
                .to_owned()
                .into());
        }
        let value = value
            .to_str()
            .map_err(|_| "invalid Content-Type header".to_owned())?
            .trim();
        if value.is_empty() {
            continue;
        }
        if content_type.is_some() {
            return Err("multiple Content-Type header values".to_owned().into());
        }
        content_type = Some(value.to_owned());
    }
    let mut encodings = Vec::new();
    total = 0;
    for value in headers.get_all("Content-Encoding") {
        total += value.as_bytes().len();
        if total > 256 {
            return Err("Content-Encoding header bytes exceeds limit 256"
                .to_owned()
                .into());
        }
        let value = value
            .to_str()
            .map_err(|_| "invalid Content-Encoding header".to_owned())?;
        for token in value.split(',') {
            let token = token.trim().to_ascii_lowercase();
            if token.is_empty() {
                return Err("Content-Encoding contains an empty token".to_owned().into());
            }
            if !matches!(
                token.as_str(),
                "identity" | "gzip" | "x-gzip" | "deflate" | "br"
            ) {
                return Err(format!("unsupported content encoding {token:?}").into());
            }
            if encodings.len() == 4 {
                return Err("content encoding layer count 5 exceeds limit 4"
                    .to_owned()
                    .into());
            }
            encodings.push(token);
        }
    }
    Ok((encodings.join(","), content_type))
}

fn body_error(url: &str, error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "context deadline exceeded".to_owned();
    }
    // Hyper preserves the concrete I/O failure beneath its body wrapper.
    // Translate only recognized decoder errors to net/http's corresponding
    // conditions; other transport errors retain their actual diagnostic.
    let mut cause = error.source();
    while let Some(inner) = cause {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::UnexpectedEof {
                return "unexpected EOF".to_owned();
            }
            let source_message = match io.to_string().as_str() {
                "Invalid chunk size line: missing size digit"
                | "Invalid chunk size line: Invalid Size"
                | "Invalid chunk size linear white space" => Some("invalid byte in chunk length"),
                "invalid chunk size: overflow" => Some("http chunk length too large"),
                "Invalid chunk body CR" | "Invalid chunk body LF" => {
                    Some("malformed chunked encoding")
                }
                _ => None,
            };
            if let Some(message) = source_message {
                return message.to_owned();
            }
        }
        cause = inner.source();
    }
    format!("could not read remote asset {url:?}: {error}")
}

async fn cancelled(cancellation: Option<CancellationToken>) -> String {
    let Some(token) = cancellation else {
        return std::future::pending().await;
    };
    loop {
        if let Some(error) = token.error() {
            return error.to_string();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_scenebuild::netpolicy::{is_local_hostname, is_public_address};
    use std::io::{Read, Write};

    fn request(wire: Vec<u8>) -> Result<(Vec<u8>, String, Option<String>), String> {
        request_policy(wire, false).map_err(|error| error.message)
    }

    fn request_policy(
        wire: Vec<u8>,
        raster: bool,
    ) -> Result<(Vec<u8>, String, Option<String>), FetchError> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            if raster {
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(
                    request.contains("user-agent: mozilla/5.0 (compatible; d2 imageasset)\r\n")
                );
                assert!(!request.contains("sec-fetch-"));
            }
            // A size-limited client may close before all bytes are accepted.
            let _ = stream.write_all(&wire);
        });
        let result = fetch_policy(&url, Duration::from_secs(5), None, raster, true);
        server.join().unwrap();
        result
    }

    // netpolicy_test.go TestNonPublicAddressRanges.
    #[test]
    fn netpolicy_non_public_address_ranges() {
        for address in [
            "0.0.0.1",
            "10.0.0.1",
            "100.100.100.200",
            "127.0.0.1",
            "168.63.129.16",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::7f00:1",
            "2001::1",
            "2002:7f00:1::",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "ff02::1",
            "fec0::1",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_address(address.parse().unwrap()), "{address}");
        }
        for host in [
            "localhost",
            "a.LOCALHOST",
            "metadata.google.internal",
            "x.metadata.goog",
        ] {
            assert!(is_local_hostname(host), "{host}");
        }
        assert!(!is_local_hostname("example.com"));
    }

    #[test]
    fn public_only_policy_blocks_loopback_before_connecting() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        for url in [
            format!("http://127.0.0.1:{port}/a.svg"),
            format!("http://localhost:{port}/a.svg"),
            format!("http://[::1]:{port}/a.svg"),
        ] {
            let error = fetch_policy(&url, Duration::from_secs(5), None, true, false).unwrap_err();
            assert!(error.unavailable, "{url}");
            assert_eq!(error.message, "HTTP request failed");
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn raster_availability_keeps_policy_limits_and_context_fatal() {
        for wire in [
            b"HTTP/1.1 404 arbitrary reason\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc".to_vec(),
        ] {
            let error = request_policy(wire, true).unwrap_err();
            assert!(error.unavailable, "{error:?}");
            assert!(!error.message.contains("arbitrary reason"));
        }
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            super::super::MAX_ASSET_BYTES + 1
        );
        assert!(
            !request_policy(wire.into_bytes(), true)
                .unwrap_err()
                .unavailable
        );
        assert!(
            !fetch_raster("http://[", Duration::from_secs(1), None)
                .unwrap_err()
                .unavailable
        );
        let token = CancellationToken::new();
        token.cancel();
        let error = fetch_raster(
            "http://127.0.0.1:1/asset",
            Duration::from_secs(1),
            Some(token),
        )
        .unwrap_err();
        assert!(!error.unavailable);
        assert!(error.message.contains("context canceled"));
        // A refused transport has no HTTP response and is unavailable.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        drop(listener);
        assert!(
            fetch_raster(&url, Duration::from_secs(1), None)
                .unwrap_err()
                .unavailable
        );
    }

    #[test]
    fn raster_header_admission_is_fatal_before_body_availability() {
        for headers in [
            "Content-Type: image/png\r\nContent-Type: image/png".to_owned(),
            format!("Content-Type: {}", "x".repeat(1025)),
            "Content-Encoding: gzip,".to_owned(),
            "Content-Encoding: identity,identity,identity,identity,identity".to_owned(),
            "Content-Encoding: unsupported".to_owned(),
            format!("Content-Encoding: {}", "x".repeat(257)),
        ] {
            let wire = format!(
                "HTTP/1.1 200 OK\r\n{headers}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc"
            );
            assert!(
                !request_policy(wire.into_bytes(), true)
                    .unwrap_err()
                    .unavailable
            );
        }
    }

    #[test]
    fn response_body_failures_keep_go_decoder_errors() {
        for (headers, body, expected) in [
            ("Content-Length: 4", "abc", "unexpected EOF"),
            ("Transfer-Encoding: chunked", "4\r\nabc", "unexpected EOF"),
            (
                "Transfer-Encoding: chunked",
                "3\r\nabc\r\n",
                "unexpected EOF",
            ),
            (
                "Transfer-Encoding: chunked",
                "xyz\r\n",
                "invalid byte in chunk length",
            ),
            (
                "Transfer-Encoding: chunked",
                "1\r\nx??0\r\n\r\n",
                "malformed chunked encoding",
            ),
        ] {
            let wire = format!("HTTP/1.1 200 OK\r\n{headers}\r\nConnection: close\r\n\r\n{body}");
            assert_eq!(request(wire.into_bytes()).unwrap_err(), expected);
        }
    }

    #[test]
    fn remote_status_retains_custom_and_empty_reason_phrases() {
        for (status, expected) in [
            ("404 Image unavailable", "404 404 Image unavailable"),
            ("599 Custom failure", "599 599 Custom failure"),
            ("404", "404 404"),
        ] {
            let wire =
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            assert_eq!(
                request(wire.into_bytes()).unwrap_err(),
                format!("expected status 200 but got {expected}")
            );
        }
    }

    #[test]
    fn empty_remote_body_retains_server_content_type() {
        let (bytes, _, content_type) = request(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: image/svg+xml; charset=utf-8\r\nConnection: close\r\n\r\n".to_vec()).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(
            content_type.as_deref(),
            Some("image/svg+xml; charset=utf-8")
        );
    }
}
