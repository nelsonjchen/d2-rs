// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use base64::Engine as _;
use d2_rs_core::CancellationToken;
use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

const WATCH_JS: &[u8] = include_bytes!("watch_static/watch.js");
const WATCH_CSS: &[u8] = include_bytes!("watch_static/watch.css");
const FAVICON: &[u8] = include_bytes!("watch_static/favicon.ico");
const ERROR_FAVICON: &[u8] = include_bytes!("watch_static/favicon-err.ico");
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const FILE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(32);
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;

type DependencySet = BTreeSet<PathBuf>;
type SharedDependencySet = Arc<Mutex<DependencySet>>;
type RemoteAssetCache = Arc<Mutex<BTreeMap<String, super::AssetData>>>;

thread_local! {
    static ACTIVE_DEPENDENCIES: RefCell<Option<SharedDependencySet>> = const { RefCell::new(None) };
    static ACTIVE_REMOTE_ASSET_CACHE: RefCell<Option<RemoteAssetCache>> = const { RefCell::new(None) };
    static ACTIVE_CANCELLATION: RefCell<Option<CancellationToken>> = const { RefCell::new(None) };
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub struct DependencyTrackingGuard(Option<SharedDependencySet>);

impl Drop for DependencyTrackingGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        ACTIVE_DEPENDENCIES.with(|active| {
            active.replace(previous);
        });
    }
}

pub fn track_dependencies(dependencies: SharedDependencySet) -> DependencyTrackingGuard {
    let previous = ACTIVE_DEPENDENCIES.with(|active| active.replace(Some(dependencies)));
    DependencyTrackingGuard(previous)
}

pub(crate) struct CancellationGuard(Option<CancellationToken>);

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        ACTIVE_CANCELLATION.with(|active| {
            active.replace(previous);
        });
    }
}

pub(crate) fn track_cancellation(cancellation: CancellationToken) -> CancellationGuard {
    let previous = ACTIVE_CANCELLATION.with(|active| active.replace(Some(cancellation)));
    CancellationGuard(previous)
}

pub fn active_cancellation_token() -> Option<CancellationToken> {
    ACTIVE_CANCELLATION.with(|active| active.borrow().clone())
}

#[derive(Clone)]
pub struct AssetWorkerContext {
    dependencies: Option<SharedDependencySet>,
    cache: Option<RemoteAssetCache>,
    cancellation: Option<CancellationToken>,
}

impl AssetWorkerContext {
    pub fn capture() -> Self {
        Self {
            dependencies: ACTIVE_DEPENDENCIES.with(|active| active.borrow().clone()),
            cache: ACTIVE_REMOTE_ASSET_CACHE.with(|active| active.borrow().clone()),
            cancellation: active_cancellation_token(),
        }
    }

    pub fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _dependencies = DependencyTrackingGuard(
            ACTIVE_DEPENDENCIES.with(|active| active.replace(self.dependencies.clone())),
        );
        let _cache = use_remote_asset_cache(self.cache.clone());
        let _cancellation = CancellationGuard(
            ACTIVE_CANCELLATION.with(|active| active.replace(self.cancellation.clone())),
        );
        operation()
    }
}

pub fn was_interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

pub fn record_dependency(path: impl AsRef<Path>) {
    let path = absolute_watch_path(path.as_ref());
    ACTIVE_DEPENDENCIES.with(|active| {
        if let Some(dependencies) = active.borrow().as_ref()
            && let Ok(mut dependencies) = dependencies.lock()
        {
            dependencies.insert(path);
        }
    });
}

// d2cli/watch.go trackedFS.Open: a failed open must neither block on a
// FIFO nor enter the successfully opened dependency set. Reuse localfile's
// descriptor validation rather than checking a path before opening it.
pub(super) fn open_tracked_file(
    policy: &d2_rs_scenebuild::ImageLocalFiles,
    path: &Path,
) -> Result<fs::File, d2_rs_scenebuild::LocalFileError> {
    let file = policy.open(path)?;
    record_dependency(path);
    Ok(file)
}

#[cfg(all(test, unix))]
#[path = "watch/tracked_fs_api_tests.rs"]
mod tracked_fs_api_tests;

struct RemoteAssetCacheGuard(Option<RemoteAssetCache>);

impl Drop for RemoteAssetCacheGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        ACTIVE_REMOTE_ASSET_CACHE.with(|active| {
            active.replace(previous);
        });
    }
}

fn use_remote_asset_cache(cache: Option<RemoteAssetCache>) -> RemoteAssetCacheGuard {
    let previous = ACTIVE_REMOTE_ASSET_CACHE.with(|active| active.replace(cache));
    RemoteAssetCacheGuard(previous)
}

#[cfg(test)]
pub fn cached_remote_asset(url: &str) -> Option<Vec<u8>> {
    cached_remote_asset_data(url).map(|asset| asset.bytes)
}

pub fn cached_remote_asset_data(url: &str) -> Option<super::AssetData> {
    ACTIVE_REMOTE_ASSET_CACHE.with(|active| {
        let cache = active.borrow().clone()?;
        cache.lock().ok()?.get(url).cloned()
    })
}

#[cfg(test)]
pub fn cache_remote_asset(url: &str, bytes: &[u8]) {
    cache_remote_asset_data(url, &bytes.to_vec().into());
}

pub fn cache_remote_asset_data(url: &str, asset: &super::AssetData) {
    ACTIVE_REMOTE_ASSET_CACHE.with(|active| {
        if let Some(cache) = active.borrow().as_ref()
            && let Ok(mut cache) = cache.lock()
        {
            cache.insert(url.to_owned(), asset.clone());
        }
    });
}

