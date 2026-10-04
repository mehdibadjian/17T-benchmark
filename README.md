# Nimble-Turing Workspace

High-performance concurrent systems in pure Rust (zero external dependencies), engineered by an autonomous multi-agent developer team.

The workspace consists of two integrated applications:
1. **`nimble-shell`**: An interactive Unix shell featuring an embedded multi-subagent hardware benchmarking engine.
2. **`turing-app` (`turing-server`)**: A high-throughput Log-Structured Merge-tree (LSM) storage engine, concurrent Actor runtime, and multi-threaded dual-protocol (HTTP/REST + Line Socket) network server.

---

## Workspace Structure

```
/workspace/nimble-turing/
├── Cargo.toml                  # Cargo workspace definition
├── README.md                   # Complete architectural and benchmark documentation
├── demo.nsh                    # Shell script demonstrating nimble-shell capabilities
├── src/                        # [nimble-shell] Crate source
│   ├── main.rs                 # CLI entry point, REPL, script runner
│   ├── lib.rs                  # Exported library interfaces
│   ├── sysinfo.rs              # Hardware and OS inspection (/proc reader)
│   ├── shell/                  # Parser, builtins (cd, pwd, bench, sysinfo), executor
│   └── subagents/              # Multi-subagent benchmark orchestrator (Compute, Memory, IPC, I/O)
├── tests/
│   └── shell_tests.rs          # 10 integration tests for nimble-shell
└── turing-app/                 # [turing-app] LSM Engine & Actor Server Crate
    ├── Cargo.toml              # turing-app manifest
    ├── src/
    │   ├── main.rs             # turing-server binary entry point with CLI argument parser
    │   ├── lib.rs              # Re-exports for storage, actor, server, cli
    │   ├── storage/            # LSM-tree storage engine
    │   │   ├── wal.rs          # Write-Ahead Log with IEEE 802.3 CRC32 checksum & crash recovery
    │   │   ├── memtable.rs     # BTreeMap in-memory table with size tracking & tombstones
    │   │   ├── sstable.rs      # Disk-backed immutable SSTable with index binary search
    │   │   ├── engine.rs       # LSM Engine orchestrating MemTable, SSTables, flush, scan, compaction
    │   │   └── mod.rs          # Storage tests (CRUD, compaction, recovery, scans)
    │   ├── actor/              # Concurrent Actor Runtime
    │   │   └── mod.rs          # Actor trait, ActorRef (ask/tell), ActorSystem, StorageActor, MetricsActor
    │   ├── server/             # Multi-Threaded TCP Server
    │   │   └── mod.rs          # Dual protocol (HTTP/1.1 REST + Line Protocol), thread pool, auto-detection
    │   └── cli/                # Interactive Client & Benchmarking Harness
    │       └── mod.rs          # ClientTarget (Local/Remote), interactive REPL, multi-threaded stress tests
    └── tests/
        └── integration_tests.rs # 4 comprehensive end-to-end integration tests
```

---

## 1. `turing-app` Architecture

```
                                +-----------------------------------+
                                |       Client Connections          |
                                |  (HTTP/1.1 REST & Line Protocol)  |
                                +-----------------+-----------------+
                                                  |
                                        +---------v----------+
                                        |    ThreadPool      |
                                        | (8 Worker Threads) |
                                        +---------+----------+
                                                  |
                               +------------------v------------------+
                               |     Protocol Auto-Detection        |
                               |  - HTTP: /api/v1/get, put, scan...  |
                               |  - Line: GET, PUT, DEL, SCAN, PING  |
                               +------------------+------------------+
                                                  |
                               +------------------v------------------+
                               |           Actor System              |
                               |  - Mailboxes (MPSC channels)        |
                               |  - StorageActor (Request router)    |
                               |  - MetricsActor (Latency & stats)   |
                               +------------------+------------------+
                                                  |
                               +------------------v------------------+
                               |         LSM Storage Engine          |
                               |                                     |
                               |  +-------------------------------+  |
                               |  |        Write-Ahead Log        |  |
                               |  |  (Append-only, CRC32, sync)   |  |
                               |  +---------------+---------------+  |
                               |                  |                  |
                               |  +---------------v---------------+  |
                               |  |       Active MemTable         |  |
                               |  |   (BTreeMap, size limit)      |  |
                               |  +---------------+---------------+  |
                               |                  | Flush            |
                               |  +---------------v---------------+  |
                               |  |   Immutable SSTables (Disk)   |  |
                               |  |  - Data block (sorted keys)   |  |
                               |  |  - Index block (binary search)|  |
                               |  |  - Trailer metadata           |  |
                               |  +---------------+---------------+  |
                               |                  | Compaction       |
                               |  +---------------v---------------+  |
                               |  |   Consolidated SSTable        |  |
                               |  |   (Tombstones purged)         |  |
                               |  +-------------------------------+  |
                               +-------------------------------------+
```

### Key Components

- **Write-Ahead Log (`wal.rs`)**:
  - Implements durable, append-only disk logging with an IEEE 802.3 CRC32 checksum.
  - Recovers state on startup and detects torn/corrupted writes from simulated power loss, safely truncating corrupted trailing bytes.
- **MemTable (`memtable.rs`)**:
  - In-memory `BTreeMap` tracking byte allocation dynamically.
  - Explicit tombstone markers (`Option<Vec<u8>> = None`) for deleted entries.
