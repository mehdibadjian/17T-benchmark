use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use turing_app::actor::{ActorSystem, MetricsActor, StorageActor};
use turing_app::cli::CliClient;
use turing_app::server::{HttpServer, ServerConfig};
use turing_app::storage::LsmStorageEngine;

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(test_name: &str) -> Self {
        let count = TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "turing_integ_{}_{}_{}",
            test_name,
            std::process::id(),
            count
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ============================================================================
// Test 1: Concurrency Test
// Multiple threads performing concurrent PUT, GET, and DELETE operations.
// ============================================================================
#[test]
fn test_concurrency_multi_threaded() {
    let dir = TestDir::new("concurrency");
    let engine = Arc::new(RwLock::new(LsmStorageEngine::open(dir.path()).unwrap()));

    let num_threads = 8;
    let keys_per_thread = 100;
    let mut handles = Vec::new();

    // Spawn 8 worker threads doing concurrent writes
    for tid in 0..num_threads {
        let engine_clone = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            // 1. Initial PUTs
            for i in 0..keys_per_thread {
                let key = format!("k_{}_{}", tid, i);
                let val = format!("val_{}_{}", tid, i);
                let mut eng = engine_clone.write().unwrap();
                eng.put(&key, val.as_bytes()).unwrap();
            }

            // 2. Concurrent GETs of own keys
            for i in 0..keys_per_thread {
                let key = format!("k_{}_{}", tid, i);
                let mut eng = engine_clone.write().unwrap();
                let res = eng.get(&key).unwrap();
                assert!(res.is_some(), "Key {} should exist", key);
                assert_eq!(
                    res.unwrap(),
                    format!("val_{}_{}", tid, i).as_bytes()
                );
            }

            // 3. Overwrites and Deletes
            for i in 0..keys_per_thread {
                let key = format!("k_{}_{}", tid, i);
                let mut eng = engine_clone.write().unwrap();
                if i % 3 == 0 {
                    // Delete
                    eng.delete(&key).unwrap();
                } else if i % 3 == 1 {
                    // Overwrite
                    let new_val = format!("updated_{}_{}", tid, i);
                    eng.put(&key, new_val.as_bytes()).unwrap();
                }
            }
        }));
    }

    // Wait for all concurrent threads to finish
    for h in handles {
        h.join().unwrap();
    }

    // Verify all keys after concurrent operations
    let mut eng = engine.write().unwrap();
    for tid in 0..num_threads {
        for i in 0..keys_per_thread {
            let key = format!("k_{}_{}", tid, i);
            let val = eng.get(&key).unwrap();
            if i % 3 == 0 {
                assert_eq!(val, None, "Key {} was deleted and should return None", key);
            } else if i % 3 == 1 {
                let expected = format!("updated_{}_{}", tid, i);
                assert_eq!(
                    val,
                    Some(expected.into_bytes()),
                    "Key {} should have updated value",
                    key
                );
            } else {
                let expected = format!("val_{}_{}", tid, i);
                assert_eq!(
                    val,
                    Some(expected.into_bytes()),
                    "Key {} should have original value",
                    key
                );
            }
        }
    }

    // Concurrent scan check
    let scan_results = eng.scan("k_", usize::MAX).unwrap();
    // Total remaining keys: 8 threads * 100 * (2/3) ~ 536 keys
    let expected_count = num_threads * (keys_per_thread - keys_per_thread / 3 - if keys_per_thread % 3 > 0 { 1 } else { 0 });
    assert_eq!(scan_results.len(), expected_count);
}