pub fn requested(args: &[String]) -> Result<bool, String> {
    let environment = std::env::var("D2_WATCH").ok();
    requested_with_env(args, environment.as_deref())
}

fn requested_with_env(args: &[String], environment: Option<&str>) -> Result<bool, String> {
    let mut requested = environment
        .map(|value| parse_bool(value, "D2_WATCH"))
        .transpose()?
        .unwrap_or(false);
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--watch" || argument == "-w" {
            requested = true;
        } else if let Some(value) = argument.strip_prefix("--watch=") {
            requested = parse_bool(value, "watch")?;
        }
        index += 1;
    }
    Ok(requested)
}

pub fn run<F>(args: &[String], mut render: F) -> Result<Vec<u8>, String>
where
    F: FnMut(&[String]) -> (Result<Vec<u8>, String>, DependencySet),
{
    let config = WatchConfig::parse(args)?;
    INTERRUPTED.store(false, Ordering::Release);
    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    ctrlc::set_handler(move || {
        INTERRUPTED.store(true, Ordering::Release);
        signal_cancellation.cancel();
    })
    .map_err(|error| format!("could not install interrupt handler: {error}"))?;
    let _cancellation_scope = track_cancellation(cancellation.clone());
    let listener = preview_listen_address(&config.host, config.port)
        .and_then(TcpListener::bind)
        .map_err(|error| {
            format!(
                "could not listen on {}:{}: {error}",
                config.host, config.port
            )
        })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure preview listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("could not inspect preview listener: {error}"))?;
    // d2cli/watch.go: every request needs the per-process access token, given
    // once as ?token= (exchanged for a port-scoped cookie) or as that cookie.
    let access = Arc::new(WatchAccess::new(address.port())?);
    let url = format!("http://{address}/?token={}", access.token);
    let shared = Arc::new(PreviewState::default());
    let server_state = Arc::clone(&shared);
    let server_cancellation = cancellation.clone();
    let title = config.output_title.clone();
    let server_access = Arc::clone(&access);
    let server_thread = thread::Builder::new()
        .name("d2-rs-preview-server".into())
        .spawn(move || serve(listener, server_state, title, server_access, server_cancellation))
        .map_err(|error| format!("could not start preview server: {error}"))?;

    eprintln!("listening on {url}");
    let mut active_target = String::new();
    let remote_cache = config
        .img_cache
        .then(|| Arc::new(Mutex::new(BTreeMap::new())));
    let mut dependencies =
        DependencySet::from([absolute_watch_path(Path::new(&config.input_path))]);
    compile_and_publish(
        &config,
        &active_target,
        &mut render,
        &shared,
        &mut dependencies,
        remote_cache.clone(),
        false,
    );
    if cancellation.is_cancelled() {
        let _ = server_thread.join();
        return Ok(Vec::new());
    }
    open_preview(&url, config.browser.as_deref());
    let mut last_snapshot = snapshot(&dependencies);

    while !cancellation.is_cancelled() {
        thread::sleep(FILE_POLL_INTERVAL);
        if cancellation.is_cancelled() {
            break;
        }
        let requested_target = shared
            .board_target
            .lock()
            .map(|target| target.clone())
            .unwrap_or_else(|_| active_target.clone());
        let mut current_snapshot = snapshot(&dependencies);
        let changed = current_snapshot != last_snapshot;
        if changed {
            thread::sleep(CHANGE_DEBOUNCE);
            current_snapshot = snapshot(&dependencies);
        }
        if requested_target != active_target || changed {
            if changed {
                // d2cli/watch.go:296–309 sorts the burst's changed paths and
                // identifies each dependency in the recompilation notice.
                let cwd = std::env::current_dir().ok();
                let changed_paths = current_snapshot
                    .iter()
                    .filter(|(path, state)| last_snapshot.get(*path) != Some(*state))
                    .map(|(path, _)| {
                        cwd.as_ref()
                            .and_then(|cwd| path.strip_prefix(cwd).ok())
                            .unwrap_or(path)
                            .display()
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!("detected change in {changed_paths}: recompiling...");
            } else {
                eprintln!("preview board changed; recompiling");
            }
            active_target = requested_target;
            compile_and_publish(
                &config,
                &active_target,
                &mut render,
                &shared,
                &mut dependencies,
                remote_cache.clone(),
                true,
            );
            last_snapshot = snapshot(&dependencies);
        }
    }
    cancellation.cancel();
    let _ = server_thread.join();
    Ok(Vec::new())
}

fn preview_listen_address(host: &str, port: u16) -> io::Result<SocketAddr> {
    // d2cli/watch.go:465–472 calls Go net.Listen, whose ListenConfig.Listen
    // selects addrs.first(isIPv4) (net/dial.go:889), falling back to the first
    // address only when no IPv4 address exists. Rust bind otherwise takes the
    // resolver's first successful address, which can make localhost IPv6-only.
    let addresses = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    addresses
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no listen addresses"))
}

fn compile_and_publish<F>(
    config: &WatchConfig,
    target: &str,
    render: &mut F,
    shared: &Arc<PreviewState>,
    dependencies: &mut DependencySet,
    remote_cache: Option<RemoteAssetCache>,
    recompile: bool,
) where
    F: FnMut(&[String]) -> (Result<Vec<u8>, String>, DependencySet),
{
    let mut arguments = config.render_args.clone();
    arguments.push(format!("--target={target}"));
    arguments.push("--watch-preview".into());
    let _remote_cache_scope = use_remote_asset_cache(remote_cache);
    let (result, opened) = render(&arguments);
    dependencies.clear();
    dependencies.insert(absolute_watch_path(Path::new(&config.input_path)));
    dependencies.extend(opened.into_iter().map(|path| absolute_watch_path(&path)));

    let (svg, error) = match result {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(svg) => (svg, String::new()),
            Err(error) => (
                String::new(),
                format!("preview renderer returned non-UTF-8 SVG: {error}"),
            ),
        },
        Err(error) => {
            let phase = if recompile { "recompile" } else { "compile" };
            (String::new(), format!("failed to {phase}: {error}"))
        }
    };
    if !error.is_empty() {
        eprintln!("{error}");
    }
    let message = json!({
        "svg": svg,
        "scale": config.scale,
        "err": error,
    })
    .to_string();
    publish(shared, message);
}

