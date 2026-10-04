use super::engine::LsmStorageEngine;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub type TxnId = u64;
pub type Timestamp = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxnError {
    WriteConflict(String),
    AlreadyCommitted,
    AlreadyAborted,
    EngineError(String),
}

impl std::fmt::Display for TxnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxnError::WriteConflict(k) => write!(f, "Write conflict on key: {}", k),
            TxnError::AlreadyCommitted => write!(f, "Transaction already committed"),
            TxnError::AlreadyAborted => write!(f, "Transaction already aborted"),
            TxnError::EngineError(e) => write!(f, "Storage engine error: {}", e),
        }
    }
}

impl std::error::Error for TxnError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnStatus {
    Active,
    Committed(Timestamp),
    Aborted,
}

pub struct TxnManager {
    inner: Arc<TxnManagerInner>,
}

pub struct TxnManagerInner {
    engine: Arc<Mutex<LsmStorageEngine>>,
    clock: AtomicU64,
    txn_id_counter: AtomicU64,
    // Key -> sorted list of (timestamp, Option<value>)
    mvcc_history: RwLock<BTreeMap<String, Vec<(Timestamp, Option<Vec<u8>>)>>>,
    active_txns: Mutex<HashSet<TxnId>>,
}

impl TxnManager {
    pub fn new(engine: Arc<Mutex<LsmStorageEngine>>) -> Self {
        Self {
            inner: Arc::new(TxnManagerInner {
                engine,
                clock: AtomicU64::new(1),
                txn_id_counter: AtomicU64::new(1),
                mvcc_history: RwLock::new(BTreeMap::new()),
                active_txns: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub fn begin(&self) -> Result<Transaction, TxnError> {
        let id = self.inner.txn_id_counter.fetch_add(1, Ordering::SeqCst);
        let read_ts = self.inner.clock.fetch_add(1, Ordering::SeqCst);

        self.inner.active_txns.lock().unwrap().insert(id);

        Ok(Transaction {
            id,
            read_ts,
            pending_writes: BTreeMap::new(),
            status: TxnStatus::Active,
            inner: Arc::clone(&self.inner),
        })
    }
}

pub struct Transaction {
    pub id: TxnId,
    pub read_ts: Timestamp,
    pending_writes: BTreeMap<String, Option<Vec<u8>>>,
    status: TxnStatus,
    inner: Arc<TxnManagerInner>,
}

impl Transaction {
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, TxnError> {
        if self.status != TxnStatus::Active {
            return Err(TxnError::AlreadyAborted);
        }

        // 1. Read-your-own-writes (uncommitted local buffer)
        if let Some(pending) = self.pending_writes.get(key) {
            return Ok(pending.clone());
        }

        // 2. MVCC Snapshot Read: find newest committed version where commit_ts <= self.read_ts
        let history = self.inner.mvcc_history.read().unwrap();
        if let Some(versions) = history.get(key) {
            for (ts, val) in versions.iter().rev() {
                if *ts <= self.read_ts {
                    return Ok(val.clone());
                }
            }
            // Key has history, but all versions were created after read_ts
            return Ok(None);
        }

        // 3. Fallback to base storage engine (for initial data never modified in transactions)
        let mut engine_guard = self.inner.engine.lock().unwrap();
        let base_val = engine_guard
            .get(key)
            .map_err(|e| TxnError::EngineError(e.to_string()))?;

        Ok(base_val)
    }

    pub fn put(&mut self, key: &str, value: &[u8]) {
        self.pending_writes
            .insert(key.to_string(), Some(value.to_vec()));
    }

    pub fn delete(&mut self, key: &str) {
        self.pending_writes.insert(key.to_string(), None);
    }

    pub fn scan(
        &self,
        prefix_or_start: &str,
        limit: usize,
    ) -> Result<Vec<(String, Vec<u8>)>, TxnError> {
        if self.status != TxnStatus::Active {
            return Err(TxnError::AlreadyAborted);
        }

        let mut merged = BTreeMap::new();

        // 1. Read base engine entries
        let base_entries = {
            let mut engine_guard = self.inner.engine.lock().unwrap();
            engine_guard
                .scan(prefix_or_start, limit * 2)
                .map_err(|e| TxnError::EngineError(e.to_string()))?
        };

        for (k, v) in base_entries {
            merged.insert(k, Some(v));
        }

        // 2. Overlay MVCC committed versions <= read_ts
        let history = self.inner.mvcc_history.read().unwrap();
        for (k, versions) in history.iter() {
            if k.starts_with(prefix_or_start) || k.as_str() >= prefix_or_start {
                let mut found = false;
                for (ts, val) in versions.iter().rev() {
                    if *ts <= self.read_ts {
                        merged.insert(k.clone(), val.clone());
                        found = true;
                        break;
                    }
                }
                if !found {
                    // All versions were created after read_ts
                    merged.remove(k);
                }
            }
        }

        // 3. Overlay pending writes (read-your-own-writes)
        for (k, v) in &self.pending_writes {
            if k.starts_with(prefix_or_start) || k.as_str() >= prefix_or_start {
                merged.insert(k.clone(), v.clone());
            }
        }

        // Filter out tombstones and apply limit
        let mut results = Vec::new();
        for (k, v_opt) in merged {
            if let Some(v) = v_opt {
                results.push((k, v));
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    pub fn commit(mut self) -> Result<(), TxnError> {
        if self.status != TxnStatus::Active {
            return Err(TxnError::AlreadyCommitted);
        }

        if self.pending_writes.is_empty() {
            self.status = TxnStatus::Committed(self.read_ts);
            self.inner.active_txns.lock().unwrap().remove(&self.id);
            return Ok(());
        }

        // Write-Write Conflict Detection:
        // Acquire write lock on mvcc_history to ensure atomic validation & commit
        let mut history = self.inner.mvcc_history.write().unwrap();

        for key in self.pending_writes.keys() {
            if let Some(versions) = history.get(key) {
                // If any version was committed after our read_ts, conflict!
                if let Some((latest_commit_ts, _)) = versions.last() {
                    if *latest_commit_ts > self.read_ts {
                        self.status = TxnStatus::Aborted;
                        self.inner.active_txns.lock().unwrap().remove(&self.id);
                        return Err(TxnError::WriteConflict(key.clone()));
                    }
                }
            }
        }

        // No conflict: allocate commit timestamp
        let commit_ts = self.inner.clock.fetch_add(1, Ordering::SeqCst);

        // Atomically write mutations to storage engine (WAL & MemTable)
        let mut engine_guard = self.inner.engine.lock().unwrap();
        for (key, val_opt) in &self.pending_writes {
            // If this key has no prior history, record its baseline engine value at ts = 0
            if !history.contains_key(key) {
                if let Ok(Some(prior)) = engine_guard.get(key) {
                    history.insert(key.clone(), vec![(0, Some(prior))]);
                }
            }

            match val_opt {
                Some(val) => {
                    engine_guard
                        .put(key, val)
                        .map_err(|e| TxnError::EngineError(e.to_string()))?;
                }
                None => {
                    engine_guard
                        .delete(key)
                        .map_err(|e| TxnError::EngineError(e.to_string()))?;
                }
            }

            // Record version in MVCC history
            history
                .entry(key.clone())
                .or_default()
                .push((commit_ts, val_opt.clone()));
        }

        self.status = TxnStatus::Committed(commit_ts);
        self.inner.active_txns.lock().unwrap().remove(&self.id);

        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), TxnError> {
        if self.status != TxnStatus::Active {
            return Err(TxnError::AlreadyAborted);
        }

        self.pending_writes.clear();
        self.status = TxnStatus::Aborted;
        self.inner.active_txns.lock().unwrap().remove(&self.id);

        Ok(())
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.status == TxnStatus::Active {
            self.pending_writes.clear();
            self.status = TxnStatus::Aborted;
            self.inner.active_txns.lock().unwrap().remove(&self.id);
        }
    }
}
