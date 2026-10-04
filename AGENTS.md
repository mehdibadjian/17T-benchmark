# Antigravity Agent Configuration: 17T-benchmark

This repository is optimized for autonomous AI coding agents (Antigravity / agy) developing and running high-performance concurrent Rust benchmarks on the **Xiaomi 17T** flagship phone.

## Device Identification
- **Hardware**: Xiaomi 17T (`chagall`)
- **Firmware**: Xiaomi HyperOS 3.0 (Android 16 SDK 36)
- **SoC**: MediaTek Dimensity 9300+ (ARMv9.2-A Cortex-X4 part `0xd87`)
- **Memory**: 12 GB LPDDR5X (11.09 GB in PocketDev Linux container)

## Agent Customizations & Skills Available
- **`17t-phone-benchmark`** ([`.agent/skills/17t-phone-benchmark/SKILL.md`](.agent/skills/17t-phone-benchmark/SKILL.md)):
  Executes comprehensive hardware diagnostics, multi-core scaling, compute stress, memory bandwidth, and I/O testing specifically on the Xiaomi 17T.
- **`rust-systems-developer`** ([`.agent/skills/rust-systems-developer/SKILL.md`](.agent/skills/rust-systems-developer/SKILL.md)):
  Provides runbooks and design patterns for LSM storage engines, actor runtime concurrency, and MVCC transaction systems in standard library Rust.

## Commands for Agents
```bash
# Run full Xiaomi 17T benchmark suite
./benchmark_17t.sh

# Run hardware scaling check (1 to 4 cores)
./target/release/nimble-shell bench --scaling --duration 2

# Inspect device and HyperOS profile
./target/release/nimble-shell sysinfo

# Run all 45 unit, integration, and TDD tests
cargo test --workspace
```
