# Antigravity Agent Guidelines: Xiaomi 17T Benchmark & Systems Engineering

These rules guide AI assistant agents working within the `17T-benchmark` repository on the Xiaomi 17T smartphone.

## Target Device Specifications
- **Device Model**: Xiaomi 17T (Codename: `chagall`, Device: `missi`)
- **Operating System**: Xiaomi HyperOS 3.0 based on Android 16 (API Level 36)
- **Application Processor**: MediaTek Dimensity 9300+ / ARM Cortex-X4 (Part `0xd87`)
- **Instruction Set**: ARMv9.2-A aarch64 (Features: SVE2, ASIMD, AES, SHA256/512, BF16, I8MM, Atomics)
- **CPU Cores**: 4 Cores allocated to the Linux PocketDev runtime
- **System Memory**: 12 GB LPDDR5X (11.09 GB accessible, 4.3+ GB available)
- **Storage**: UFS 4.0 high-speed flash

## Core Behavioral Guidelines
1. **Workspace Boundary**:
   - Always operate strictly within `/workspace/nimble-turing`. Never write temporary or project files outside this directory.
2. **Architecture-Aware Optimizations**:
   - Build binaries targeting `aarch64` with `--release` flags (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`).
   - Standard library Rust only: avoid unnecessary external crate dependencies to maintain instant compilation and zero network friction on device.
3. **Multi-Subagent Concurrency**:
   - Utilize thread barrier synchronization (`std::sync::Barrier`) across subagents so all CPU cores on the Xiaomi 17T are saturated simultaneously at the exact same clock tick.
   - Collect and display per-agent throughput, min/max latency percentiles, and multi-core scaling efficiency.
4. **Test-Driven Development (TDD)**:
   - For all new subsystems, follow strict Red -> Green -> Refactor methodology.
   - Validate crash recovery (WAL replay with simulated power failure and corrupted write truncation), snapshot isolation, and atomic commit semantics.
