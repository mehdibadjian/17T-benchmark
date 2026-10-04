use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use turing_app::storage::txn::{TxnError, TxnManager};
use turing_app::storage::LsmStorageEngine;

fn temp_engine() -> Arc<Mutex<LsmStorageEngine>> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = format!("/workspace/nimble-turing/.tdd_txn_test_{}_{}", std::process::id(), id);
    let _ = std::fs::remove_dir_all(&path);
    Arc::new(Mutex::new(
        LsmStorageEngine::open(&path).expect("Failed to open temp engine"),
    ))
}

#[test]
fn test_txn_read_your_own_writes() {
    let engine = temp_engine();
    let txn_mgr = TxnManager::new(Arc::clone(&engine));

    // Initial state in base engine
    engine.lock().unwrap().put("user:alice", b"initial_balance_100").unwrap();

    let mut tx1 = txn_mgr.begin().unwrap();
    // Verify reading base engine state
    assert_eq!(
        tx1.get("user:alice").unwrap(),
        Some(b"initial_balance_100".to_vec())
    );

    // Modify in transaction
    tx1.put("user:alice", b"new_balance_250");
    tx1.put("user:bob", b"new_user_bob");

    // Read your own writes before commit
    assert_eq!(
        tx1.get("user:alice").unwrap(),
        Some(b"new_balance_250".to_vec())
    );
    assert_eq!(
        tx1.get("user:bob").unwrap(),
        Some(b"new_user_bob".to_vec())
    );

    // Outside engine still sees uncommitted old state
    assert_eq!(
        engine.lock().unwrap().get("user:alice").unwrap(),
        Some(b"initial_balance_100".to_vec())
    );
    assert_eq!(engine.lock().unwrap().get("user:bob").unwrap(), None);

    // Another transaction tx2 started concurrently also does NOT see tx1's uncommitted writes
    let tx2 = txn_mgr.begin().unwrap();
    assert_eq!(
        tx2.get("user:alice").unwrap(),
        Some(b"initial_balance_100".to_vec())
    );
    assert_eq!(tx2.get("user:bob").unwrap(), None);

    // Commit tx1
    tx1.commit().unwrap();

    // Now outside engine sees committed state
    assert_eq!(
        engine.lock().unwrap().get("user:alice").unwrap(),
        Some(b"new_balance_250".to_vec())
    );
    assert_eq!(
        engine.lock().unwrap().get("user:bob").unwrap(),
        Some(b"new_user_bob".to_vec())
    );
}

#[test]
fn test_txn_snapshot_isolation() {
    let engine = temp_engine();
    let txn_mgr = TxnManager::new(Arc::clone(&engine));

    engine.lock().unwrap().put("stock:AAPL", b"150").unwrap();

    // tx1 begins at T1
    let tx1 = txn_mgr.begin().unwrap();
    assert_eq!(tx1.get("stock:AAPL").unwrap(), Some(b"150".to_vec()));

    // tx2 begins at T2, updates stock:AAPL, and commits at T3
    let mut tx2 = txn_mgr.begin().unwrap();
    tx2.put("stock:AAPL", b"185");
    tx2.put("stock:GOOG", b"2800");
    tx2.commit().unwrap();

    // tx1 still sees its original snapshot at T1 (Snapshot Isolation)
    assert_eq!(
        tx1.get("stock:AAPL").unwrap(),
        Some(b"150".to_vec()),
        "Snapshot isolation must ensure reproducible reads"
    );
    assert_eq!(
        tx1.get("stock:GOOG").unwrap(),
        None,
        "Snapshot isolation must not see keys created after read_ts"
    );

    // tx3 starting after tx2's commit sees the new values
    let tx3 = txn_mgr.begin().unwrap();
    assert_eq!(tx3.get("stock:AAPL").unwrap(), Some(b"185".to_vec()));
    assert_eq!(tx3.get("stock:GOOG").unwrap(), Some(b"2800".to_vec()));
}

#[test]
fn test_txn_rollback() {
    let engine = temp_engine();
    let txn_mgr = TxnManager::new(Arc::clone(&engine));

    engine.lock().unwrap().put("order:1", b"pending").unwrap();

    let mut tx = txn_mgr.begin().unwrap();
    tx.put("order:1", b"completed");
    tx.put("order:2", b"created");
    tx.delete("order:1");

    // Rollback explicitly
    tx.rollback().unwrap();

    // Base engine must be completely untouched
    assert_eq!(engine.lock().unwrap().get("order:1").unwrap(), Some(b"pending".to_vec()));
    assert_eq!(engine.lock().unwrap().get("order:2").unwrap(), None);

    // Starting new tx verifies state is unchanged
    let tx_new = txn_mgr.begin().unwrap();
    assert_eq!(
        tx_new.get("order:1").unwrap(),
        Some(b"pending".to_vec())
    );
    assert_eq!(tx_new.get("order:2").unwrap(), None);
}