#[derive(Default)]
struct PreviewState {
    latest: Mutex<Option<String>>,
    clients: Mutex<Vec<mpsc::Sender<String>>>,
    board_target: Mutex<String>,
    connection_threads: Mutex<Vec<thread::JoinHandle<()>>>,
}

fn publish(shared: &Arc<PreviewState>, message: String) {
    if let Ok(mut latest) = shared.latest.lock() {
        *latest = Some(message.clone());
    }
    if let Ok(mut clients) = shared.clients.lock() {
        clients.retain(|client| client.send(message.clone()).is_ok());
        // d2cli/watch.go:619–637: this log is also the preview update signal.
        let count = clients.len();
        let suffix = if count == 1 { "" } else { "s" };
        eprintln!("broadcasting update to {count} client{suffix}");
    }
}

fn serve(
    listener: TcpListener,
    shared: Arc<PreviewState>,
    title: String,
    access: Arc<WatchAccess>,
    cancellation: CancellationToken,
) {
    while !cancellation.is_cancelled() {
        match listener.accept() {
            Ok((stream, _)) => {
                let client_shared = Arc::clone(&shared);
                let title = title.clone();
                let client_access = Arc::clone(&access);
                let client_cancellation = cancellation.clone();
                if let Ok(handle) = thread::Builder::new()
                    .name("d2-rs-preview-client".into())
                    .spawn(move || {
                        handle_connection(
                            stream,
                            client_shared,
                            &title,
                            &client_access,
                            client_cancellation,
                        )
                    })
                    && let Ok(mut threads) = shared.connection_threads.lock()
                {
                    threads.push(handle);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                eprintln!("preview server accept failed: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let threads = shared
        .connection_threads
        .lock()
        .map(|mut threads| std::mem::take(&mut *threads))
        .unwrap_or_default();
    for handle in threads {
        let _ = handle.join();
    }
}

fn handle_connection(
    mut stream: TcpStream,
    shared: Arc<PreviewState>,
    title: &str,
    access: &WatchAccess,
    cancellation: CancellationToken,
) {
    // macOS may inherit O_NONBLOCK from the accept loop's listener. Like
    // Go's net/http server (d2cli/watch.go:475–486), wait for request bytes
    // before parsing the HTTP/WebSocket handshake. WebSockets switch back
    // to nonblocking mode after their handshake for cancellation polling.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let request = match read_http_header(&mut stream) {
        Ok(request) => request,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&request);
    let mut lines = request.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let request_target = parts.next().unwrap_or("/");
    let path = request_target.split('?').next().unwrap_or("/");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();

    if !access.authorize(&mut stream, request_target, &headers) {
        return;
    }
    if method != "GET" {
        write_http_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed",
        );
        return;
    }
    if path == "/watch" {
        websocket_connection(stream, headers, shared, cancellation);
        return;
    }
    if path == "/static/watch.js" {
        write_http_response(
            &mut stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            WATCH_JS,
        );
        return;
    }
    if path == "/static/watch.css" {
        write_http_response(&mut stream, "200 OK", "text/css; charset=utf-8", WATCH_CSS);
        return;
    }
    if path == "/static/favicon.ico" {
        write_http_response(&mut stream, "200 OK", "image/x-icon", FAVICON);
        return;
    }
    if path == "/static/favicon-err.ico" {
        write_http_response(&mut stream, "200 OK", "image/x-icon", ERROR_FAVICON);
        return;
    }
    if path.starts_with("/static/") {
        write_http_response(&mut stream, "404 Not Found", "text/plain", b"not found");
        return;
    }

    if let Ok(mut current_target) = shared.board_target.lock() {
        *current_target = board_target_for_path(path);
    }
    let page = preview_page(title);
    write_http_response(
        &mut stream,
        "200 OK",
        "text/html; charset=utf-8",
        page.as_bytes(),
    );
}

fn websocket_connection(
    mut stream: TcpStream,
    headers: BTreeMap<String, String>,
    shared: Arc<PreviewState>,
    cancellation: CancellationToken,
) {
    if headers
        .get("upgrade")
        .is_none_or(|value| !value.eq_ignore_ascii_case("websocket"))
    {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"websocket upgrade required",
        );
        return;
    }
    let Some(key) = headers.get("sec-websocket-key") else {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"missing websocket key",
        );
        return;
    };
    let accept = websocket_accept(key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut clients) = shared.clients.lock() {
        clients.push(sender);
    }
    if let Ok(latest) = shared.latest.lock()
        && let Some(latest) = latest.as_ref()
        && write_websocket_frame(&mut stream, 0x1, latest.as_bytes()).is_err()
    {
        return;
    }
    let _ = stream.set_nonblocking(true);
    let mut incoming = [0; 1024];
    let mut last_ping = Instant::now();
    while !cancellation.is_cancelled() {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => {
                if write_websocket_frame(&mut stream, 0x1, message.as_bytes()).is_err() {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        match stream.read(&mut incoming) {
            Ok(0) => return,
            Ok(length) => {
                if length > 0 && incoming[0] & 0x0f == 0x08 {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
        if last_ping.elapsed() >= Duration::from_secs(30) {
            if write_websocket_frame(&mut stream, 0x9, &[]).is_err() {
                return;
            }
            last_ping = Instant::now();
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn read_http_header(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0; 1];
    while header.len() < MAX_HTTP_HEADER_BYTES {
        match stream.read(&mut byte)? {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "closed HTTP request",
                ));
            }
            _ => header.push(byte[0]),
        }
        if header.ends_with(b"\r\n\r\n") {
            return Ok(header);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "HTTP header exceeds size limit",
    ))
}

/// watch.go authorizeRequest and its access token.
#[cfg(test)]
mod security_api_tests;

struct WatchAccess {
    token: String,
    cookie_name: String,
}

impl WatchAccess {
    fn new(port: u16) -> Result<Self, String> {
        let mut bytes = [0_u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| "failed to generate watch access token".to_owned())?;
        Ok(Self {
            token: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
            cookie_name: format!("d2-watch-{port}"),
        })
    }

    fn matches(&self, provided: &str) -> bool {
        provided.len() == self.token.len()
            && ring::constant_time::verify_slices_are_equal(provided.as_bytes(), self.token.as_bytes())
                .is_ok()
    }

    fn authorize(
        &self,
        stream: &mut TcpStream,
        request_target: &str,
        headers: &BTreeMap<String, String>,
    ) -> bool {
        let (path, query) = request_target
            .split_once('?')
            .unwrap_or((request_target, ""));
        // URL.Query ignores malformed pairs, including unescaped semicolons.
        // Go strings carry arbitrary bytes; decoding through UTF-8 would alter
        // the token-free redirect's query values.
        let mut pairs = query
            .split('&')
            .filter(|pair| !pair.is_empty() && !pair.contains(';'))
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                Some((url_unescape(key, true)?, url_unescape(value, true)?))
            })
            .collect::<Vec<_>>();
        if let Some(position) = pairs.iter().position(|(key, _)| key == b"token") {
            if !self.matches_bytes(&pairs[position].1) {
                write_access_denied(stream);
                return false;
            }
            // Redirect to the same page without the token (url.Values.Encode
            // sorts by key), installing the cookie.
            pairs.retain(|(key, _)| key != b"token");
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            let query = pairs
                .iter()
                .map(|(key, value)| format!("{}={}", query_escape(key), query_escape(value)))
                .collect::<Vec<_>>()
                .join("&");
            let mut location = watch_redirect_path(path);
            if !query.is_empty() {
                location.push('?');
                location.push_str(&query);
            }
            let body = format!("<a href=\"{}\">See Other</a>.\n\n", html_escape_href(&location));
            let header = format!(
                "HTTP/1.1 303 See Other\r\nCache-Control: no-store\r\nContent-Type: text/html; charset=utf-8\r\nLocation: {location}\r\nReferrer-Policy: no-referrer\r\nSet-Cookie: {}={}; Path=/; HttpOnly; SameSite=Strict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                self.cookie_name,
                self.token,
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body.as_bytes());
            return false;
        }
        // Request.Cookie returns the first successfully parsed cookie of the
        // requested name, rather than accepting any later matching duplicate.
        let authorized = headers
            .get("cookie")
            .and_then(|cookies| first_cookie(cookies, &self.cookie_name))
            .is_some_and(|value| self.matches(value));
        if !authorized {
            write_access_denied(stream);
        }
        authorized
    }
}

impl WatchAccess {
    fn matches_bytes(&self, provided: &[u8]) -> bool {
        provided.len() == self.token.len()
            && ring::constant_time::verify_slices_are_equal(provided, self.token.as_bytes()).is_ok()
    }
}

// net/http readCookies + parseCookieValue (Go 1.27.0).
fn first_cookie<'a>(cookies: &'a str, wanted: &str) -> Option<&'a str> {
    cookies.split(';').find_map(|part| {
        let part = part.trim_matches([' ', '\t', '\r', '\n']);
        let (name, raw) = part.split_once('=').unwrap_or((part, ""));
        if name.trim_matches([' ', '\t', '\r', '\n']) != wanted {
            return None;
        }
        let value = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw);
        value
            .bytes()
            .all(|b| (0x20..0x7f).contains(&b) && !matches!(b, b'"' | b';' | b'\\'))
            .then_some(value)
    })
}

