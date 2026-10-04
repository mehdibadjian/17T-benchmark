use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::actor::{ActorError, ActorMessage, ActorRef, ActorResponse, ActorSystem};

/// Helper to decode percent-encoded URL strings.
pub fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                result.push(val);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            result.push(b' ');
            i += 1;
            continue;
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

/// Helper to parse query parameter strings (e.g. "key=val&start=a&limit=10").
pub fn parse_query_string(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let val = parts.next().unwrap_or("");
        map.insert(url_decode(key), url_decode(val));
    }
    map
}

/// Properly escapes characters for JSON strings.
pub fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Unescapes simple JSON escape sequences.
pub fn unescape_json(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    '"' => res.push('"'),
                    '\\' => res.push('\\'),
                    '/' => res.push('/'),
                    'b' => res.push('\x08'),
                    'f' => res.push('\x0c'),
                    'n' => res.push('\n'),
                    'r' => res.push('\r'),
                    't' => res.push('\t'),
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                res.push(ch);
                            }
                        }
                    }
                    other => {
                        res.push('\\');
                        res.push(other);
                    }
                }
            }
        } else {
            res.push(c);
        }
    }
    res
}

/// Extracts a string or raw field value from a JSON object string.
fn extract_json_field(json: &str, field: &str) -> Option<String> {
    let pattern = format!("\"{}\"", field);
    let idx = json.find(&pattern)?;
    let after = &json[idx + pattern.len()..];
    let colon_idx = after.find(':')?;
    let rest = after[colon_idx + 1..].trim_start();
    if rest.starts_with('"') {
        let mut end = 1;
        let bytes = rest.as_bytes();
        while end < bytes.len() {
            if bytes[end] == b'"' && bytes[end - 1] != b'\\' {
                break;
            }
            end += 1;
        }
        if end < bytes.len() {
            return Some(unescape_json(&rest[1..end]));
        }
    } else {
        let end = rest
            .find(|c: char| c == ',' || c == '}' || c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let val = rest[..end].trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// Parses POST body for key and value, supporting JSON, form-urlencoded, or query param override.
pub fn parse_put_body(
    body_bytes: &[u8],
    query_params: &HashMap<String, String>,
) -> Option<(String, String)> {
    let body_str = std::str::from_utf8(body_bytes).unwrap_or("").trim();

    // 1. If key is present in query parameters:
    if let Some(key) = query_params.get("key") {
        let val = if !body_str.is_empty() {
            body_str.to_string()
        } else if let Some(v) = query_params.get("value") {
            v.clone()
        } else {
            String::new()
        };
        return Some((key.clone(), val));
    }

    // 2. Try parsing JSON body:
    if body_str.starts_with('{') && body_str.ends_with('}') {
        let key = extract_json_field(body_str, "key");
        let val = extract_json_field(body_str, "value");
        if let (Some(k), Some(v)) = (key, val) {
            return Some((k, v));
        }
    }

    // 3. Try parsing application/x-www-form-urlencoded:
    if body_str.contains('=') {
        let form_map = parse_query_string(body_str);
        if let (Some(k), Some(v)) = (form_map.get("key"), form_map.get("value")) {
            return Some((k.clone(), v.clone()));
        }
    }

    None
}

/// Returns true if the first request line is an HTTP/1.x request.
pub fn is_http_request_line(line: &str) -> bool {
    let trimmed = line.trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() >= 3 && parts[2].starts_with("HTTP/") {
        return true;
    }
    if parts.len() >= 2 && parts[1].starts_with('/') {
        let m = parts[0].to_uppercase();
        if m == "GET" || m == "POST" || m == "DELETE" || m == "PUT" || m == "HEAD" {
            return true;
        }
    }
    false
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Worker thread pool to handle concurrent TCP connections.
pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Option<Sender<Job>>,
}

struct Worker {
    _id: usize,
    thread: Option<JoinHandle<()>>,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        assert!(size > 0);
        let (sender, receiver) = channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            let rx = Arc::clone(&receiver);
            let thread = thread::Builder::new()
                .name(format!("worker-{}", id))
                .spawn(move || loop {
                    let job = {
                        let lock = match rx.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        match lock.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        }
                    };
                    job();
                })
                .expect("Failed to spawn worker thread");

            workers.push(Worker {
                _id: id,
                thread: Some(thread),
            });
        }

        Self {
            workers,
            sender: Some(sender),
        }
    }

    pub fn execute<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(ref sender) = self.sender {
            sender.send(Box::new(f)).map_err(|e| e.to_string())
        } else {
            Err("ThreadPool is shut down".to_string())
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        drop(self.sender.take());
        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// HTTP response model.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: &'static str,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn ok_json(body: &str) -> Self {
        Self {
            status_code: 200,
            status_text: "OK",
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn bad_request(body: &str) -> Self {
        Self {
            status_code: 400,
            status_text: "Bad Request",
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn not_found(body: &str) -> Self {
        Self {
            status_code: 404,
            status_text: "Not Found",
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn internal_error(body: &str) -> Self {
        Self {
            status_code: 500,
            status_text: "Internal Server Error",
            content_type: "application/json",
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn to_bytes(&self, keep_alive: bool) -> Vec<u8> {
        let conn_str = if keep_alive { "keep-alive" } else { "close" };
        let header = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\nServer: turing-server/0.1.0\r\n\r\n",
            self.status_code,
            self.status_text,
            self.content_type,
            self.body.len(),
            conn_str
        );
        let mut bytes = Vec::with_capacity(header.len() + self.body.len());
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: String,
    pub worker_threads: usize,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:8080".to_string(),
            worker_threads: 4,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
        }
    }
}

/// Running server handle.
pub struct ServerHandle {
    local_addr: SocketAddr,
    running: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn join(mut self) {
        self.shutdown();
        if let Some(thread) = self.listener_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(thread) = self.listener_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Multi-threaded TCP and HTTP REST server with line-based protocol auto-detection.
pub struct HttpServer {
    config: ServerConfig,
    _system: ActorSystem,
    storage_actor: ActorRef,
    metrics_actor: ActorRef,
}

impl HttpServer {
    pub fn new(addr: impl Into<String>, system: ActorSystem) -> Result<Self, ActorError> {
        let storage_actor = system
            .get_actor("storage")
            .ok_or_else(|| ActorError::ActorNotFound("storage".to_string()))?;
        let metrics_actor = system
            .get_actor("metrics")
            .ok_or_else(|| ActorError::ActorNotFound("metrics".to_string()))?;

        Ok(Self {
            config: ServerConfig {
                addr: addr.into(),
                ..Default::default()
            },
            _system: system,
            storage_actor,
            metrics_actor,
        })
    }

    pub fn with_threads(
        addr: impl Into<String>,
        system: ActorSystem,
        worker_threads: usize,
    ) -> Result<Self, ActorError> {
        let storage_actor = system
            .get_actor("storage")
            .ok_or_else(|| ActorError::ActorNotFound("storage".to_string()))?;
        let metrics_actor = system
            .get_actor("metrics")
            .ok_or_else(|| ActorError::ActorNotFound("metrics".to_string()))?;

        Ok(Self {
            config: ServerConfig {
                addr: addr.into(),
                worker_threads,
                ..Default::default()
            },
            _system: system,
            storage_actor,
            metrics_actor,
        })
    }

    pub fn with_actors(
        config: ServerConfig,
        system: ActorSystem,
        storage_actor: ActorRef,
        metrics_actor: ActorRef,
    ) -> Self {
        Self {
            config,
            _system: system,
            storage_actor,
            metrics_actor,
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Spawns the server onto background threads, returning a control handle.
    pub fn bind(&self) -> std::io::Result<ServerHandle> {
        let listener = TcpListener::bind(&self.config.addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;
        let running = Arc::new(AtomicBool::new(true));

        let pool = Arc::new(ThreadPool::new(self.config.worker_threads));
        let running_clone = Arc::clone(&running);
        let storage_actor = self.storage_actor.clone();
        let metrics_actor = self.metrics_actor.clone();
        let read_timeout = self.config.read_timeout;
        let write_timeout = self.config.write_timeout;

        let listener_thread = thread::Builder::new()
            .name("server-listener".to_string())
            .spawn(move || {
                while running_clone.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _addr)) => {
                            let storage = storage_actor.clone();
                            let metrics = metrics_actor.clone();
                            let _ = pool.execute(move || {
                                let _ = handle_connection(
                                    stream,
                                    storage,
                                    metrics,
                                    read_timeout,
                                    write_timeout,
                                );
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => {
                            if !running_clone.load(Ordering::SeqCst) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            })?;

        Ok(ServerHandle {
            local_addr,
            running,
            listener_thread: Some(listener_thread),
        })
    }

    /// Runs the server synchronously until stopped.
    pub fn start(&self) -> std::io::Result<()> {
        let handle = self.bind()?;
        handle.join();
        Ok(())
    }
}

fn handle_connection(
    mut stream: TcpStream,
    storage: ActorRef,
    metrics: ActorRef,
    read_timeout: Duration,
    write_timeout: Duration,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(write_timeout));

    let _ = metrics.send(ActorMessage::RecordConnectionOpened);

    let res = process_stream(&mut stream, &storage, &metrics);

    let _ = metrics.send(ActorMessage::RecordConnectionClosed);
    res
}

fn process_stream(
    stream: &mut TcpStream,
    storage: &ActorRef,
    metrics: &ActorRef,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first_line = String::new();

    match reader.read_line(&mut first_line) {
        Ok(0) => return Ok(()),
        Err(e) => return Err(e),
        Ok(_) => {}
    }

    if is_http_request_line(&first_line) {
        handle_http_connection(&mut reader, stream, first_line, storage, metrics)
    } else {
        handle_line_connection(&mut reader, stream, first_line, storage, metrics)
    }
}

fn handle_line_connection(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    initial_line: String,
    storage: &ActorRef,
    metrics: &ActorRef,
) -> std::io::Result<()> {
    let trimmed = initial_line.trim_end_matches(|c| c == '\r' || c == '\n');
    if !trimmed.is_empty() {
        if trimmed.eq_ignore_ascii_case("QUIT") || trimmed.eq_ignore_ascii_case("EXIT") {
            let _ = writer.write_all(b"BYE\r\n");
            let _ = writer.flush();
            return Ok(());
        }
        let reply = dispatch_line_command(trimmed, storage, metrics);
        writer.write_all(reply.as_bytes())?;
        writer.flush()?;
    }

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.eq_ignore_ascii_case("QUIT") || trimmed.eq_ignore_ascii_case("EXIT") {
                    let _ = writer.write_all(b"BYE\r\n");
                    let _ = writer.flush();
                    break;
                }
                let reply = dispatch_line_command(trimmed, storage, metrics);
                writer.write_all(reply.as_bytes())?;
                writer.flush()?;
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                break;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

fn dispatch_line_command(line: &str, storage: &ActorRef, metrics: &ActorRef) -> String {
    let start_time = Instant::now();
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(3, |c: char| c.is_ascii_whitespace());
    let cmd = parts.next().unwrap_or("").to_uppercase();

    let (response, op_name) = match cmd.as_str() {
        "PING" => ("PONG\r\n".to_string(), "PING"),
        "PUT" => {
            let key = parts.next();
            let val = parts.next();
            match (key, val) {
                (Some(k), Some(v)) => {
                    let msg = ActorMessage::put_str(k, v);
                    match storage.ask(msg, Duration::from_secs(5)) {
                        Ok(ActorResponse::Ok) => ("OK\r\n".to_string(), "PUT"),
                        Ok(ActorResponse::Error(err)) => (format!("ERR {}\r\n", err), "PUT"),
                        Ok(_) => ("OK\r\n".to_string(), "PUT"),
                        Err(e) => (format!("ERR {}\r\n", e), "PUT"),
                    }
                }
                _ => ("ERR syntax: PUT <key> <val>\r\n".to_string(), "PUT_ERR"),
            }
        }
        "GET" => {
            let key = parts.next();
            match key {
                Some(k) if !k.is_empty() => {
                    let msg = ActorMessage::get(k);
                    match storage.ask(msg, Duration::from_secs(5)) {
                        Ok(ActorResponse::Value(Some(val))) => {
                            let val_str = String::from_utf8_lossy(&val);
                            (format!("VALUE {}\r\n", val_str), "GET")
                        }
                        Ok(ActorResponse::Value(None)) => ("NOT_FOUND\r\n".to_string(), "GET"),
                        Ok(ActorResponse::Error(err)) => (format!("ERR {}\r\n", err), "GET"),
                        Ok(_) => ("NOT_FOUND\r\n".to_string(), "GET"),
                        Err(e) => (format!("ERR {}\r\n", e), "GET"),
                    }
                }
                _ => ("ERR syntax: GET <key>\r\n".to_string(), "GET_ERR"),
            }
        }
        "DEL" => {
            let key = parts.next();
            match key {
                Some(k) if !k.is_empty() => {
                    let msg = ActorMessage::delete(k);
                    match storage.ask(msg, Duration::from_secs(5)) {
                        Ok(ActorResponse::Deleted(_)) | Ok(ActorResponse::Ok) => {
                            ("OK\r\n".to_string(), "DEL")
                        }
                        Ok(ActorResponse::Error(err)) => (format!("ERR {}\r\n", err), "DEL"),
                        Ok(_) => ("OK\r\n".to_string(), "DEL"),
                        Err(e) => (format!("ERR {}\r\n", e), "DEL"),
                    }
                }
                _ => ("ERR syntax: DEL <key>\r\n".to_string(), "DEL_ERR"),
            }
        }
        "SCAN" => {
            let start_arg = parts.next();
            let limit_arg = parts.next();
            let start = match start_arg {
                Some(s) if !s.is_empty() && s != "\"\"" && s != "*" => Some(s.to_string()),
                _ => None,
            };
            let limit = limit_arg
                .and_then(|l| l.parse::<usize>().ok())
                .unwrap_or(100);

            let msg = ActorMessage::scan(start, limit);
            match storage.ask(msg, Duration::from_secs(5)) {
                Ok(ActorResponse::ScanResult(entries)) => {
                    let mut out = format!("ENTRIES {}\r\n", entries.len());
                    for (k, v) in entries {
                        out.push_str(&format!("{} {}\r\n", k, String::from_utf8_lossy(&v)));
                    }
                    out.push_str("END\r\n");
                    (out, "SCAN")
                }
                Ok(ActorResponse::Error(err)) => (format!("ERR {}\r\n", err), "SCAN"),
                Ok(_) => ("ENTRIES 0\r\nEND\r\n".to_string(), "SCAN"),
                Err(e) => (format!("ERR {}\r\n", e), "SCAN"),
            }
        }
        "STATS" => match metrics.ask(ActorMessage::GetMetrics, Duration::from_secs(5)) {
            Ok(ActorResponse::Metrics(snap)) => (snap.to_text(), "STATS"),
            _ => ("ERR failed to retrieve stats\r\n".to_string(), "STATS"),
        },
        _ => ("ERR unknown command\r\n".to_string(), "UNKNOWN"),
    };

    let latency_us = start_time.elapsed().as_micros() as u64;
    let _ = metrics.send(ActorMessage::RecordOp {
        op_type: op_name.to_string(),
        latency_us,
    });

    response
}

fn handle_http_connection(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    mut req_line: String,
    storage: &ActorRef,
    metrics: &ActorRef,
) -> std::io::Result<()> {
    loop {
        let (keep_alive, response_bytes) =
            parse_and_handle_http_request(reader, &req_line, storage, metrics)?;
        writer.write_all(&response_bytes)?;
        writer.flush()?;

        if !keep_alive {
            break;
        }

        req_line.clear();
        match reader.read_line(&mut req_line) {
            Ok(0) => break,
            Ok(_) => {
                if req_line.trim().is_empty() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}

fn parse_and_handle_http_request(
    reader: &mut BufReader<TcpStream>,
    request_line: &str,
    storage: &ActorRef,
    metrics: &ActorRef,
) -> std::io::Result<(bool, Vec<u8>)> {
    let start_time = Instant::now();
    let trimmed_req = request_line.trim_end_matches(|c| c == '\r' || c == '\n');
    let parts: Vec<&str> = trimmed_req.split_whitespace().collect();

    if parts.len() < 2 {
        let resp = HttpResponse::bad_request(r#"{"error":"Malformed HTTP request line"}"#);
        return Ok((false, resp.to_bytes(false)));
    }

    let method = parts[0].to_uppercase();
    let full_uri = parts[1];
    let is_http_11 = parts.len() >= 3 && parts[2].contains("1.1");

    let mut headers = HashMap::new();
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        let h_trim = header_line.trim_end_matches(|c| c == '\r' || c == '\n');
        if h_trim.is_empty() {
            break;
        }
        if let Some(colon) = h_trim.find(':') {
            let key = h_trim[..colon].trim().to_lowercase();
            let val = h_trim[colon + 1..].trim().to_string();
            headers.insert(key, val);
        }
    }

    let connection_hdr = headers.get("connection").map(|s| s.to_lowercase());
    let keep_alive = if let Some(ref conn) = connection_hdr {
        conn == "keep-alive"
    } else {
        is_http_11
    };

    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let (path, query_str) = match full_uri.find('?') {
        Some(idx) => (&full_uri[..idx], &full_uri[idx + 1..]),
        None => (full_uri, ""),
    };
    let query_params = parse_query_string(query_str);

    let response = route_http_request(&method, path, &query_params, &body, storage, metrics);

    let latency_us = start_time.elapsed().as_micros() as u64;
    let _ = metrics.send(ActorMessage::RecordOp {
        op_type: format!("HTTP_{}", method),
        latency_us,
    });

    Ok((keep_alive, response.to_bytes(keep_alive)))
}

fn route_http_request(
    method: &str,
    path: &str,
    query: &HashMap<String, String>,
    body: &[u8],
    storage: &ActorRef,
    metrics: &ActorRef,
) -> HttpResponse {
    match (method, path) {
        ("GET", "/health") => HttpResponse::ok_json(r#"{"status":"ok"}"#),
        ("GET", "/api/v1/get") => {
            let key = match query.get("key") {
                Some(k) if !k.is_empty() => k,
                _ => {
                    return HttpResponse::bad_request(
                        r#"{"error":"Missing 'key' query parameter"}"#,
                    )
                }
            };

            match storage.ask(ActorMessage::get(key), Duration::from_secs(5)) {
                Ok(ActorResponse::Value(Some(val))) => {
                    let val_str = String::from_utf8_lossy(&val);
                    let escaped_val = escape_json(&val_str);
                    let escaped_key = escape_json(key);
                    HttpResponse::ok_json(&format!(
                        r#"{{"status":"ok","key":"{}","value":"{}"}}"#,
                        escaped_key, escaped_val
                    ))
                }
                Ok(ActorResponse::Value(None)) => {
                    HttpResponse::not_found(r#"{"error":"Key not found"}"#)
                }
                Ok(ActorResponse::Error(err)) => {
                    HttpResponse::internal_error(&format!(r#"{{"error":"{}"}}"#, escape_json(&err)))
                }
                _ => HttpResponse::not_found(r#"{"error":"Key not found"}"#),
            }
        }
        ("POST", "/api/v1/put") => {
            let (key, val) = match parse_put_body(body, query) {
                Some((k, v)) => (k, v),
                None => {
                    return HttpResponse::bad_request(
                        r#"{"error":"Missing key or value in body/query"}"#,
                    )
                }
            };

            let msg = ActorMessage::put_str(&key, &val);
            match storage.ask(msg, Duration::from_secs(5)) {
                Ok(ActorResponse::Ok) => {
                    let escaped_key = escape_json(&key);
                    let escaped_val = escape_json(&val);
                    HttpResponse::ok_json(&format!(
                        r#"{{"status":"ok","key":"{}","value":"{}"}}"#,
                        escaped_key, escaped_val
                    ))
                }
                Ok(ActorResponse::Error(err)) => {
                    HttpResponse::internal_error(&format!(r#"{{"error":"{}"}}"#, escape_json(&err)))
                }
                _ => HttpResponse::internal_error(r#"{"error":"Failed to store entry"}"#),
            }
        }
        ("DELETE", "/api/v1/delete") => {
            let key = match query.get("key") {
                Some(k) if !k.is_empty() => k,
                _ => {
                    return HttpResponse::bad_request(
                        r#"{"error":"Missing 'key' query parameter"}"#,
                    )
                }
            };

            match storage.ask(ActorMessage::delete(key), Duration::from_secs(5)) {
                Ok(ActorResponse::Deleted(deleted)) => {
                    let escaped_key = escape_json(key);
                    if deleted {
                        HttpResponse::ok_json(&format!(
                            r#"{{"status":"ok","deleted":true,"key":"{}"}}"#,
                            escaped_key
                        ))
                    } else {
                        HttpResponse::not_found(&format!(
                            r#"{{"status":"not_found","deleted":false,"key":"{}"}}"#,
                            escaped_key
                        ))
                    }
                }
                Ok(ActorResponse::Ok) => {
                    let escaped_key = escape_json(key);
                    HttpResponse::ok_json(&format!(
                        r#"{{"status":"ok","deleted":true,"key":"{}"}}"#,
                        escaped_key
                    ))
                }
                Ok(ActorResponse::Error(err)) => {
                    HttpResponse::internal_error(&format!(r#"{{"error":"{}"}}"#, escape_json(&err)))
                }
                _ => HttpResponse::not_found(r#"{"error":"Key not found"}"#),
            }
        }
        ("GET", "/api/v1/scan") => {
            let start = query.get("start").cloned();
            let limit = query
                .get("limit")
                .and_then(|l| l.parse::<usize>().ok())
                .unwrap_or(100);

            match storage.ask(ActorMessage::scan(start, limit), Duration::from_secs(5)) {
                Ok(ActorResponse::ScanResult(entries)) => {
                    let mut entries_json = String::from("[");
                    let mut first = true;
                    for (k, v) in &entries {
                        if !first {
                            entries_json.push(',');
                        }
                        first = false;
                        let val_str = String::from_utf8_lossy(v);
                        entries_json.push_str(&format!(
                            r#"{{"key":"{}","value":"{}"}}"#,
                            escape_json(k),
                            escape_json(&val_str)
                        ));
                    }
                    entries_json.push(']');

                    HttpResponse::ok_json(&format!(
                        r#"{{"status":"ok","count":{},"entries":{}}}"#,
                        entries.len(),
                        entries_json
                    ))
                }
                Ok(ActorResponse::Error(err)) => {
                    HttpResponse::internal_error(&format!(r#"{{"error":"{}"}}"#, escape_json(&err)))
                }
                _ => HttpResponse::ok_json(r#"{"status":"ok","count":0,"entries":[]}"#),
            }
        }
        ("GET", "/api/v1/stats") => {
            match metrics.ask(ActorMessage::GetMetrics, Duration::from_secs(5)) {
                Ok(ActorResponse::Metrics(snap)) => HttpResponse::ok_json(&snap.to_json()),
                _ => HttpResponse::internal_error(r#"{"error":"Failed to retrieve stats"}"#),
            }
        }
        _ => HttpResponse::not_found(r#"{"error":"Endpoint not found"}"#),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{MetricsActor, StorageActor};

    #[test]
    fn test_url_decoding() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("foo+bar%21"), "foo bar!");
        assert_eq!(url_decode("simple"), "simple");
    }

    #[test]
    fn test_parse_query() {
        let q = parse_query_string("key=hello%20world&limit=10&flag");
        assert_eq!(q.get("key"), Some(&"hello world".to_string()));
        assert_eq!(q.get("limit"), Some(&"10".to_string()));
        assert_eq!(q.get("flag"), Some(&"".to_string()));
    }

    #[test]
    fn test_json_escaping() {
        assert_eq!(escape_json("hello \"world\"\n"), "hello \\\"world\\\"\\n");
    }

    #[test]
    fn test_parse_put_body_json() {
        let body = br#"{"key": "test_key", "value": "test_val"}"#;
        let query = HashMap::new();
        let parsed = parse_put_body(body, &query);
        assert_eq!(
            parsed,
            Some(("test_key".to_string(), "test_val".to_string()))
        );
    }

    #[test]
    fn test_parse_put_body_urlencoded() {
        let body = b"key=url_key&value=url_val";
        let query = HashMap::new();
        let parsed = parse_put_body(body, &query);
        assert_eq!(parsed, Some(("url_key".to_string(), "url_val".to_string())));
    }

    #[test]
    fn test_parse_put_body_query_override() {
        let body = b"raw_body_content";
        let mut query = HashMap::new();
        query.insert("key".to_string(), "my_key".to_string());
        let parsed = parse_put_body(body, &query);
        assert_eq!(
            parsed,
            Some(("my_key".to_string(), "raw_body_content".to_string()))
        );
    }

    #[test]
    fn test_is_http() {
        assert!(is_http_request_line("GET /health HTTP/1.1\r\n"));
        assert!(is_http_request_line("POST /api/v1/put HTTP/1.0"));
        assert!(is_http_request_line("GET /api/v1/get?key=foo"));
        assert!(!is_http_request_line("PING\r\n"));
        assert!(!is_http_request_line("PUT foo bar\r\n"));
        assert!(!is_http_request_line("GET foo\r\n"));
        assert!(!is_http_request_line("SCAN 0 10\r\n"));
    }

    fn setup_test_server() -> (ActorSystem, ServerHandle) {
        let system = ActorSystem::new("test-server-sys");
        let storage = system.spawn("storage", StorageActor::new()).unwrap();
        let metrics = system.spawn("metrics", MetricsActor::new()).unwrap();

        let config = ServerConfig {
            addr: "127.0.0.1:0".to_string(),
            worker_threads: 2,
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
        };

        let server = HttpServer::with_actors(config, system.clone(), storage, metrics);
        let handle = server.bind().unwrap();
        (system, handle)
    }

    #[test]
    fn test_http_endpoints() {
        let (system, server) = setup_test_server();
        let addr = server.local_addr();

        // 1. GET /health
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains(r#"{"status":"ok"}"#));
        }

        // 2. POST /api/v1/put
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            let body = r#"{"key":"city","value":"london"}"#;
            let req = format!(
                "POST /api/v1/put HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("\"key\":\"city\""));
            assert!(resp.contains("\"value\":\"london\""));
        }

        // 3. GET /api/v1/get?key=city
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"GET /api/v1/get?key=city HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("\"value\":\"london\""));
        }

        // 4. GET /api/v1/scan?limit=10
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"GET /api/v1/scan?limit=10 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("\"count\":1"));
            assert!(resp.contains("\"key\":\"city\""));
        }

        // 5. GET /api/v1/stats
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"GET /api/v1/stats HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("\"total_requests\""));
        }

        // 6. DELETE /api/v1/delete?key=city
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"DELETE /api/v1/delete?key=city HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 200 OK"));
            assert!(resp.contains("\"deleted\":true"));
        }

        // 7. GET after DELETE -> 404
        {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(b"GET /api/v1/get?key=city HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            assert!(resp.starts_with("HTTP/1.1 404 Not Found"));
        }

        server.shutdown();
        system.shutdown();
    }

    #[test]
    fn test_line_protocol() {
        let (system, server) = setup_test_server();
        let addr = server.local_addr();

        let mut stream = TcpStream::connect(addr).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        // PING -> PONG
        stream.write_all(b"PING\r\n").unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "PONG");

        // PUT key value
        stream.write_all(b"PUT user:1 Alice Smith\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK");

        // GET key -> VALUE ...
        stream.write_all(b"GET user:1\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "VALUE Alice Smith");

        // PUT another key
        stream.write_all(b"PUT user:2 Bob Jones\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK");

        // SCAN
        stream.write_all(b"SCAN user 10\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ENTRIES 2");
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("user:1"));
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("user:2"));
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "END");

        // STATS
        stream.write_all(b"STATS\r\n").unwrap();
        line.clear();
        let mut stats_output = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            stats_output.push_str(&line);
            if line.trim() == "END" {
                break;
            }
        }
        assert!(stats_output.contains("STAT total_requests"));

        // DEL key -> OK
        stream.write_all(b"DEL user:1\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "OK");

        // GET deleted key -> NOT_FOUND
        stream.write_all(b"GET user:1\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "NOT_FOUND");

        // QUIT -> BYE
        stream.write_all(b"QUIT\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "BYE");

        server.shutdown();
        system.shutdown();
    }
}