#[test]
fn test_txn_write_write_conflict_detection() {
    let engine = temp_engine();
    let txn_mgr = TxnManager::new(Arc::clone(&engine));

    engine.lock().unwrap().put("inventory:item42", b"count=10").unwrap();

    // Tx1 and Tx2 start concurrently with same read_ts
    let mut tx1 = txn_mgr.begin().unwrap();
    let mut tx2 = txn_mgr.begin().unwrap();

    // Both read the same item
    assert_eq!(
        tx1.get("inventory:item42").unwrap(),
        Some(b"count=10".to_vec())
    );
    assert_eq!(
        tx2.get("inventory:item42").unwrap(),
        Some(b"count=10".to_vec())
    );

    // Both attempt to modify item42
    tx1.put("inventory:item42", b"count=9");
    tx2.put("inventory:item42", b"count=8");

    // Tx1 commits first -> Success!
    assert!(tx1.commit().is_ok());

    // Tx2 attempts to commit -> MUST fail with WriteConflict on "inventory:item42"
    let result = tx2.commit();
    match result {
        Err(TxnError::WriteConflict(key)) => {
            assert_eq!(key, "inventory:item42");
        }
        other => panic!("Expected WriteConflict, got {:?}", other),
    }

    // Engine holds Tx1's value
    assert_eq!(
        engine.lock().unwrap().get("inventory:item42").unwrap(),
        Some(b"count=9".to_vec())
    );
}

#[test]
fn test_txn_range_scan_with_pending_writes() {
    let engine = temp_engine();
    let txn_mgr = TxnManager::new(Arc::clone(&engine));

    engine.lock().unwrap().put("k:1", b"v1").unwrap();
    engine.lock().unwrap().put("k:2", b"v2").unwrap();
    engine.lock().unwrap().put("k:3", b"v3").unwrap();

    let mut tx = txn_mgr.begin().unwrap();
    // Update k:2, delete k:1, insert k:2b
    tx.put("k:2", b"v2_updated");
    tx.delete("k:1");
    tx.put("k:2b", b"v2b_new");

    let entries = tx.scan("k:", 10).unwrap();
    let keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
    let values: Vec<String> = entries
        .iter()
        .map(|(_, v)| String::from_utf8_lossy(v).to_string())
        .collect();

    // k:1 should be deleted (tombstoned), k:2 updated, k:2b inserted, k:3 untouched
    assert_eq!(keys, vec!["k:2", "k:2b", "k:3"]);
    assert_eq!(values, vec!["v2_updated", "v2b_new", "v3"]);

    tx.commit().unwrap();

    // Post-commit scan on engine must match
    let engine_entries = engine.lock().unwrap().scan("k:", 10).unwrap();
    let engine_keys: Vec<String> = engine_entries.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(engine_keys, vec!["k:2", "k:2b", "k:3"]);
}

#[test]
fn test_txn_concurrent_bank_transfers_acid() {
    let engine = temp_engine();
    let txn_mgr = Arc::new(TxnManager::new(Arc::clone(&engine)));

    // Initial state: Account A = $1000, Account B = $1000. Total = $2000.
    engine.lock().unwrap().put("acc:A", b"1000").unwrap();
    engine.lock().unwrap().put("acc:B", b"1000").unwrap();

    let num_threads = 6;
    let transfers_per_thread = 50;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();

    let committed_transfers = Arc::new(AtomicUsize::new(0));
    let conflict_aborts = Arc::new(AtomicUsize::new(0));

    for thread_idx in 0..num_threads {
        let mgr: Arc<TxnManager> = Arc::clone(&txn_mgr);
        let bar = Arc::clone(&barrier);
        let commit_counter = Arc::clone(&committed_transfers);
        let abort_counter = Arc::clone(&conflict_aborts);

        handles.push(thread::spawn(move || {
            bar.wait();

            for i in 0..transfers_per_thread {
                let amount = (i % 20) + 1; // transfer $1 to $20
                let transfer_a_to_b = (thread_idx + i) % 2 == 0;

                // Try transaction
                if let Ok(mut tx) = mgr.begin() {
                    let bal_a_bytes = tx.get("acc:A").unwrap().unwrap();
                    let bal_b_bytes = tx.get("acc:B").unwrap().unwrap();

                    let bal_a: i64 = String::from_utf8(bal_a_bytes).unwrap().parse().unwrap();
                    let bal_b: i64 = String::from_utf8(bal_b_bytes).unwrap().parse().unwrap();

                    let (new_a, new_b) = if transfer_a_to_b {
                        (bal_a - amount as i64, bal_b + amount as i64)
                    } else {
                        (bal_a + amount as i64, bal_b - amount as i64)
                    };

                    tx.put("acc:A", new_a.to_string().as_bytes());
                    tx.put("acc:B", new_b.to_string().as_bytes());

                    match tx.commit() {
                        Ok(_) => {
                            commit_counter.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(TxnError::WriteConflict(_)) => {
                            abort_counter.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => panic!("Unexpected transaction error: {:?}", e),
                    }
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // VERIFY ACID INVARIANT: Total balance MUST still be exactly $2000!
    let final_a_bytes = engine.lock().unwrap().get("acc:A").unwrap().unwrap();
    let final_b_bytes = engine.lock().unwrap().get("acc:B").unwrap().unwrap();
    let final_a: i64 = String::from_utf8(final_a_bytes).unwrap().parse().unwrap();
    let final_b: i64 = String::from_utf8(final_b_bytes).unwrap().parse().unwrap();

    let total = final_a + final_b;
    println!(
        "Concurrent Bank Transfers Result: Commits={}, Conflicts={}, Final A={}, Final B={}, Total={}",
        committed_transfers.load(Ordering::Relaxed),
        conflict_aborts.load(Ordering::Relaxed),
        final_a,
        final_b,
        total
    );

    assert_eq!(
        total, 2000,
        "ACID invariant violated: total balance must remain strictly $2000"
    );
    assert!(
        committed_transfers.load(Ordering::Relaxed) > 0,
        "At least some transactions must commit"
    );
}