/// http.Error(w, "watch access denied", http.StatusForbidden).
fn write_access_denied(stream: &mut TcpStream) {
    write_http_response(
        stream,
        "403 Forbidden",
        "text/plain; charset=utf-8",
        b"watch access denied\n",
    );
}

fn url_unescape(value: &str, query: bool) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if query => out.push(b' '),
            b'%' => {
                let hi = char::from(*bytes.get(index + 1)?).to_digit(16)?;
                let lo = char::from(*bytes.get(index + 2)?).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    Some(out)
}

/// url.QueryEscape.
fn query_escape(value: &[u8]) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// authorizeRequest clears RawPath before URL.String. http.Redirect then
// path.Clean's the escaped path, preserving its trailing slash.
fn watch_redirect_path(path: &str) -> String {
    let decoded = url_unescape(path, false).unwrap_or_else(|| path.as_bytes().to_vec());
    let mut escaped = String::new();
    for byte in decoded {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'@')
        {
            escaped.push(byte as char);
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    let mut segments = Vec::new();
    for segment in escaped.split('/') {
        match segment {
            "" | "." => {}
            ".." => { segments.pop(); }
            _ => segments.push(segment),
        }
    }
    let mut result = format!("/{}", segments.join("/"));
    if escaped.ends_with('/') && result != "/" { result.push('/'); }
    result
}

fn html_escape_href(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#39;")
}

fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

fn write_websocket_frame(stream: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | (opcode & 0x0f));
    match payload.len() {
        length @ 0..=125 => header.push(length as u8),
        length @ 126..=65_535 => {
            header.push(126);
            header.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            header.push(127);
            header.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    stream.write_all(&header)?;
    stream.write_all(payload)
}

fn websocket_accept(key: &str) -> String {
    let challenge = format!("{key}{WEBSOCKET_GUID}");
    let hash = digest(&SHA1_FOR_LEGACY_USE_ONLY, challenge.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hash.as_ref())
}

fn preview_page(title: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n\t<meta charset=\"UTF-8\">\n\t<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\t<title>{}</title>\n\t<script src=\"/static/watch.js\"></script>\n\t<link rel=\"stylesheet\" href=\"/static/watch.css\">\n\t<link id=\"favicon\" rel=\"icon\" href=\"/static/favicon.ico\">\n</head>\n<body data-d2-dev-mode=false>\n\t<div id=\"d2-err\" style=\"display: none\"></div>\n\t<div id=\"d2-svg-container\"></div>\n</body>\n</html>",
        escape_html(title)
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn board_target_for_path(path: &str) -> String {
    let mut board_path = path.trim_start_matches('/').to_owned();
    if let Some((head, _)) = board_path.rsplit_once('.')
        && !head.is_empty()
    {
        board_path = head.to_owned();
    }
    board_path = board_path.trim_end_matches("/index").to_owned();
    if board_path == "index" {
        board_path.clear();
    }
    board_path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(percent_decode)
        .collect::<Vec<_>>()
        .join(".")
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn open_preview(url: &str, browser: Option<&str>) {
    if browser == Some("0") || std::env::var_os("D2_RS_NO_BROWSER").is_some() {
        return;
    }
    let mut command = if let Some(browser) = browser.filter(|browser| !browser.is_empty()) {
        Command::new(browser)
    } else {
        #[cfg(target_os = "macos")]
        let command = Command::new("open");
        #[cfg(target_os = "linux")]
        let command = Command::new("xdg-open");
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let command = return;
        command
    };
    let _ = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| eprintln!("could not open browser at {url}: {error}"));
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileSnapshot {
    exists: bool,
    length: u64,
    modified_nanos: Option<u128>,
}

fn snapshot(paths: &DependencySet) -> BTreeMap<PathBuf, FileSnapshot> {
    paths
        .iter()
        .map(|path| {
            let file_snapshot = fs::metadata(path)
                .map(|metadata| file_snapshot(&metadata))
                .unwrap_or(FileSnapshot {
                    exists: false,
                    length: 0,
                    modified_nanos: None,
                });
            (path.clone(), file_snapshot)
        })
        .collect()
}

fn file_snapshot(metadata: &fs::Metadata) -> FileSnapshot {
    FileSnapshot {
        exists: true,
        length: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos()),
    }
}

fn absolute_watch_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map(|current_dir| current_dir.join(path))
            .unwrap_or_else(|_| path.to_owned())
    }
}

#[derive(Debug)]
struct WatchConfig {
    input_path: String,
    output_title: String,
    host: String,
    port: u16,
    browser: Option<String>,
    scale: Option<f64>,
    img_cache: bool,
    render_args: Vec<String>,
}

impl WatchConfig {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut host = std::env::var("HOST").unwrap_or_else(|_| "localhost".into());
        let port_value = std::env::var("PORT").unwrap_or_else(|_| "0".into());
        let mut port = port_value
            .parse::<u16>()
            .map_err(|error| format!("invalid PORT value {port_value:?}: {error}"))?;
        let mut browser = std::env::var("BROWSER").ok();
        let mut scale = parse_scale_env()?;
        let mut img_cache = std::env::var("IMG_CACHE")
            .ok()
            .map(|value| parse_bool(&value, "IMG_CACHE"))
            .transpose()?
            .unwrap_or(true);
        let mut input_path = None;
        let mut output_path = None;
        let mut render_flags = Vec::new();
        let mut output_format = None;
        let mut target = None;
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            let (flag, inline_value) = argument
                .split_once('=')
                .map_or((argument.as_str(), None), |(flag, value)| {
                    (flag, Some(value))
                });
            match flag {
                "--watch" | "-w" => {}
                "--host" | "-h" => {
                    let value = option_value(args, &mut index, flag, inline_value)?;
                    host = value.to_owned();
                }
                "--port" | "-p" => {
                    let value = option_value(args, &mut index, flag, inline_value)?;
                    port = value
                        .parse::<u16>()
                        .map_err(|error| format!("invalid port value {value:?}: {error}"))?;
                }
                "--browser" => {
                    browser = Some(option_value(args, &mut index, flag, inline_value)?.to_owned());
                }
                "--img-cache" => {
                    if let Some(value) = inline_value {
                        img_cache = parse_bool(value, "img-cache")?;
                    } else {
                        img_cache = true;
                    }
                }
                "--stdout-format" => {
                    let value = option_value(args, &mut index, flag, inline_value)?;
                    output_format = Some(value.to_owned());
                    render_flags.push(format!("--stdout-format={value}"));
                }
                "--target" => {
                    let value = option_value(args, &mut index, flag, inline_value)?;
                    target = Some(value.to_owned());
                }
                "--scale" => {
                    let value = option_value(args, &mut index, flag, inline_value)?;
                    let parsed = value
                        .parse::<f64>()
                        .map_err(|error| format!("invalid scale value {value:?}: {error}"))?;
                    if !parsed.is_finite() || parsed <= 0.0 {
                        return Err("scale must be a finite positive number".into());
                    }
                    scale = Some(parsed);
                    render_flags.push(format!("--scale={value}"));
                }
                _ if is_value_flag(flag) => {
                    if inline_value.is_some() {
                        render_flags.push(argument.clone());
                    } else {
                        render_flags.push(argument.clone());
                        index += 1;
                        if index >= args.len() {
                            return Err(format!("{flag} requires a value"));
                        }
                        render_flags.push(args[index].clone());
                    }
                }
                _ if argument.starts_with('-') && argument != "-" => {
                    render_flags.push(argument.clone());
                }
                _ => {
                    if input_path.is_none() {
                        input_path = Some(argument.clone());
                    } else if output_path.is_none() {
                        output_path = Some(argument.clone());
                    } else {
                        return Err(super::command_line::usage("too many arguments passed"));
                    }
                    // Keep the normal output path; watch mode writes it while
                    // serving the corresponding preview from memory.
                }
            }
            index += 1;
        }
        if let Some(target) = target
            && target != "*"
        {
            return Err(super::command_line::usage("-w[atch] cannot be combined with --target"));
        }
        let input_path = input_path.ok_or_else(|| "expected an input .d2 file or -".to_owned())?;
        if input_path == "-" {
            return Err(super::command_line::usage(
                "-w[atch] cannot be combined with reading input from stdin",
            ));
        }
        let default_output_path = output_path.clone().unwrap_or_else(|| {
            let mut output_path = PathBuf::from(&input_path);
            let extension = output_format
                .as_deref()
                .map(|value| match value.to_ascii_lowercase().as_str() {
                    "ascii" | "txt" | "text" => "txt",
                    "png" => "png",
                    "gif" => "gif",
                    "pdf" => "pdf",
                    "pptx" | "ppt" => "pptx",
                    _ => "svg",
                })
                .unwrap_or("svg");
            output_path.set_extension(extension);
            output_path.display().to_string()
        });
        let output_title = Path::new(&default_output_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&default_output_path)
            .to_owned();
        let mut render_args = vec![input_path.clone()];
        render_args.extend(render_flags);
        if let Some(output_path) = output_path {
            render_args.push(output_path);
        }
        Ok(Self {
            input_path,
            output_title,
            host,
            port,
            browser,
            scale,
            img_cache,
            render_args,
        })
    }
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
    inline_value: Option<&'a str>,
) -> Result<&'a str, String> {
    if let Some(value) = inline_value {
        return Ok(value);
    }
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn is_value_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--layout"
            | "--ascii-mode"
            | "--pad"
            | "--salt"
            | "--theme"
            | "--theme-id"
            | "--dark-theme"
            | "--dark-theme-id"
            | "--animate-interval"
            | "--timeout"
            | "--tala-seeds"
            | "--tala-rs-seeds"
            | "--font"
            | "--font-regular"
            | "--font-italic"
            | "--font-bold"
            | "--font-semibold"
            | "--font-mono"
            | "--font-mono-bold"
            | "--font-mono-italic"
            | "--font-mono-semibold"
    )
}

