use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::storage::LsmStorageEngine;

/// Error types for the Actor runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorError {
    ActorNotFound(String),
    ActorAlreadyExists(String),
    ActorDead(String),
    Timeout,
    SendFailed(String),
    Custom(String),
}

impl std::fmt::Display for ActorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorError::ActorNotFound(name) => write!(f, "Actor '{}' not found", name),
            ActorError::ActorAlreadyExists(name) => write!(f, "Actor '{}' already exists", name),
            ActorError::ActorDead(name) => write!(f, "Actor '{}' is dead or mailbox disconnected", name),
            ActorError::Timeout => write!(f, "Actor ask request timed out"),
            ActorError::SendFailed(msg) => write!(f, "Failed to send message to actor: {}", msg),
            ActorError::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for ActorError {}

/// Messages supported by system and custom actors.
#[derive(Debug, Clone)]
pub enum ActorMessage {
    // Storage operations
    Get { key: String },
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
    Scan { start: Option<String>, limit: usize },

    // Metrics operations
    RecordOp { op_type: String, latency_us: u64 },
    RecordConnectionOpened,
    RecordConnectionClosed,
    GetMetrics,
    ResetMetrics,

    // System / Lifecycle / Ping
    Ping,
    Stop,
    Custom { tag: String, payload: Vec<u8> },
}

impl ActorMessage {
    pub fn put_str(key: impl Into<String>, value: impl Into<String>) -> Self {
        ActorMessage::Put {
            key: key.into(),
            value: value.into().into_bytes(),
        }
    }

    pub fn get(key: impl Into<String>) -> Self {
        ActorMessage::Get { key: key.into() }
    }

    pub fn delete(key: impl Into<String>) -> Self {
        ActorMessage::Delete { key: key.into() }
    }

    pub fn scan(start: Option<String>, limit: usize) -> Self {
        ActorMessage::Scan { start, limit }
    }
}

/// Snapshot of system metrics for reporting in JSON or plain-text line format.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub ops_per_second: f64,
    pub active_connections: usize,
    pub avg_latency_us: f64,
    pub min_latency_us: u64,
    pub max_latency_us: u64,
    pub ops_by_type: HashMap<String, u64>,
    pub uptime_secs: u64,
}

impl MetricsSnapshot {
    pub fn to_json(&self) -> String {
        let mut ops_json = String::from("{");
        let mut first = true;
        let mut keys: Vec<&String> = self.ops_by_type.keys().collect();
        keys.sort();
        for k in keys {
            if !first {
                ops_json.push(',');
            }
            first = false;
            ops_json.push_str(&format!("\"{}\":{}", k, self.ops_by_type[k]));
        }
        ops_json.push('}');

        format!(
            "{{\"status\":\"ok\",\"total_requests\":{},\"ops_per_second\":{:.2},\"active_connections\":{},\"avg_latency_us\":{:.2},\"min_latency_us\":{},\"max_latency_us\":{},\"uptime_secs\":{},\"ops_by_type\":{}}}",
            self.total_requests,
            self.ops_per_second,
            self.active_connections,
            self.avg_latency_us,
            self.min_latency_us,
            self.max_latency_us,
            self.uptime_secs,
            ops_json
        )
    }

    pub fn to_text(&self) -> String {
        let mut out = format!(
            "STAT total_requests {}\r\nSTAT ops_per_sec {:.2}\r\nSTAT active_connections {}\r\nSTAT avg_latency_us {:.2}\r\nSTAT min_latency_us {}\r\nSTAT max_latency_us {}\r\nSTAT uptime_secs {}\r\n",
            self.total_requests,
            self.ops_per_second,
            self.active_connections,
            self.avg_latency_us,
            self.min_latency_us,
            self.max_latency_us,
            self.uptime_secs
        );
        let mut keys: Vec<&String> = self.ops_by_type.keys().collect();
        keys.sort();
        for k in keys {
            out.push_str(&format!("STAT op_{} {}\r\n", k, self.ops_by_type[k]));
        }
        out.push_str("END\r\n");
        out
    }
}

