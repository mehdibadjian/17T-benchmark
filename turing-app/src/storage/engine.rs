use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use super::memtable::MemTable;
use super::sstable::{SsTableReader, SsTableWriter};
use super::wal::{WalOpType, WalReader, WalWriter};

/// Default threshold in bytes before active MemTable is automatically flushed to SSTable.
pub const DEFAULT_MEMTABLE_THRESHOLD: usize = 4 * 1024 * 1024; // 4MB

/// High-performance Log-Structured Merge-tree (LSM) storage engine.
pub struct LsmStorageEngine {
    data_dir: PathBuf,
    wal_writer: WalWriter,
    memtable: MemTable,
    sstables: Vec<SsTableReader>,
    next_sst_id: u64,
    memtable_threshold: usize,
}

impl LsmStorageEngine {
    /// Opens or creates an LSM storage engine in the specified directory using default threshold.
    pub fn open<P: AsRef<Path>>(data_dir: P) -> io::Result<Self> {
        Self::open_with_threshold(data_dir, DEFAULT_MEMTABLE_THRESHOLD)
    }

    /// Opens or creates an LSM storage engine with a custom MemTable byte-size flush threshold.
    pub fn open_with_threshold<P: AsRef<Path>>(
        data_dir: P,
        threshold: usize,
    ) -> io::Result<Self> {
        let dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        // Discover and load existing SSTables sorted by chronological ID ascending
        let mut sst_entries = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sst") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(id) = stem.parse::<u64>() {
                        sst_entries.push((id, path));
                    }
                }
            }
        }
        sst_entries.sort_by_key(|(id, _)| *id);

        let mut sstables = Vec::with_capacity(sst_entries.len());
        let mut max_sst_id = 0u64;
        for (id, path) in sst_entries {
            if id > max_sst_id {
                max_sst_id = id;
            }
            let reader = SsTableReader::open(&path)?;
            sstables.push(reader);
        }
        let next_sst_id = max_sst_id + 1;

        // WAL recovery: replay valid records into MemTable
        let wal_path = dir.join("wal.log");
        let mut memtable = MemTable::new();
        if wal_path.exists() {
            let recovered = WalReader::recover_and_truncate(&wal_path)?;
            for record in recovered {
                match record.op_type {
                    WalOpType::Put => {
                        if let Some(val) = record.value {
                            memtable.put(record.key, val);
                        }
                    }
                    WalOpType::Delete => {
                        memtable.delete(record.key);
                    }
                }
            }
        }

        let wal_writer = WalWriter::open(&wal_path)?;

        Ok(Self {
            data_dir: dir,
            wal_writer,
            memtable,
            sstables,
            next_sst_id,
            memtable_threshold: threshold,
        })
    }

    /// Writes key-value pair to WAL and MemTable. Flushes to SSTable if threshold is exceeded.
    pub fn put(&mut self, key: &str, value: &[u8]) -> io::Result<()> {
        self.wal_writer.write_put(key, value)?;
        self.wal_writer.flush()?;
        self.memtable.put(key.to_string(), value.to_vec());

        if self.memtable.byte_size() >= self.memtable_threshold {
            self.flush()?;
        }
        Ok(())
    }

    /// Writes tombstone deletion to WAL and MemTable. Flushes to SSTable if threshold is exceeded.
    pub fn delete(&mut self, key: &str) -> io::Result<()> {
        self.wal_writer.write_delete(key)?;
        self.wal_writer.flush()?;
        self.memtable.delete(key.to_string());

        if self.memtable.byte_size() >= self.memtable_threshold {
            self.flush()?;
        }
        Ok(())
    }

    /// Retrieves the value for `key`.
    ///
    /// Reads active MemTable first, then searches SSTables in reverse chronological order (newest to oldest).
    pub fn get(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
        // 1. Check active MemTable
        if let Some(entry) = self.memtable.get(key) {
            match entry {
                Some(val) => return Ok(Some(val)),
                None => return Ok(None), // Tombstone
            }
        }

        // 2. Check SSTables in reverse chronological order
        for sstable in self.sstables.iter_mut().rev() {
            if let Some(entry) = sstable.get(key)? {
                match entry {
                    Some(val) => return Ok(Some(val)),
                    None => return Ok(None), // Tombstone
                }
            }
        }

        Ok(None)
    }

    /// Merges range scan across MemTable and all SSTables.
    ///
    /// Scans keys starting from `prefix_or_start` (inclusive), deduplicating and resolving
    /// tombstones, returning up to `limit` entries in sorted order.
    pub fn scan(
        &mut self,
        prefix_or_start: &str,
        limit: usize,
    ) -> io::Result<Vec<(String, Vec<u8>)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut merged: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();

        // 1. Oldest to newest SSTables
        for sstable in &mut self.sstables {
            let entries = sstable.scan(prefix_or_start, usize::MAX)?;
            for (k, v) in entries {
                merged.insert(k, v);
            }
        }

        // 2. Active MemTable (newest priority)
        let mem_entries = self.memtable.scan(prefix_or_start, usize::MAX);
        for (k, v) in mem_entries {
            merged.insert(k, v);
        }

        // 3. Collect active entries up to limit
        let mut results = Vec::new();
        for (k, v) in merged {
            if let Some(val) = v {
                results.push((k, val));
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Scans all active key-value pairs with a given prefix.
    pub fn scan_prefix(
        &mut self,
        prefix: &str,
        limit: usize,
    ) -> io::Result<Vec<(String, Vec<u8>)>> {
        let all = self.scan(prefix, usize::MAX)?;
        let filtered = all
            .into_iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .take(limit)
            .collect();
        Ok(filtered)
    }

    /// Flushes the active MemTable to a new numbered SSTable on disk and rotates the WAL.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.memtable.is_empty() {
            return Ok(());
        }

        let sst_filename = format!("{:06}.sst", self.next_sst_id);
        let sst_path = self.data_dir.join(&sst_filename);
        self.next_sst_id += 1;

        let entries = self.memtable.entries();
        SsTableWriter::write_all(&sst_path, &entries)?;

        let reader = SsTableReader::open(&sst_path)?;
        self.sstables.push(reader);

        // Rotate WAL: flush, truncate, and reopen
        let wal_path = self.data_dir.join("wal.log");
        let _ = self.wal_writer.flush();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&wal_path)?;
        file.sync_all()?;
        drop(file);

        self.wal_writer = WalWriter::open(&wal_path)?;
        self.memtable.clear();

        Ok(())
    }

    /// Compaction: merges all SSTables into a single consolidated SSTable,
    /// dropping overwritten keys and delete tombstones, and removing obsolete files.
    pub fn compact(&mut self) -> io::Result<()> {
        if self.sstables.is_empty() {
            return Ok(());
        }

        let mut merged: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
        for sstable in &mut self.sstables {
            let entries = sstable.scan_all()?;
            for (k, v) in entries {
                merged.insert(k, v);
            }
        }

        // Drop tombstones and retain latest values
        let compacted_entries: Vec<(String, Option<Vec<u8>>)> = merged
            .into_iter()
            .filter_map(|(k, v)| v.map(|val| (k, Some(val))))
            .collect();

        let sst_filename = format!("{:06}.sst", self.next_sst_id);
        let sst_path = self.data_dir.join(&sst_filename);
        self.next_sst_id += 1;

        SsTableWriter::write_all(&sst_path, &compacted_entries)?;
        let new_reader = SsTableReader::open(&sst_path)?;

        // Remove obsolete SSTable files
        for old_sst in &self.sstables {
            let _ = std::fs::remove_file(old_sst.path());
        }

        self.sstables = vec![new_reader];
        Ok(())
    }

    /// Explicitly flushes the WAL to disk.
    pub fn sync(&mut self) -> io::Result<()> {
        self.wal_writer.flush()
    }

    /// Returns the number of SSTables on disk.
    pub fn sstable_count(&self) -> usize {
        self.sstables.len()
    }

    /// Returns the active MemTable entry count.
    pub fn memtable_entry_count(&self) -> usize {
        self.memtable.entry_count()
    }

    /// Returns the active MemTable byte size.
    pub fn memtable_byte_size(&self) -> usize {
        self.memtable.byte_size()
    }

    /// Returns the configured MemTable byte-size flush threshold.
    pub fn memtable_threshold(&self) -> usize {
        self.memtable_threshold
    }

    /// Sets the MemTable byte-size flush threshold.
    pub fn set_memtable_threshold(&mut self, threshold: usize) {
        self.memtable_threshold = threshold;
    }

    /// Returns the root data directory path.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
