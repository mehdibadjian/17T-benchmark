pub mod engine;
pub mod memtable;
pub mod sstable;
pub mod txn;
pub mod wal;

pub use engine::LsmStorageEngine;
pub use memtable::MemTable;
pub use sstable::{IndexEntry, SsTableReader, SsTableWriter};
pub use txn::{Transaction, TxnError, TxnId, TxnManager, TxnStatus};
pub use wal::{crc32, WalOpType, WalReader, WalRecord, WalWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "turing_test_{}_{}_{}",
                prefix,
                std::process::id(),
                count
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_memtable_operations() {
        let mut mem = MemTable::new();
        assert!(mem.is_empty());
        assert_eq!(mem.entry_count(), 0);
        assert_eq!(mem.byte_size(), 0);

        // Put operations
        mem.put("alpha".to_string(), b"val1".to_vec());
        mem.put("beta".to_string(), b"val2".to_vec());
        assert_eq!(mem.entry_count(), 2);
        assert!(mem.byte_size() > 0);

        // Get operations
        assert_eq!(mem.get("alpha"), Some(Some(b"val1".to_vec())));
        assert_eq!(mem.get_value("beta"), Some(b"val2".as_slice()));
        assert_eq!(mem.get("gamma"), None);

        // Overwrite
        let prev_size = mem.byte_size();
        mem.put("alpha".to_string(), b"val1_updated".to_vec());
        assert_eq!(mem.entry_count(), 2);
        assert!(mem.byte_size() > prev_size);
        assert_eq!(mem.get("alpha"), Some(Some(b"val1_updated".to_vec())));

        // Delete (tombstone)
        mem.delete("alpha".to_string());
        assert_eq!(mem.entry_count(), 2);
        assert_eq!(mem.get("alpha"), Some(None));
        assert_eq!(mem.get_value("alpha"), None);

        // Scan operations
        mem.put("delta".to_string(), b"val3".to_vec());
        mem.put("charlie".to_string(), b"val4".to_vec());

        // Scan starting at "beta" with limit 2
        let scanned = mem.scan("beta", 2);
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0].0, "beta");
        assert_eq!(scanned[1].0, "charlie");

        // Scan all
        let all_scanned = mem.scan("", 10);
        assert_eq!(all_scanned.len(), 4);
        assert_eq!(all_scanned[0].0, "alpha");
        assert_eq!(all_scanned[0].1, None); // Tombstone retained

        // Entries
        let entries = mem.entries();
        assert_eq!(entries.len(), 4);

        // Clear
        mem.clear();
        assert!(mem.is_empty());
        assert_eq!(mem.byte_size(), 0);
    }

    #[test]
    fn test_wal_recovery() {
        let temp = TempDir::new("wal_recovery");
        let wal_path = temp.path().join("wal.log");

        // 1. Write valid records
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();
            writer.write_put("k1", b"v1").unwrap();
            writer.write_put("k2", b"v2").unwrap();
            writer.write_delete("k1").unwrap();
            writer.write_put("k3", b"v3").unwrap();
            writer.flush().unwrap();
        }

        // 2. Read and verify records
        let records = WalReader::recover(&wal_path).unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].op_type, WalOpType::Put);
        assert_eq!(records[0].key, "k1");
        assert_eq!(records[0].value, Some(b"v1".to_vec()));
        assert_eq!(records[2].op_type, WalOpType::Delete);
        assert_eq!(records[2].key, "k1");
        assert_eq!(records[2].value, None);
        assert_eq!(records[3].key, "k3");

        // 3. Corrupt trailing bytes to simulate a crash during write
        {
            let mut file = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&wal_path)
                .unwrap();
            // Append corrupted partial record
            file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02]).unwrap();
            file.flush().unwrap();
        }

        // 4. Recovery should safely discard the corrupted trailing write and truncate
        let recovered = WalReader::recover_and_truncate(&wal_path).unwrap();
        assert_eq!(recovered.len(), 4);
        assert_eq!(recovered[3].key, "k3");

        // Verify appending new record works after truncation
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();
            writer.write_put("k4", b"v4").unwrap();
            writer.flush().unwrap();
        }

        let updated_records = WalReader::recover(&wal_path).unwrap();
        assert_eq!(updated_records.len(), 5);
        assert_eq!(updated_records[4].key, "k4");
    }

    #[test]
    fn test_sstable_write_and_read() {
        let temp = TempDir::new("sstable_test");
        let sst_path = temp.path().join("000001.sst");

        let entries = vec![
            ("apple".to_string(), Some(b"fruit_1".to_vec())),
            ("banana".to_string(), None), // tombstone
            ("cherry".to_string(), Some(b"fruit_3".to_vec())),
            ("date".to_string(), Some(b"fruit_4".to_vec())),
            ("elderberry".to_string(), Some(b"fruit_5".to_vec())),
        ];

        // Write SSTable
        SsTableWriter::write_all(&sst_path, &entries).unwrap();

        // Read SSTable
        let mut reader = SsTableReader::open(&sst_path).unwrap();
        assert_eq!(reader.entry_count(), 5);

        // Binary search gets
        assert_eq!(
            reader.get("apple").unwrap(),
            Some(Some(b"fruit_1".to_vec()))
        );
        assert_eq!(reader.get("banana").unwrap(), Some(None)); // tombstone
        assert_eq!(
            reader.get("elderberry").unwrap(),
            Some(Some(b"fruit_5".to_vec()))
        );
        assert_eq!(reader.get("fig").unwrap(), None); // not found

        // Scan starting from "cherry" with limit 2
        let scanned = reader.scan("cherry", 2).unwrap();
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0].0, "cherry");
        assert_eq!(scanned[1].0, "date");

        // Scan all
        let all = reader.scan_all().unwrap();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_lsm_engine_crud_and_flush() {
        let temp = TempDir::new("engine_crud");
        let mut engine = LsmStorageEngine::open(temp.path()).unwrap();

        // Put and Get
        engine.put("user:101", b"Alice").unwrap();
        engine.put("user:102", b"Bob").unwrap();
        assert_eq!(engine.get("user:101").unwrap(), Some(b"Alice".to_vec()));
        assert_eq!(engine.get("user:102").unwrap(), Some(b"Bob".to_vec()));
        assert_eq!(engine.get("user:999").unwrap(), None);

        // Delete
        engine.delete("user:101").unwrap();
        assert_eq!(engine.get("user:101").unwrap(), None);

        // Manual Flush
        assert_eq!(engine.sstable_count(), 0);
        engine.flush().unwrap();
        assert_eq!(engine.sstable_count(), 1);
        assert_eq!(engine.memtable_entry_count(), 0);

        // Verify data persists in SSTable
        assert_eq!(engine.get("user:101").unwrap(), None); // Tombstone persisted
        assert_eq!(engine.get("user:102").unwrap(), Some(b"Bob".to_vec()));

        // Active MemTable shadowing SSTable
        engine.put("user:101", b"Alice_Recreated").unwrap();
        assert_eq!(
            engine.get("user:101").unwrap(),
            Some(b"Alice_Recreated".to_vec())
        );

        engine.delete("user:102").unwrap();
        assert_eq!(engine.get("user:102").unwrap(), None);
    }

    #[test]
    fn test_lsm_engine_threshold_flush() {
        let temp = TempDir::new("engine_threshold");
        // Set low threshold of 50 bytes to force automatic flushes
        let mut engine = LsmStorageEngine::open_with_threshold(temp.path(), 50).unwrap();

        engine.put("key_1", b"large_value_payload_1").unwrap();
        engine.put("key_2", b"large_value_payload_2").unwrap();
        engine.put("key_3", b"large_value_payload_3").unwrap();

        // Automatic flush should have created SSTable(s)
        assert!(engine.sstable_count() >= 1);
        assert_eq!(
            engine.get("key_1").unwrap(),
            Some(b"large_value_payload_1".to_vec())
        );
        assert_eq!(
            engine.get("key_2").unwrap(),
            Some(b"large_value_payload_2".to_vec())
        );
        assert_eq!(
            engine.get("key_3").unwrap(),
            Some(b"large_value_payload_3".to_vec())
        );
    }

    #[test]
    fn test_lsm_engine_compaction() {
        let temp = TempDir::new("engine_compaction");
        let mut engine = LsmStorageEngine::open(temp.path()).unwrap();

        // Generation 1
        engine.put("k1", b"v1_old").unwrap();
        engine.put("k2", b"v2").unwrap();
        engine.flush().unwrap();

        // Generation 2
        engine.put("k1", b"v1_new").unwrap();
        engine.delete("k2").unwrap();
        engine.put("k3", b"v3").unwrap();
        engine.flush().unwrap();

        assert_eq!(engine.sstable_count(), 2);

        // Compact SSTables
        engine.compact().unwrap();
        assert_eq!(engine.sstable_count(), 1);

        // Verify values
        assert_eq!(engine.get("k1").unwrap(), Some(b"v1_new".to_vec()));
        assert_eq!(engine.get("k2").unwrap(), None); // Tombstone dropped, key gone
        assert_eq!(engine.get("k3").unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn test_lsm_engine_scan_merge() {
        let temp = TempDir::new("engine_scan");
        let mut engine = LsmStorageEngine::open(temp.path()).unwrap();

        // SSTable 1
        engine.put("item:01", b"val_1").unwrap();
        engine.put("item:02", b"val_2").unwrap();
        engine.put("item:03", b"val_3_old").unwrap();
        engine.flush().unwrap();

        // SSTable 2
        engine.put("item:03", b"val_3_new").unwrap();
        engine.delete("item:02").unwrap();
        engine.flush().unwrap();

        // Active MemTable
        engine.put("item:04", b"val_4").unwrap();
        engine.delete("item:01").unwrap();

        // Scans all items starting from "item:"
        let results = engine.scan("item:", 10).unwrap();
        // item:01 deleted in MemTable -> excluded
        // item:02 deleted in SSTable 2 -> excluded
        // item:03 updated to val_3_new -> included
        // item:04 in MemTable -> included
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], ("item:03".to_string(), b"val_3_new".to_vec()));
        assert_eq!(results[1], ("item:04".to_string(), b"val_4".to_vec()));
    }

    #[test]
    fn test_lsm_engine_crash_recovery() {
        let temp = TempDir::new("engine_crash");
        let dir = temp.path().to_path_buf();

        // Step 1: Write data without flushing to SSTable, then drop engine
        {
            let mut engine = LsmStorageEngine::open(&dir).unwrap();
            engine.put("session:1", b"active").unwrap();
            engine.put("session:2", b"idle").unwrap();
            engine.delete("session:1").unwrap();
            engine.put("session:3", b"pending").unwrap();
            // engine is dropped here without calling flush()
        }

        // Step 2: Reopen engine from disk - WAL should replay all operations
        {
            let mut engine = LsmStorageEngine::open(&dir).unwrap();
            assert_eq!(engine.sstable_count(), 0);
            assert_eq!(engine.memtable_entry_count(), 3);
            assert_eq!(engine.get("session:1").unwrap(), None); // Tombstone replayed
            assert_eq!(engine.get("session:2").unwrap(), Some(b"idle".to_vec()));
            assert_eq!(engine.get("session:3").unwrap(), Some(b"pending".to_vec()));
        }
    }
}