// ============================================================================
// Test 2: Crash Recovery Test
// Write keys, simulate abrupt restart, verify WAL replays state accurately.
// Also tests recovery when WAL has trailing corrupted writes.
// ============================================================================
#[test]
fn test_crash_recovery_wal_replay() {
    let dir = TestDir::new("crash_recovery");

    // Phase 1: Open engine, write keys without flushing to SSTable, then abruptly drop engine
    {
        // High threshold ensures no SSTable flushes occur; all data is in MemTable + WAL
        let mut engine = LsmStorageEngine::open_with_threshold(dir.path(), 10_000_000).unwrap();
        engine.put("alpha", b"val_alpha_1").unwrap();
        engine.put("beta", b"val_beta_1").unwrap();
        engine.put("gamma", b"val_gamma_1").unwrap();
        engine.delete("beta").unwrap();
        engine.put("alpha", b"val_alpha_2").unwrap(); // overwrite
        engine.put("delta", b"val_delta_1").unwrap();
        // Abrupt drop without flush()
        drop(engine);
    }

    // Phase 2: Reopen engine. The WAL must replay all records into the MemTable.
    {
        let mut engine = LsmStorageEngine::open(dir.path()).unwrap();
        assert_eq!(engine.get("alpha").unwrap(), Some(b"val_alpha_2".to_vec()));
        assert_eq!(engine.get("beta").unwrap(), None); // Tombstone correctly replayed!
        assert_eq!(engine.get("gamma").unwrap(), Some(b"val_gamma_1".to_vec()));
        assert_eq!(engine.get("delta").unwrap(), Some(b"val_delta_1".to_vec()));
        assert_eq!(engine.get("epsilon").unwrap(), None);

        // Scan verification
        let scan = engine.scan("", 10).unwrap();
        let keys: Vec<String> = scan.into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["alpha", "delta", "gamma"]);
    }

    // Phase 3: Simulate trailing corrupt writes (e.g. abrupt power failure mid-write)
    {
        let wal_path = dir.path().join("wal.log");
        let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
        // Append 7 bytes of garbage data representing a torn/incomplete WAL record
        file.write_all(b"CORRUPT").unwrap();
        file.sync_all().unwrap();
    }

    // Phase 4: Reopen engine. WalReader::recover_and_truncate must discard the corrupt write,
    // truncate to the last valid record, and recover clean state.
    {
        let mut engine = LsmStorageEngine::open(dir.path()).unwrap();
        assert_eq!(engine.get("alpha").unwrap(), Some(b"val_alpha_2".to_vec()));
        assert_eq!(engine.get("beta").unwrap(), None);
        assert_eq!(engine.get("gamma").unwrap(), Some(b"val_gamma_1".to_vec()));
        assert_eq!(engine.get("delta").unwrap(), Some(b"val_delta_1".to_vec()));

        // Write a new record to verify WAL is healthy after truncation
        engine.put("zeta", b"val_zeta_1").unwrap();
        assert_eq!(engine.get("zeta").unwrap(), Some(b"val_zeta_1".to_vec()));
    }
}

// ============================================================================
// Test 3: Flush and Compaction Test
// Write enough keys to trigger SSTable flushes, verify lookups across SSTables,
// verify compaction merges SSTables and drops tombstones/overwrites.
// ============================================================================
#[test]
fn test_flush_and_compaction() {
    let dir = TestDir::new("compaction");

    // Open engine with very small flush threshold (300 bytes) to force frequent SSTable flushes
    let mut engine = LsmStorageEngine::open_with_threshold(dir.path(), 300).unwrap();

    // 1. Write 50 distinct keys to produce multiple SSTables
    for i in 0..50 {
        let key = format!("k_{:03}", i);
        let val = format!("val_{:03}", i);
        engine.put(&key, val.as_bytes()).unwrap();
    }
    // Flush any remaining active entries to SSTable
    engine.flush().unwrap();

    // Count SSTables in directory
    let sst_count_initial = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("sst"))
        .count();
    assert!(
        sst_count_initial >= 2,
        "Expected multiple SSTables, found {}",
        sst_count_initial
    );

    // Verify all 50 keys are readable across the multiple SSTables
    for i in 0..50 {
        let key = format!("k_{:03}", i);
        let expected = format!("val_{:03}", i);
        assert_eq!(
            engine.get(&key).unwrap(),
            Some(expected.into_bytes()),
            "Failed reading key {} across SSTables",
            key
        );
    }

    // 2. Overwrite some keys and delete others
    engine.put("k_010", b"updated_val_010").unwrap();
    engine.put("k_020", b"updated_val_020").unwrap();
    engine.delete("k_005").unwrap();
    engine.delete("k_015").unwrap();
    // Flush updates/deletions into a new, newer SSTable
    engine.flush().unwrap();

    // Verify multi-SSTable precedence (newest SSTable shadows older SSTables)
    assert_eq!(
        engine.get("k_010").unwrap(),
        Some(b"updated_val_010".to_vec())
    );
    assert_eq!(
        engine.get("k_020").unwrap(),
        Some(b"updated_val_020".to_vec())
    );
    assert_eq!(engine.get("k_005").unwrap(), None); // Tombstone in newer SSTable shadows old value
    assert_eq!(engine.get("k_015").unwrap(), None); // Tombstone in newer SSTable shadows old value
    assert_eq!(
        engine.get("k_001").unwrap(),
        Some(b"val_001".to_vec()) // Untouched key from older SSTable
    );

    // Verify scan across all SSTables returns 48 entries (50 - 2 deleted)
    let scan_before = engine.scan("k_", 100).unwrap();
    assert_eq!(scan_before.len(), 48);

    // 3. Trigger Compaction
    engine.compact().unwrap();

    // Verify only 1 consolidated SSTable exists after compaction
    let sst_count_after = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("sst"))
        .count();
    assert_eq!(
        sst_count_after, 1,
        "Expected exactly 1 SSTable after compaction, found {}",
        sst_count_after
    );

    // Verify all keys remain accurate and identical after compaction
    assert_eq!(
        engine.get("k_010").unwrap(),
        Some(b"updated_val_010".to_vec())
    );
    assert_eq!(
        engine.get("k_020").unwrap(),
        Some(b"updated_val_020".to_vec())
    );
    assert_eq!(engine.get("k_005").unwrap(), None);
    assert_eq!(engine.get("k_015").unwrap(), None);
    assert_eq!(
        engine.get("k_001").unwrap(),
        Some(b"val_001".to_vec())
    );

    let scan_after = engine.scan("k_", 100).unwrap();
    assert_eq!(scan_after.len(), 48);
    assert_eq!(scan_before, scan_after);
}