- **SSTable (`sstable.rs`)**:
  - Binary formatted disk files containing sorted key-value pairs and an index block at EOF for $O(\log N)$ binary search seek without reading the whole file.
- **Compaction (`engine.rs`)**:
  - Multi-generation SSTable merge into consolidated single-generation files, discarding obsolete historical versions and tombstones.
- **Actor Runtime (`actor/mod.rs`)**:
  - Asynchronous message passing via `ActorRef::send` (tell) and synchronous request-response via `ActorRef::ask` (ask pattern with oneshot response channels).
- **Dual-Protocol Server (`server/mod.rs`)**:
  - Auto-detects HTTP vs Line protocol over the same TCP socket.
  - Supports HTTP REST (`/health`, `/api/v1/get`, `/api/v1/put`, `/api/v1/delete`, `/api/v1/scan`, `/api/v1/stats`) and Line protocol (`PUT`, `GET`, `DEL`, `SCAN`, `STATS`, `PING`).

---

## 2. Test Verification Suite (39 Tests Passed)

```text
cargo test --workspace

running 10 tests (nimble-shell/tests/shell_tests.rs)
test test_builtins_is_builtin ... ok
test test_compute_subagent_standalone ... ok
test test_io_subagent_standalone ... ok
test test_parser_pipelines_and_redirection ... ok
test test_ipc_subagent_standalone ... ok
test test_parser_tokenization ... ok
test test_shell_execute_string ... ok
test test_sysinfo_collection ... ok
test test_memory_subagent_standalone ... ok
test test_subagent_pool_orchestration ... ok
test result: ok. 10 passed; 0 failed

running 25 tests (turing-app/src/lib.rs)
test actor::tests::test_actor_not_found_and_duplicate ... ok
test actor::tests::test_actor_spawn_and_ask ... ok
test actor::tests::test_storage_actor ... ok
test actor::tests::test_metrics_actor ... ok
test actor::tests::test_storage_actor_with_engine ... ok
test cli::tests::test_cli_command_parser_local ... ok
test cli::tests::test_cli_local_crud_and_scan ... ok
test cli::tests::test_cli_local_benchmark ... ok
test server::tests::test_is_http ... ok
test server::tests::test_json_escaping ... ok
test server::tests::test_parse_put_body_json ... ok
test server::tests::test_parse_put_body_query_override ... ok
test server::tests::test_parse_put_body_urlencoded ... ok
test server::tests::test_parse_query ... ok
test server::tests::test_line_protocol ... ok
test server::tests::test_url_decoding ... ok
test server::tests::test_http_endpoints ... ok
test storage::tests::test_lsm_engine_crash_recovery ... ok
test storage::tests::test_lsm_engine_compaction ... ok
test storage::tests::test_lsm_engine_crud_and_flush ... ok
test storage::tests::test_memtable_operations ... ok
test storage::tests::test_lsm_engine_scan_merge ... ok
test storage::tests::test_lsm_engine_threshold_flush ... ok
test storage::tests::test_sstable_write_and_read ... ok
test storage::tests::test_wal_recovery ... ok
test result: ok. 25 passed; 0 failed

running 4 tests (turing-app/tests/integration_tests.rs)
test test_crash_recovery_wal_replay ... ok
test test_flush_and_compaction ... ok
test test_end_to_end_network ... ok
test test_concurrency_multi_threaded ... ok
test result: ok. 4 passed; 0 failed

Total: 39 passed, 0 failed, 0 ignored.
```

---

## 3. Measured Performance Benchmarks

### LSM Storage Engine Local Stress Benchmark (4 Threads Concurrent)

| Workload Phase | Throughput | Average Latency | p50 Latency | p95 Latency | Errors |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Point GETs (Reads)** | **853,780 ops/s** | **0.51 µs** | 0 µs | 0 µs | **0** |
| **Mixed (70% R / 20% W / 10% D)**| **1,085 ops/s** | **2.51 ms** | 1 µs | 6.50 ms | **0** |
| **Durable PUTs (Sync WAL)** | **493 ops/s** | **5.89 ms** | 1.84 ms | 9.50 ms | **0** |

### Device Hardware Saturation (`nimble-shell bench --all`)

| Subsystem | Throughput | Primary Metric |
| :--- | :---: | :--- |
| **Compute ALU & SHA-256** | **20,153,930 ops/s** | **5.71 MHashes/s** |
| **RAM Bandwidth** | **2,899,100,161 ops/s** | **20.40 GB/s** |
| **IPC Channels** | **131,081,783 ops/s** | **66,172,360 msgs/s** |
| **Workspace I/O** | **8,141 IOPS** | **509.49 MB/s** |

---

## 4. Usage Guide

### Build All Binaries in Release Mode
```bash
cargo build --release --workspace
```
Binaries generated:
- [`target/release/nimble-shell`](file:///workspace/nimble-turing/target/release/nimble-shell)
- [`target/release/turing-server`](file:///workspace/nimble-turing/target/release/turing-server)

### Run the Server
```bash
./target/release/turing-server --port 8088
```

### Launch Interactive CLI Client
```bash
./target/release/turing-server --interactive
```

### Run Multi-Threaded Storage Benchmark
```bash
./target/release/turing-server --bench 20000 8
```

### Run Hardware Benchmark via Nimble-Shell
```bash
./target/release/nimble-shell bench --all --agents 4 --duration 2
```
