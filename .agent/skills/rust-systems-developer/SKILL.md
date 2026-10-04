---
name: rust-systems-developer
description: >-
  Expert engineering skill for designing, testing, and optimizing concurrent systems,
  LSM-tree storage engines, actor runtimes, and MVCC transaction systems in pure standard library Rust
  on aarch64 Linux/Android environments.
---

# Rust Systems Developer Skill

This skill guides the design, implementation, and verification of concurrent Rust systems without external dependencies on mobile/embedded ARMv9 environments.

## Architectural Patterns
1. **LSM-Tree Storage Engine**:
   - Write-Ahead Log (WAL) with CRC32 checksums and crash recovery.
   - In-memory MemTable using `BTreeMap` with size thresholds.
   - Immutable SSTables with binary search index blocks at file trailer.
   - Compaction: merging multi-generation SSTables into consolidated single files, purging tombstones.
2. **Actor Runtime**:
   - Actor trait with `handle(&mut self, msg, ctx) -> Option<Response>`.
   - Mailboxes using `std::sync::mpsc::channel`.
   - Ask/Tell pattern with oneshot response channels.
3. **MVCC ACID Transactions**:
   - Monotonic logical clock for snapshot timestamps (`read_ts`, `commit_ts`).
   - Private uncommitted mutation buffer (`pending_writes`).
   - First-committer-wins write-write conflict detection on `commit()`.
4. **Testing Philosophy**:
   - Test-Driven Development (TDD): write failing unit and integration tests first.
   - Run `cargo test --workspace` to ensure all tests pass.