fn parse_scale_env() -> Result<Option<f64>, String> {
    let Some(value) = std::env::var("SCALE").ok() else {
        return Ok(None);
    };
    let scale = value
        .parse::<f64>()
        .map_err(|error| format!("invalid SCALE value {value:?}: {error}"))?;
    if scale == -1.0 {
        return Ok(None);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err("SCALE must be -1 or a finite positive number".into());
    }
    Ok(Some(scale))
}

fn parse_bool(value: &str, name: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid {name} value {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn websocket_accept_matches_rfc_6455_example() {
        assert_eq!(
            websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn watch_board_paths_match_upstream_board_path_shape() {
        assert_eq!(board_target_for_path("/"), "");
        assert_eq!(board_target_for_path("/index"), "");
        assert_eq!(board_target_for_path("/layers/a.svg"), "layers.a");
        assert_eq!(
            board_target_for_path("/layers/a/steps/one"),
            "layers.a.steps.one"
        );
        assert_eq!(board_target_for_path("/layers/a%20b.svg"), "layers.a b");
    }

    #[test]
    fn preview_listener_prefers_ipv4_and_preserves_explicit_ipv6() {
        let localhost = preview_listen_address("localhost", 0).unwrap();
        assert_eq!(localhost, "127.0.0.1:0".parse::<SocketAddr>().unwrap());
        let listener = TcpListener::bind(localhost).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        assert!(client.peer_addr().unwrap().is_ipv4());
        assert_eq!(
            preview_listen_address("::1", 1234).unwrap(),
            "[::1]:1234".parse::<SocketAddr>().unwrap()
        );
    }

    fn watch_request(access: WatchAccess, request: &str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let server = thread::spawn(move || {
            handle_connection(
                stream,
                Arc::new(PreviewState::default()),
                "test.d2",
                &access,
                CancellationToken::new(),
            );
        });
        client.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn watch_requests_need_the_access_token() {
        let access = WatchAccess::new(80).unwrap();
        let token = access.token.clone();
        assert_eq!(token.len(), 43);
        let redirect = watch_request(
            access,
            &format!("GET /layers/x?b=2&token={token}&a=1 HTTP/1.1\r\n\r\n"),
        );
        assert!(redirect.starts_with("HTTP/1.1 303 See Other\r\n"), "{redirect}");
        assert!(redirect.contains("\r\nLocation: /layers/x?a=1&b=2\r\n"));
        assert!(redirect.contains(&format!(
            "\r\nSet-Cookie: d2-watch-80={token}; Path=/; HttpOnly; SameSite=Strict\r\n"
        )));
        let denied = watch_request(
            WatchAccess::new(80).unwrap(),
            "GET /?token=wrong HTTP/1.1\r\n\r\n",
        );
        assert!(denied.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        assert!(denied.ends_with("watch access denied\n"));
        let anonymous = watch_request(WatchAccess::new(80).unwrap(), "GET / HTTP/1.1\r\n\r\n");
        assert!(anonymous.starts_with("HTTP/1.1 403 Forbidden\r\n"));
    }

    #[test]
    fn preview_waits_for_headers_on_an_accepted_nonblocking_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (stream, _) = listener.accept().unwrap();
        // Force the flag macOS can inherit from the nonblocking listener.
        // No request bytes exist yet: an immediate read must not close it.
        stream.set_nonblocking(true).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let access = WatchAccess::new(1234).unwrap();
        let request = format!(
            "GET /static/favicon.ico HTTP/1.1\r\nHost: localhost\r\nCookie: d2-watch-1234={}\r\n\r\n",
            access.token
        );
        let server = thread::spawn(move || {
            started_tx.send(()).unwrap();
            handle_connection(
                stream,
                Arc::new(PreviewState::default()),
                "test.d2",
                &access,
                CancellationToken::new(),
            );
            done_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        let early = done_rx.recv_timeout(Duration::from_millis(50));
        let write_result = client.write_all(request.as_bytes());
        let mut response = Vec::new();
        let read_result = client.read_to_end(&mut response);
        server.join().unwrap();
        assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
        write_result.unwrap();
        read_result.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with(FAVICON));
    }

    #[test]
    fn preview_page_escapes_the_input_title() {
        let page = preview_page("<diagram & friends>.d2");
        assert!(page.contains("&lt;diagram &amp; friends&gt;.d2"));
        assert!(!page.contains("<diagram & friends>"));
    }

    #[test]
    fn websocket_frames_use_the_required_lengths() {
        fn frame(payload: &[u8]) -> Vec<u8> {
            let mut output = Vec::new();
            write_websocket_frame(&mut output, 0x1, payload).unwrap();
            output
        }
        assert_eq!(frame(b"a"), [0x81, 1, b'a']);
        assert_eq!(frame(&[7; 126])[..4], [0x81, 126, 0, 126]);
        let large = frame(&[9; 65_536]);
        assert_eq!(&large[..10], &[0x81, 127, 0, 0, 0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn tracking_records_only_successful_watcher_scope_paths() {
        let dependencies = Arc::new(Mutex::new(DependencySet::new()));
        {
            let _guard = track_dependencies(Arc::clone(&dependencies));
            record_dependency("watch-fixture-import.d2");
        }
        assert_eq!(dependencies.lock().unwrap().len(), 1);
        record_dependency("outside-watch-scope.d2");
        assert_eq!(dependencies.lock().unwrap().len(), 1);
    }

    #[test]
    fn remote_assets_are_cached_only_inside_enabled_watch_compiles() {
        let cache = Arc::new(Mutex::new(BTreeMap::new()));
        assert_eq!(
            cached_remote_asset("https://example.invalid/icon.svg"),
            None
        );
        {
            let _scope = use_remote_asset_cache(Some(Arc::clone(&cache)));
            assert_eq!(
                cached_remote_asset("https://example.invalid/icon.svg"),
                None
            );
            cache_remote_asset("https://example.invalid/icon.svg", b"<svg/>");
            assert_eq!(
                cached_remote_asset("https://example.invalid/icon.svg").as_deref(),
                Some(&b"<svg/>"[..])
            );
        }
        assert_eq!(
            cached_remote_asset("https://example.invalid/icon.svg"),
            None
        );
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn raster_unavailable_images_retry_then_cache_only_valid_success() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let icon = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(15);
            let mut count = 0;
            while count < 2 && std::time::Instant::now() < until {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                // On macOS accepted sockets can inherit the nonblocking
                // listener flag. Requests may not have arrived at accept time.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                if count == 0 {
                    stream.write_all(b"HTTP/1.1 404 Missing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let png = d2_rs_testdata::oracle_fixture_bytes!("tests/fixtures/unavailable-image-v090.png");
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",png.len()).unwrap();
                    stream.write_all(png).unwrap();
                }
                count += 1;
            }
            count
        });
        let _scope = use_remote_asset_cache(Some(Arc::new(Mutex::new(BTreeMap::new()))));
        let input = format!("a: \"\" {{shape: image; icon: {icon}; width: 64; height: 64}}\n");
        let render = || {
            crate::render_source_format(
                input.as_bytes(),
                &["--engine=dagre".into()],
                d2_rs_export::ExportFormat::Png,
            )
            .unwrap()
        };
        let first = render();
        let second = render();
        let third = render();
        assert!(first != second, "unavailable result must be retried");
        assert!(second == third, "successful result must be cached");
        assert_eq!(server.join().unwrap(), 2);
    }

    #[test]
    fn cancelled_asset_context_does_not_open_a_connection() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/cancelled.svg", listener.local_addr().unwrap());
        let token = CancellationToken::new();
        token.cancel();
        let _scope = track_cancellation(token);
        assert_eq!(
            crate::fetch_remote_asset(&url).unwrap_err(),
            format!("Get {url:?}: context canceled")
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        // Go's worker checks the image cache before invoking httpGet.
        let _cache = use_remote_asset_cache(Some(Arc::new(Mutex::new(BTreeMap::new()))));
        cache_remote_asset(&url, b"cached image");
        assert_eq!(crate::fetch_remote_asset(&url).unwrap(), b"cached image");
    }

    #[test]
    fn remote_cache_retains_empty_bodies_and_content_types() {
        let _cache = use_remote_asset_cache(Some(Arc::new(Mutex::new(BTreeMap::new()))));
        let asset = super::super::AssetData {
            bytes: Vec::new(),
            content_type: Some("image/svg+xml; charset=utf-8".into()),
        };
        cache_remote_asset_data("https://example.invalid/empty", &asset);
        let fetched = cached_remote_asset_data("https://example.invalid/empty").unwrap();
        assert_eq!(fetched.bytes, asset.bytes);
        assert_eq!(fetched.content_type, asset.content_type);
    }

    #[test]
    fn asset_worker_context_shares_cache_dependencies_and_restores_thread_state() {
        let dependencies = Arc::new(Mutex::new(BTreeSet::new()));
        let cache = Arc::new(Mutex::new(BTreeMap::new()));
        let _tracking = track_dependencies(Arc::clone(&dependencies));
        let _cache = use_remote_asset_cache(Some(Arc::clone(&cache)));
        let cancellation = CancellationToken::new();
        let _cancellation = track_cancellation(cancellation.clone());
        let context = AssetWorkerContext::capture();
        std::thread::spawn(move || {
            context.run(|| {
                record_dependency("asset-worker-context.svg");
                cache_remote_asset("https://example.invalid/worker.svg", b"worker");
                assert!(active_cancellation_token().is_some());
            });
            assert!(active_cancellation_token().is_none());
            assert!(cached_remote_asset("https://example.invalid/worker.svg").is_none());
        })
        .join()
        .unwrap();
        assert!(
            dependencies
                .lock()
                .unwrap()
                .contains(&absolute_watch_path(Path::new("asset-worker-context.svg")))
        );
        assert_eq!(
            cached_remote_asset("https://example.invalid/worker.svg"),
            Some(b"worker".to_vec())
        );
    }

    #[test]
    fn snapshots_notice_creation_and_content_metadata_changes() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "d2-rs-watch-snapshot-{}-{}.d2",
            std::process::id(),
            timestamp
        ));
        let paths = DependencySet::from([path.clone()]);
        let before = snapshot(&paths);
        fs::write(&path, b"a").unwrap();
        let created = snapshot(&paths);
        fs::write(&path, b"changed").unwrap();
        let changed = snapshot(&paths);
        let _ = fs::remove_file(&path);
        assert_ne!(before, created);
        assert_ne!(created, changed);
    }

    #[test]
    fn requested_watch_flag_obeys_explicit_values() {
        assert!(requested_with_env(&[], Some("1")).unwrap());
        assert!(requested_with_env(&["--watch".into()], None).unwrap());
        assert!(requested_with_env(&["-w".into()], None).unwrap());
        assert!(!requested_with_env(&["--watch=false".into()], Some("1")).unwrap());
        assert!(requested_with_env(&[], Some("bad")).is_err());
    }

    #[test]
    fn watch_arguments_preserve_render_options_format_and_file_output() {
        let config = WatchConfig::parse(&[
            "--watch".into(),
            "--host=127.0.0.1".into(),
            "--port=43000".into(),
            "--browser=0".into(),
            "--scale=0.5".into(),
            "--stdout-format=png".into(),
            "input.d2".into(),
            "output.png".into(),
        ])
        .unwrap();
        assert_eq!(config.input_path, "input.d2");
        assert_eq!(config.output_title, "output.png");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 43000);
        assert_eq!(config.browser.as_deref(), Some("0"));
        assert_eq!(config.scale, Some(0.5));
        assert_eq!(
            config.render_args,
            [
                "input.d2",
                "--scale=0.5",
                "--stdout-format=png",
                "output.png"
            ]
        );

        let default_output = WatchConfig::parse(&["--watch".into(), "input.d2".into()]).unwrap();
        assert_eq!(default_output.output_title, "input.svg");
        let uncached = WatchConfig::parse(&[
            "--watch".into(),
            "--img-cache=false".into(),
            "input.d2".into(),
        ])
        .unwrap();
        assert!(!uncached.img_cache);
    }

    #[test]
    fn watch_rejects_stdin_and_target_overrides() {
        assert!(
            WatchConfig::parse(&["--watch".into(), "-".into()])
                .unwrap_err()
                .contains("stdin")
        );
        assert!(
            WatchConfig::parse(&[
                "--watch".into(),
                "--target=layers.one".into(),
                "input.d2".into(),
            ])
            .unwrap_err()
            .contains("--target")
        );
        assert!(
            WatchConfig::parse(&[
                "--watch".into(),
                "input.d2".into(),
                "output.svg".into(),
                "extra.d2".into(),
            ])
            .unwrap_err()
            .contains("too many arguments passed")
        );
    }
}