/// Responses returned by actors.
#[derive(Debug, Clone)]
pub enum ActorResponse {
    Ok,
    Value(Option<Vec<u8>>),
    Deleted(bool),
    ScanResult(Vec<(String, Vec<u8>)>),
    Metrics(MetricsSnapshot),
    Pong,
    Error(String),
    Custom { tag: String, payload: Vec<u8> },
}

impl ActorResponse {
    pub fn as_value(&self) -> Option<&[u8]> {
        match self {
            ActorResponse::Value(Some(v)) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ActorResponse::Value(Some(v)) => std::str::from_utf8(v).ok(),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<String> {
        self.as_str().map(|s| s.to_string())
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, ActorResponse::Ok)
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self, ActorResponse::Deleted(true))
    }

    pub fn as_metrics(&self) -> Option<&MetricsSnapshot> {
        match self {
            ActorResponse::Metrics(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_scan(&self) -> Option<&[(String, Vec<u8>)]> {
        match self {
            ActorResponse::ScanResult(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }
}

/// Message envelope containing the message and optional oneshot response sender.
pub struct Envelope {
    pub message: ActorMessage,
    pub reply_sender: Option<Sender<ActorResponse>>,
}

/// Thread-safe handle containing `Sender<Envelope>` to dispatch messages asynchronously or synchronously.
#[derive(Debug, Clone)]
pub struct ActorRef {
    name: String,
    sender: Sender<Envelope>,
}

impl ActorRef {
    pub fn new(name: impl Into<String>, sender: Sender<Envelope>) -> Self {
        Self {
            name: name.into(),
            sender,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Asynchronous fire-and-forget message send (tell pattern).
    pub fn send(&self, msg: ActorMessage) -> Result<(), ActorError> {
        self.sender
            .send(Envelope {
                message: msg,
                reply_sender: None,
            })
            .map_err(|_| ActorError::ActorDead(self.name.clone()))
    }

    /// Synchronous request-response with timeout (ask pattern).
    pub fn ask(&self, msg: ActorMessage, timeout: Duration) -> Result<ActorResponse, ActorError> {
        let (tx, rx) = channel();
        self.sender
            .send(Envelope {
                message: msg,
                reply_sender: Some(tx),
            })
            .map_err(|_| ActorError::ActorDead(self.name.clone()))?;

        rx.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => ActorError::Timeout,
            RecvTimeoutError::Disconnected => ActorError::ActorDead(self.name.clone()),
        })
    }

    /// Synchronous request-response without timeout (blocking).
    pub fn ask_blocking(&self, msg: ActorMessage) -> Result<ActorResponse, ActorError> {
        let (tx, rx) = channel();
        self.sender
            .send(Envelope {
                message: msg,
                reply_sender: Some(tx),
            })
            .map_err(|_| ActorError::ActorDead(self.name.clone()))?;

        rx.recv()
            .map_err(|_| ActorError::ActorDead(self.name.clone()))
    }

    /// Stop the actor.
    pub fn stop(&self) -> Result<(), ActorError> {
        self.send(ActorMessage::Stop)
    }
}

/// Execution context provided to an actor during message processing.
pub struct ActorContext {
    name: String,
    self_ref: ActorRef,
    system: ActorSystem,
    stopped: AtomicBool,
}

impl ActorContext {
    pub fn new(name: impl Into<String>, self_ref: ActorRef, system: ActorSystem) -> Self {
        Self {
            name: name.into(),
            self_ref,
            system,
            stopped: AtomicBool::new(false),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn self_ref(&self) -> &ActorRef {
        &self.self_ref
    }

    pub fn system(&self) -> &ActorSystem {
        &self.system
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn lookup(&self, name: &str) -> Option<ActorRef> {
        self.system.get_actor(name)
    }
}

/// Core Actor trait: processes incoming messages within its execution context.
pub trait Actor: Send + 'static {
    fn handle(&mut self, msg: ActorMessage, ctx: &ActorContext) -> Option<ActorResponse>;

    fn pre_start(&mut self, _ctx: &ActorContext) {}

    fn post_stop(&mut self, _ctx: &ActorContext) {}
}

struct ActorEntry {
    actor_ref: ActorRef,
    thread_handle: Option<JoinHandle<()>>,
}

struct ActorSystemInner {
    name: String,
    actors: Mutex<HashMap<String, ActorEntry>>,
    is_shutdown: AtomicBool,
}

/// Actor system that spawns actors onto worker threads, tracks them by name, and coordinates graceful shutdown.
#[derive(Clone)]
pub struct ActorSystem {
    inner: Arc<ActorSystemInner>,
}

impl ActorSystem {
    pub fn new(name: &str) -> Self {
        Self {
            inner: Arc::new(ActorSystemInner {
                name: name.to_string(),
                actors: Mutex::new(HashMap::new()),
                is_shutdown: AtomicBool::new(false),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown.load(Ordering::SeqCst)
    }

    pub fn spawn<A: Actor>(&self, name: &str, actor: A) -> Result<ActorRef, ActorError> {
        if self.is_shutdown() {
            return Err(ActorError::Custom("ActorSystem is shutdown".to_string()));
        }

        let mut actors = self
            .inner
            .actors
            .lock()
            .map_err(|_| ActorError::Custom("Poisoned lock".to_string()))?;

        if actors.contains_key(name) {
            return Err(ActorError::ActorAlreadyExists(name.to_string()));
        }

        let (tx, rx): (Sender<Envelope>, Receiver<Envelope>) = channel();
        let actor_ref = ActorRef::new(name, tx);
        let ctx = ActorContext::new(name, actor_ref.clone(), self.clone());

        let name_str = name.to_string();
        let handle = thread::Builder::new()
            .name(format!("actor-{}", name_str))
            .spawn(move || {
                let mut actor = actor;
                actor.pre_start(&ctx);

                while !ctx.is_stopped() {
                    match rx.recv() {
                        Ok(envelope) => {
                            let is_stop = matches!(envelope.message, ActorMessage::Stop);
                            let reply = actor.handle(envelope.message, &ctx);
                            if let Some(reply_sender) = envelope.reply_sender {
                                if let Some(resp) = reply {
                                    let _ = reply_sender.send(resp);
                                }
                            }
                            if is_stop || ctx.is_stopped() {
                                break;
                            }
                        }
                        Err(_) => {
                            // All senders dropped
                            break;
                        }
                    }
                }

                actor.post_stop(&ctx);
            })
            .map_err(|e| ActorError::Custom(format!("Failed to spawn actor thread: {}", e)))?;

        actors.insert(
            name.to_string(),
            ActorEntry {
                actor_ref: actor_ref.clone(),
                thread_handle: Some(handle),
            },
        );

        Ok(actor_ref)
    }

    pub fn get_actor(&self, name: &str) -> Option<ActorRef> {
        let actors = self.inner.actors.lock().ok()?;
        actors.get(name).map(|entry| entry.actor_ref.clone())
    }

    pub fn stop_actor(&self, name: &str) -> Result<(), ActorError> {
        let mut actors = self
            .inner
            .actors
            .lock()
            .map_err(|_| ActorError::Custom("Poisoned lock".to_string()))?;

        if let Some(mut entry) = actors.remove(name) {
            let _ = entry.actor_ref.send(ActorMessage::Stop);
            if let Some(handle) = entry.thread_handle.take() {
                let _ = handle.join();
            }
            Ok(())
        } else {
            Err(ActorError::ActorNotFound(name.to_string()))
        }
    }

    pub fn shutdown(&self) {
        if self.inner.is_shutdown.swap(true, Ordering::SeqCst) {
            return;
        }

        let mut actors = match self.inner.actors.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut entries = Vec::new();
        for (_, entry) in actors.drain() {
            entries.push(entry);
        }
        drop(actors);

        // Send stop message to all actors
        for entry in &entries {
            let _ = entry.actor_ref.send(ActorMessage::Stop);
        }

        // Join actor worker threads
        for mut entry in entries {
            if let Some(handle) = entry.thread_handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for ActorSystemInner {
    fn drop(&mut self) {
        if !self.is_shutdown.swap(true, Ordering::SeqCst) {
            let mut actors = match self.actors.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (_, mut entry) in actors.drain() {
                let _ = entry.actor_ref.send(ActorMessage::Stop);
                if let Some(handle) = entry.thread_handle.take() {
                    let _ = handle.join();
                }
            }
        }
    }
}

/// Builtin StorageActor: bridges incoming storage requests to LSM engine or operates as a dedicated worker actor.
pub struct StorageActor {
    engine: Option<Arc<RwLock<LsmStorageEngine>>>,
    store: BTreeMap<String, Vec<u8>>,
}

impl StorageActor {
    pub fn new() -> Self {
        Self {
            engine: None,
            store: BTreeMap::new(),
        }
    }

    pub fn with_engine(engine: Arc<RwLock<LsmStorageEngine>>) -> Self {
        Self {
            engine: Some(engine),
            store: BTreeMap::new(),
        }
    }

    pub fn engine(&self) -> Option<&Arc<RwLock<LsmStorageEngine>>> {
        self.engine.as_ref()
    }
}

impl Default for StorageActor {
    fn default() -> Self {
        Self::new()
    }
}

impl Actor for StorageActor {
    fn handle(&mut self, msg: ActorMessage, _ctx: &ActorContext) -> Option<ActorResponse> {
        match msg {
            ActorMessage::Get { key } => {
                if let Some(ref engine) = self.engine {
                    match engine.write() {
                        Ok(mut eng) => match eng.get(&key) {
                            Ok(val) => Some(ActorResponse::Value(val)),
                            Err(e) => Some(ActorResponse::Error(e.to_string())),
                        },
                        Err(e) => Some(ActorResponse::Error(e.to_string())),
                    }
                } else {
                    let val = self.store.get(&key).cloned();
                    Some(ActorResponse::Value(val))
                }
            }
            ActorMessage::Put { key, value } => {
                if let Some(ref engine) = self.engine {
                    match engine.write() {
                        Ok(mut eng) => match eng.put(&key, &value) {
                            Ok(()) => Some(ActorResponse::Ok),
                            Err(e) => Some(ActorResponse::Error(e.to_string())),
                        },
                        Err(e) => Some(ActorResponse::Error(e.to_string())),
                    }
                } else {
                    self.store.insert(key, value);
                    Some(ActorResponse::Ok)
                }
            }
            ActorMessage::Delete { key } => {
                if let Some(ref engine) = self.engine {
                    match engine.write() {
                        Ok(mut eng) => match eng.delete(&key) {
                            Ok(()) => Some(ActorResponse::Deleted(true)),
                            Err(e) => Some(ActorResponse::Error(e.to_string())),
                        },
                        Err(e) => Some(ActorResponse::Error(e.to_string())),
                    }
                } else {
                    let removed = self.store.remove(&key).is_some();
                    Some(ActorResponse::Deleted(removed))
                }
            }
            ActorMessage::Scan { start, limit } => {
                if let Some(ref engine) = self.engine {
                    let prefix_or_start = start.unwrap_or_default();
                    match engine.write() {
                        Ok(mut eng) => match eng.scan(&prefix_or_start, limit) {
                            Ok(entries) => Some(ActorResponse::ScanResult(entries)),
                            Err(e) => Some(ActorResponse::Error(e.to_string())),
                        },
                        Err(e) => Some(ActorResponse::Error(e.to_string())),
                    }
                } else {
                    let entries: Vec<(String, Vec<u8>)> = match &start {
                        Some(s) if !s.is_empty() => self
                            .store
                            .range(s.clone()..)
                            .take(limit)
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        _ => self
                            .store
                            .iter()
                            .take(limit)
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    };
                    Some(ActorResponse::ScanResult(entries))
                }
            }
            ActorMessage::Ping => Some(ActorResponse::Pong),
            _ => Some(ActorResponse::Error(
                "Unsupported message for StorageActor".to_string(),
            )),
        }
    }
}

/// Builtin MetricsActor: tracks operations per second, request counts, latencies, and active connections.
pub struct MetricsActor {
    start_time: Instant,
    total_requests: u64,
    total_latency_us: u64,
    min_latency_us: u64,
    max_latency_us: u64,
    active_connections: usize,
    ops_by_type: HashMap<String, u64>,
}

impl MetricsActor {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_requests: 0,
            total_latency_us: 0,
            min_latency_us: u64::MAX,
            max_latency_us: 0,
            active_connections: 0,
            ops_by_type: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let uptime_secs = self.start_time.elapsed().as_secs();
        let ops_per_second = if uptime_secs > 0 {
            self.total_requests as f64 / uptime_secs as f64
        } else {
            self.total_requests as f64
        };
        let avg_latency_us = if self.total_requests > 0 {
            self.total_latency_us as f64 / self.total_requests as f64
        } else {
            0.0
        };
        let min_latency = if self.min_latency_us == u64::MAX {
            0
        } else {
            self.min_latency_us
        };

        MetricsSnapshot {
            total_requests: self.total_requests,
            ops_per_second,
            active_connections: self.active_connections,
            avg_latency_us,
            min_latency_us: min_latency,
            max_latency_us: self.max_latency_us,
            ops_by_type: self.ops_by_type.clone(),
            uptime_secs,
        }
    }
}

impl Default for MetricsActor {
    fn default() -> Self {
        Self::new()
    }
}

impl Actor for MetricsActor {
    fn handle(&mut self, msg: ActorMessage, _ctx: &ActorContext) -> Option<ActorResponse> {
        match msg {
            ActorMessage::RecordOp { op_type, latency_us } => {
                self.total_requests += 1;
                self.total_latency_us += latency_us;
                if latency_us < self.min_latency_us {
                    self.min_latency_us = latency_us;
                }
                if latency_us > self.max_latency_us {
                    self.max_latency_us = latency_us;
                }
                *self.ops_by_type.entry(op_type).or_insert(0) += 1;
                None
            }
            ActorMessage::RecordConnectionOpened => {
                self.active_connections += 1;
                None
            }
            ActorMessage::RecordConnectionClosed => {
                self.active_connections = self.active_connections.saturating_sub(1);
                None
            }
            ActorMessage::GetMetrics => Some(ActorResponse::Metrics(self.snapshot())),
            ActorMessage::ResetMetrics => {
                self.start_time = Instant::now();
                self.total_requests = 0;
                self.total_latency_us = 0;
                self.min_latency_us = u64::MAX;
                self.max_latency_us = 0;
                self.ops_by_type.clear();
                Some(ActorResponse::Ok)
            }
            ActorMessage::Ping => Some(ActorResponse::Pong),
            _ => Some(ActorResponse::Error(
                "Unsupported message for MetricsActor".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoActor;
    impl Actor for EchoActor {
        fn handle(&mut self, msg: ActorMessage, _ctx: &ActorContext) -> Option<ActorResponse> {
            match msg {
                ActorMessage::Ping => Some(ActorResponse::Pong),
                ActorMessage::Custom { tag, payload } => {
                    Some(ActorResponse::Custom { tag, payload })
                }
                _ => None,
            }
        }
    }

    #[test]
    fn test_actor_spawn_and_ask() {
        let system = ActorSystem::new("test-sys");
        let echo = system.spawn("echo", EchoActor).unwrap();

        let resp = echo.ask(ActorMessage::Ping, Duration::from_secs(1)).unwrap();
        assert!(matches!(resp, ActorResponse::Pong));

        system.shutdown();
    }

    #[test]
    fn test_actor_not_found_and_duplicate() {
        let system = ActorSystem::new("test-sys");
        assert!(system.get_actor("nonexistent").is_none());

        let _ = system.spawn("echo", EchoActor).unwrap();
        let err = system.spawn("echo", EchoActor).unwrap_err();
        assert_eq!(err, ActorError::ActorAlreadyExists("echo".to_string()));

        system.shutdown();
    }

    #[test]
    fn test_storage_actor() {
        let system = ActorSystem::new("test-storage");
        let storage = system.spawn("storage", StorageActor::new()).unwrap();

        // Put
        let put_resp = storage
            .ask(
                ActorMessage::put_str("name", "pocketdev"),
                Duration::from_secs(1),
            )
            .unwrap();
        assert!(put_resp.is_ok());

        // Get
        let get_resp = storage
            .ask(ActorMessage::get("name"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(get_resp.as_str(), Some("pocketdev"));

        // Scan
        storage
            .ask(ActorMessage::put_str("k1", "v1"), Duration::from_secs(1))
            .unwrap();
        storage
            .ask(ActorMessage::put_str("k2", "v2"), Duration::from_secs(1))
            .unwrap();

        let scan_resp = storage
            .ask(ActorMessage::scan(Some("k".to_string()), 10), Duration::from_secs(1))
            .unwrap();
        let entries = scan_resp.as_scan().unwrap();
        assert!(entries.iter().any(|(k, v)| k == "k1" && v == b"v1"));
        assert!(entries.iter().any(|(k, v)| k == "k2" && v == b"v2"));

        // Delete
        let del_resp = storage
            .ask(ActorMessage::delete("name"), Duration::from_secs(1))
            .unwrap();
        assert!(del_resp.is_deleted());

        let get_again = storage
            .ask(ActorMessage::get("name"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(get_again.as_value(), None);

        system.shutdown();
    }

    #[test]
    fn test_metrics_actor() {
        let system = ActorSystem::new("test-metrics");
        let metrics = system.spawn("metrics", MetricsActor::new()).unwrap();

        metrics
            .send(ActorMessage::RecordOp {
                op_type: "GET".to_string(),
                latency_us: 150,
            })
            .unwrap();
        metrics
            .send(ActorMessage::RecordOp {
                op_type: "PUT".to_string(),
                latency_us: 250,
            })
            .unwrap();
        metrics.send(ActorMessage::RecordConnectionOpened).unwrap();

        // Allow messages to be processed
        thread::sleep(Duration::from_millis(50));

        let resp = metrics
            .ask(ActorMessage::GetMetrics, Duration::from_secs(1))
            .unwrap();
        let snapshot = resp.as_metrics().unwrap();
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.active_connections, 1);
        assert_eq!(snapshot.min_latency_us, 150);
        assert_eq!(snapshot.max_latency_us, 250);
        assert_eq!(snapshot.ops_by_type.get("GET"), Some(&1));
        assert_eq!(snapshot.ops_by_type.get("PUT"), Some(&1));

        let json = snapshot.to_json();
        assert!(json.contains("\"total_requests\":2"));
        assert!(json.contains("\"active_connections\":1"));

        let text = snapshot.to_text();
        assert!(text.contains("STAT total_requests 2"));
        assert!(text.contains("END"));

        system.shutdown();
    }

    #[test]
    fn test_storage_actor_with_engine() {
        let temp_dir = std::env::temp_dir().join(format!("turing_actor_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let engine = LsmStorageEngine::open(&temp_dir).unwrap();
        let engine_arc = Arc::new(RwLock::new(engine));

        let system = ActorSystem::new("test-engine-sys");
        let storage = system
            .spawn("storage", StorageActor::with_engine(engine_arc.clone()))
            .unwrap();

        // Put via actor
        let put_res = storage
            .ask(
                ActorMessage::put_str("db_key", "db_val"),
                Duration::from_secs(1),
            )
            .unwrap();
        assert!(put_res.is_ok());

        // Get via actor
        let get_res = storage
            .ask(ActorMessage::get("db_key"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(get_res.as_str(), Some("db_val"));

        // Scan via actor
        let scan_res = storage
            .ask(ActorMessage::scan(Some("db_".to_string()), 10), Duration::from_secs(1))
            .unwrap();
        let entries = scan_res.as_scan().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "db_key");

        // Delete via actor
        let del_res = storage
            .ask(ActorMessage::delete("db_key"), Duration::from_secs(1))
            .unwrap();
        assert!(del_res.is_deleted());

        let get_del = storage
            .ask(ActorMessage::get("db_key"), Duration::from_secs(1))
            .unwrap();
        assert_eq!(get_del.as_value(), None);

        system.shutdown();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
