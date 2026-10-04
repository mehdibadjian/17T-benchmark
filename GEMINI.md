# Antigravity Rules: 17T-benchmark

- Target Phone: **Xiaomi 17T** (HyperOS 3.0 / Android 16 / MediaTek Dimensity 9300+ Cortex-X4).
- Primary Workspaces: `/workspace/nimble-turing` (never write project files outside).
- Active Skill: `17t-phone-benchmark` in `.agent/skills/17t-phone-benchmark/SKILL.md`.
- Language & Toolchain: Rust 1.75.0 (pure standard library, 0 external crates).
- Architecture: 64-bit ARM (`aarch64`).
- Testing: Strict TDD with `cargo test --workspace`.
