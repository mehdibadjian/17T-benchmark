use std::collections::BTreeMap;

/// In-memory table backed by a BTreeMap with tombstone deletion support.
///
/// An entry mapping `key -> None` represents a delete tombstone.
/// Tracks approximate memory byte size and entry count.
#[derive(Debug, Default, Clone)]
pub struct MemTable {
    table: BTreeMap<String, Option<Vec<u8>>>,
    byte_size: usize,
}

impl MemTable {
    /// Creates an empty MemTable.
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
            byte_size: 0,
        }
    }

    /// Inserts or updates a key-value pair.
    pub fn put(&mut self, key: String, value: Vec<u8>) {
        let key_len = key.len();
        let val_len = value.len();
        if let Some(old) = self.table.insert(key, Some(value)) {
            let old_size = key_len + old.as_ref().map_or(0, |v| v.len());
            self.byte_size = self.byte_size.saturating_sub(old_size);
        }
        self.byte_size += key_len + val_len;
    }

    /// Inserts a delete tombstone (`None`) for the key.
    pub fn delete(&mut self, key: String) {
        let key_len = key.len();
        if let Some(old) = self.table.insert(key, None) {
            let old_size = key_len + old.as_ref().map_or(0, |v| v.len());
            self.byte_size = self.byte_size.saturating_sub(old_size);
        }
        self.byte_size += key_len;
    }

    /// Looks up a key in the MemTable.
    ///
    /// Returns:
    /// - `None` if the key is not present in this MemTable.
    /// - `Some(None)` if the key has a delete tombstone.
    /// - `Some(Some(value))` if the key is present with a value.
    pub fn get(&self, key: &str) -> Option<Option<Vec<u8>>> {
        self.table.get(key).cloned()
    }

    /// Returns a borrowed reference to the inner Option value, if the key is present.
    pub fn get_ref(&self, key: &str) -> Option<&Option<Vec<u8>>> {
        self.table.get(key)
    }

    /// Convenience method returning the active value bytes, or None if missing / deleted.
    pub fn get_value(&self, key: &str) -> Option<&[u8]> {
        match self.table.get(key) {
            Some(Some(val)) => Some(val.as_slice()),
            _ => None,
        }
    }

    /// Performs a range scan starting from `start` (inclusive) returning up to `limit` entries.
    /// If `start` is empty, scans from the first entry.
    pub fn scan(&self, start: &str, limit: usize) -> Vec<(String, Option<Vec<u8>>)> {
        if limit == 0 {
            return Vec::new();
        }
        self.table
            .range(start.to_string()..)
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Returns all entries in sorted order (used when flushing to SSTable).
    pub fn entries(&self) -> Vec<(String, Option<Vec<u8>>)> {
        self.table
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Clears all entries and resets the byte size counter.
    pub fn clear(&mut self) {
        self.table.clear();
        self.byte_size = 0;
    }

    /// Returns the approximate memory byte size of all keys and values.
    pub fn byte_size(&self) -> usize {
        self.byte_size
    }

    /// Returns the total number of entries (including tombstones).
    pub fn entry_count(&self) -> usize {
        self.table.len()
    }

    /// Returns the number of entries in the MemTable.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Returns true if the MemTable contains no entries.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Returns true if the key exists in the MemTable (as either value or tombstone).
    pub fn contains_key(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }
}
