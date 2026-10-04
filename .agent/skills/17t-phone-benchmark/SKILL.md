---
name: 17t-phone-benchmark
description: >-
  Autonomous hardware benchmark runner, diagnostics, and performance profiling suite
  specifically designed for the Xiaomi 17T flagship smartphone (chagall) running
  Xiaomi HyperOS 3.0 on Android 16 with MediaTek Dimensity 9300+ / ARM Cortex-X4.
  Use when evaluating phone performance, multi-core scaling, memory bandwidth,
  storage I/O, actor IPC, or LSM database stress on the device.
---

# Xiaomi 17T Hardware Benchmark & Diagnostics Skill

This skill provides step-by-step procedures, automated runners, and analytical diagnostics for benchmarking the **Xiaomi 17T** flagship phone inside the `/workspace/nimble-turing` environment.

---

## Device & Platform Context

- **Device**: Xiaomi 17T (Codename: `chagall`, Board: `missi`)
- **System**: Xiaomi HyperOS 3.0 based on Android 16 (API 36)
- **SoC**: MediaTek Dimensity 9300+ (4 Cortex-X4 logical cores allocated to container)
- **Architecture**: `aarch64` (ARMv9.2-A with SVE2, ASIMD, AES, SHA-256/512, BF16, I8MM, Atomics)
- **Memory**: 12 GB LPDDR5X RAM (11.09 GB accessible, 4.3+ GB available)
- **Storage**: UFS 4.0 Flash Storage

---

## Available Benchmark Workflows

### Workflow 1: Inspect 17T Hardware Profile
Run the sysinfo introspection engine to report CPU, memory, OS, and platform metrics:
```bash
/workspace/nimble-turing/target/release/nimble-shell sysinfo
```

### Workflow 2: Full Multi-Subagent Hardware Benchmark
Spawns 4 concurrent subagents synchronized by an atomic thread barrier across all cores:
```bash
/workspace/nimble-turing/target/release/nimble-shell bench --all --agents 4 --duration 2
```
Workloads evaluated:
1. **Compute Stress**: 64-bit prime search, 64-round SHA-256 compression, matrix float dot products.
2. **Memory Subsystem**: LPDDR5X sequential multi-MB read/write bandwidth and 262k random stride latency.
3. **IPC & Concurrency**: Multi-agent MPSC channel message passing, contended atomic counters, and mutex contention.
4. **Storage & Workspace I/O**: UFS 4.0 chunk writes with periodic fsync and verified sequential reads.

### Workflow 3: Multi-Core Scaling & Thermal Check
Progressively spins off 1, 2, and 4 subagents to measure multi-core speedup and scaling efficiency:
```bash
/workspace/nimble-turing/target/release/nimble-shell bench --scaling --duration 2
```
*Expected on 17T*:
- 1 Agent: ~5.27M ops/s (1.00x)
- 2 Agents: ~10.38M ops/s (1.97x, 98.4% efficiency)
- 4 Agents: ~20.24M ops/s (3.84x, 96.0% efficiency)
- Efficiency $\ge 90\%$ confirms that the device is running cool without thermal throttling.

### Workflow 4: LSM Database & ACID Transactions Stress Test
Runs the built-in multi-threaded storage and MVCC transaction engine (`turing-app`):
```bash
/workspace/nimble-turing/target/release/turing-server --bench 20000 8
```
Measures:
- Point GET read throughput (> 800,000 ops/sec, sub-microsecond latency)
- Durable write throughput with synchronous WAL flush to disk
- Mixed transactional workloads (70% read / 20% write / 10% delete)

### Workflow 5: Run Automated One-Click 17T Benchmark
Runs the complete master runner script:
```bash
/workspace/nimble-turing/benchmark_17t.sh
```
