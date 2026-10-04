# Xiaomi 17T Hardware Benchmark & Systems Suite (`17T-benchmark`)

A high-performance system benchmarking suite, interactive Unix shell (`nimble-shell`), concurrent LSM storage engine, and MVCC transaction runtime (`turing-app`) built in pure Rust (zero external dependencies) and optimized for the **Xiaomi 17T** flagship smartphone.

Engineered with autonomous AI developer subagents and strict **Test-Driven Development (TDD)**.

---

## Target Phone Specifications (Xiaomi 17T)

Introspected directly from the hardware and `/product/etc/build.prop`:

| Property | Value |
| :--- | :--- |
| **Phone Model** | **Xiaomi 17T** (Codename: `chagall`, Device: `missi`) |
| **Operating System** | **Xiaomi HyperOS 3.0** based on **Android 16** (API Level 36) |
| **SoC / Chipset** | **MediaTek Dimensity 9300+** Flagship Processor |
| **CPU Core Architecture** | **ARMv9.2-A Cortex-X4** (Part `0xd87`) |
| **CPU Cores Active** | 4 Logical Cores (Dedicated to PocketDev container) |
| **Hardware Features** | SVE2, ASIMD, AES, SHA-256, SHA-512, BF16, I8MM, Atomics |
| **System Memory (RAM)** | **12 GB LPDDR5X** (11.09 GB accessible, 4.3+ GB available) |
| **Storage Subsystem** | **UFS 4.0** Flash Storage |
| **Linux Runtime** | Ubuntu 20.04.6 LTS on Kernel Linux 6.6 aarch64 |

---

## Antigravity AI Agent & Skills Integration

This repository includes native configuration for **Antigravity (agy)** AI coding agents:

- **[`AGENTS.md`](AGENTS.md) & [`GEMINI.md`](GEMINI.md)**: Workspace agent configuration, guidelines, and toolchain rules.
- **[`.agent/rules/17t-benchmark.md`](.agent/rules/17t-benchmark.md)**: Agent operational rules, aarch64 compiler optimization flags, and barrier synchronization protocols.
- **[`.agent/skills/17t-phone-benchmark/SKILL.md`](.agent/skills/17t-phone-benchmark/SKILL.md)**:
  Dedicated Antigravity skill for autonomous benchmarking of the Xiaomi 17T:
  - Automated hardware profiling & HyperOS detection.
  - Multi-subagent compute, memory, and I/O saturation.
  - Multi-core scaling and thermal throttling analysis.
  - Scripts: [`bench_all.sh`](.agent/skills/17t-phone-benchmark/scripts/bench_all.sh), [`scaling_analysis.sh`](.agent/skills/17t-phone-benchmark/scripts/scaling_analysis.sh), [`lsm_stress.sh`](.agent/skills/17t-phone-benchmark/scripts/lsm_stress.sh).
- **[`.agent/skills/rust-systems-developer/SKILL.md`](.agent/skills/rust-systems-developer/SKILL.md)**:
  Engineering skill for concurrent actor runtimes, LSM storage engines, and MVCC transaction systems on aarch64.

---

## One-Click Master Benchmark Runner

To run the complete benchmark battery on your Xiaomi 17T:

```bash
./benchmark_17t.sh
```

---

## Measured Xiaomi 17T Performance Results

### 1. Multi-Subagent Hardware Saturation (4 Concurrent Agents)

```
========================================================================
                 XIAOMI 17T HARDWARE BENCHMARK SUMMARY                  
========================================================================
  • Compute Stress           :     20,153,930 aggregate ops/s | 5.71 MHashes/s (SHA-256)
  • Memory Bandwidth         :  2,899,100,161 aggregate ops/s | 20.40 GB/s (LPDDR5X RAM)
  • IPC & Concurrency        :    131,081,783 aggregate ops/s | 66,172,360 msgs/s (MPSC)
  • Flash Storage I/O        :          8,141 aggregate ops/s | 509.49 MB/s (UFS 4.0 R/W)
========================================================================
```

### 2. Multi-Core Scaling & Thermal Check (Dimensity 9300+)

Progressively evaluated across active Cortex-X4 cores:

| Subagents | Aggregate Throughput | Speedup vs 1-Agent | Multi-Core Efficiency | Thermal Status |
| :---: | :---: | :---: | :---: | :---: |
| **1 Agent** | 5,272,414 ops/s | **1.00x** | **100.0%** | Optimal (Cool) |
| **2 Agents** | 10,375,710 ops/s | **1.97x** | **98.4%** | Optimal (Cool) |
| **4 Agents** | 20,238,351 ops/s | **3.84x** | **96.0%** | **No Throttling** |

> **Analysis**: The Dimensity 9300+ achieves **96.0% multi-core efficiency** with a **3.84x speedup**, proving excellent sustained thermal headroom and zero core frequency throttling.

### 3. Local LSM Database & MVCC Transactions Benchmark

Stress test running 30,000 operations across 8 concurrent worker threads:

| Operation Type | Throughput | Average Latency | p50 Latency | p99 Latency |
| :--- | :---: | :---: | :---: | :---: |
| **Point GETs (Reads)** | **853,780 ops/s** | **0.51 µs** | 0 µs | 1 µs |
| **Mixed Workload (70% R / 20% W / 10% D)**| **1,085–2,028 ops/s** | **2.51 ms** | 1 µs | 35.1 ms |
| **Durable Writes (Sync WAL to disk)**| **493–706 ops/s** | **5.89 ms** | 1.84 ms | 155.8 ms |

---

## Test Verification Suite (45 Tests Passed)

All 45 tests pass in ~8 seconds:

```bash
cargo test --workspace
```

| Crate / Target | Test Suite | Tests Passed | Status |
| :--- | :--- | :---: | :---: |
| **`nimble-shell`** | [`tests/shell_tests.rs`](tests/shell_tests.rs) (Parser, Builtins, Subagents, Sysinfo) | **10** | Passed |
| **`turing-app`** | [`src/lib.rs`](turing-app/src/lib.rs) (WAL, MemTable, SSTables, Actor, Server) | **25** | Passed |
| **`turing-app`** | [`tests/integration_tests.rs`](turing-app/tests/integration_tests.rs) (Crash recovery, Sockets) | **4** | Passed |
| **`turing-app`** | [`tests/mvcc_txn_tests.rs`](turing-app/tests/mvcc_txn_tests.rs) (**TDD MVCC ACID Suite**) | **6** | Passed |
| **Total** | | **45** | **All Passed** |

---

## Quick Reference Commands

```bash
# Run one-click master 17T benchmark
./benchmark_17t.sh

# Run hardware scaling check (1 to 4 cores)
./target/release/nimble-shell bench --scaling --duration 2

# Inspect 17T hardware profile
./target/release/nimble-shell sysinfo

# Start the concurrent network server on port 8088
./target/release/turing-server --port 8088

# Launch interactive CLI client
./target/release/turing-server --interactive

# Run full test suite
cargo test --workspace
```