// ============================================================================
// Test 4: End-to-End Network Test
// Start server in background thread, send HTTP and line protocol requests,
// verify responses and concurrency.
// ============================================================================
#[test]
fn test_end_to_end_network() {
    let dir = TestDir::new("network");
    let engine = Arc::new(RwLock::new(LsmStorageEngine::open(dir.path()).unwrap()));

    let system = ActorSystem::new("e2e-sys");
    let storage_actor = system
        .spawn("storage", StorageActor::with_engine(Arc::clone(&engine)))
        .unwrap();
    let metrics_actor = system.spawn("metrics", MetricsActor::new()).unwrap();

    let config = ServerConfig {
        addr: "127.0.0.1:0".to_string(), // Dynamic port allocation
        worker_threads: 4,
        read_timeout: Duration::from_secs(10),
        write_timeout: Duration::from_secs(10),
    };

    let server = HttpServer::with_actors(config, system.clone(), storage_actor, metrics_actor);
    let handle = server.bind().unwrap();
    let server_addr = handle.local_addr();

    // ------------------------------------------------------------------------
    // Part A: Line-based Protocol (CLI & telnet commands)
    // ------------------------------------------------------------------------
    let mut client = CliClient::new_remote(server_addr.to_string());

    // PING
    assert_eq!(client.ping().unwrap(), "PONG");

    // PUT
    assert_eq!(client.put("net_key1", "net_val1").unwrap(), "OK");
    assert_eq!(client.put("net_key2", "net_val2").unwrap(), "OK");

    // GET
    assert_eq!(
        client.get("net_key1").unwrap(),
        Some("net_val1".to_string())
    );
    assert_eq!(
        client.get("net_key2").unwrap(),
        Some("net_val2".to_string())
    );
    assert_eq!(client.get("net_missing").unwrap(), None);

    // SCAN
    let entries = client.scan(Some("net_key"), 10).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], ("net_key1".to_string(), "net_val1".to_string()));
    assert_eq!(entries[1], ("net_key2".to_string(), "net_val2".to_string()));

    // STATS
    let stats = client.stats().unwrap();
    assert!(stats.contains("STAT"));
    assert!(stats.contains("END"));

    // DEL
    assert!(client.del("net_key1").unwrap());
    assert!(client.del("net_key2").unwrap());
    assert_eq!(client.get("net_key1").unwrap(), None);
    assert_eq!(client.get("net_key2").unwrap(), None);

    // ------------------------------------------------------------------------
    // Part B: HTTP REST Endpoints
    // ------------------------------------------------------------------------
    // 1. GET /health
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
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
        let mut stream = TcpStream::connect(server_addr).unwrap();
        let body = r#"{"key":"http_user","value":"alice"}"#;
        let req = format!(
            "POST /api/v1/put HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"key\":\"http_user\""));
        assert!(resp.contains("\"value\":\"alice\""));
    }

    // 3. GET /api/v1/get?key=http_user
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
        stream
            .write_all(b"GET /api/v1/get?key=http_user HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"value\":\"alice\""));
    }

    // 4. GET /api/v1/scan?start=http_&limit=10
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
        stream
            .write_all(b"GET /api/v1/scan?start=http_&limit=10 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"count\":1"));
        assert!(resp.contains("\"key\":\"http_user\""));
    }

    // 5. GET /api/v1/stats
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
        stream
            .write_all(b"GET /api/v1/stats HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"total_requests\""));
    }

    // 6. DELETE /api/v1/delete?key=http_user
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
        stream
            .write_all(b"DELETE /api/v1/delete?key=http_user HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"deleted\":true"));
    }

    // 7. GET after DELETE -> 404 Not Found
    {
        let mut stream = TcpStream::connect(server_addr).unwrap();
        stream
            .write_all(b"GET /api/v1/get?key=http_user HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 404 Not Found"));
    }

    // ------------------------------------------------------------------------
    // Part C: Multi-Client Concurrent Stress Over Network
    // ------------------------------------------------------------------------
    let num_net_clients = 4;
    let ops_per_net_client = 50;
    let mut net_handles = Vec::new();

    for cid in 0..num_net_clients {
        let addr = server_addr;
        net_handles.push(thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            for i in 0..ops_per_net_client {
                let cmd = format!("PUT net_c{}_{} data_{}\r\n", cid, i, i);
                stream.write_all(cmd.as_bytes()).unwrap();
                stream.flush().unwrap();

                let mut resp_line = String::new();
                reader.read_line(&mut resp_line).unwrap();
                assert_eq!(resp_line.trim(), "OK");
            }
        }));
    }

    for h in net_handles {
        h.join().unwrap();
    }

    // Cleanup
    handle.shutdown();
    system.shutdown();
}
