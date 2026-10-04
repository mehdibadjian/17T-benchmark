use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Metadata entry stored in the SSTable index block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub key: String,
    pub offset: u64,
}

/// Writer for generating immutable Sorted String Table (SSTable) files.
///
/// Layout:
/// - [Data Block]: sorted records of [is_tombstone: 1B, key_len: 4B, key, val_len: 4B, val]
/// - [Index Block]: [entry_count: 4B] followed by [key_len: 4B, key, offset: 8B] for each entry
/// - [Trailer]: [index_offset: 8B, entry_count: 8B, magic: 8B ("TURINGS1")]
pub struct SsTableWriter {
    file: BufWriter<File>,
    path: PathBuf,
    current_offset: u64,
    index: Vec<IndexEntry>,
}

impl SsTableWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;

        Ok(Self {
            file: BufWriter::new(file),
            path,
            current_offset: 0,
            index: Vec::new(),
        })
    }

    /// Appends a single sorted key-value entry to the SSTable data block.
    pub fn write_entry(&mut self, key: &str, value: Option<&[u8]>) -> io::Result<()> {
        let entry_offset = self.current_offset;
        let is_tombstone = if value.is_none() { 1u8 } else { 0u8 };
        let val_bytes = value.unwrap_or(&[]);

        self.file.write_all(&[is_tombstone])?;
        self.file.write_all(&(key.len() as u32).to_le_bytes())?;
        self.file.write_all(key.as_bytes())?;
        self.file.write_all(&(val_bytes.len() as u32).to_le_bytes())?;
        self.file.write_all(val_bytes)?;

        let bytes_written = 1 + 4 + key.len() + 4 + val_bytes.len();
        self.current_offset += bytes_written as u64;

        self.index.push(IndexEntry {
            key: key.to_string(),
            offset: entry_offset,
        });

        Ok(())
    }

    /// Writes index block and trailer metadata, flushing and syncing file to disk.
    pub fn finish(mut self) -> io::Result<()> {
        let index_offset = self.current_offset;
        let entry_count = self.index.len() as u64;

        // Write index block
        self.file.write_all(&(self.index.len() as u32).to_le_bytes())?;
        for entry in &self.index {
            self.file.write_all(&(entry.key.len() as u32).to_le_bytes())?;
            self.file.write_all(entry.key.as_bytes())?;
            self.file.write_all(&entry.offset.to_le_bytes())?;
        }

        // Write trailer (24 bytes)
        self.file.write_all(&index_offset.to_le_bytes())?;
        self.file.write_all(&entry_count.to_le_bytes())?;
        self.file.write_all(b"TURINGS1")?;

        self.file.flush()?;
        self.file.get_ref().sync_all()?;
        Ok(())
    }

    /// Writes a complete slice of sorted entries to an SSTable file.
    pub fn write_all<P: AsRef<Path>>(
        path: P,
        entries: &[(String, Option<Vec<u8>>)],
    ) -> io::Result<()> {
        let mut writer = Self::create(path)?;
        for (k, v) in entries {
            writer.write_entry(k, v.as_deref())?;
        }
        writer.finish()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Reader for disk-backed immutable SSTable files.
///
/// Loads the index block on open for fast binary search without loading full data payload.
pub struct SsTableReader {
    file: File,
    path: PathBuf,
    index: Vec<IndexEntry>,
    file_size: u64,
}

impl SsTableReader {
    pub const TRAILER_SIZE: u64 = 24;
    pub const MAGIC: &'static [u8; 8] = b"TURINGS1";

    /// Opens an SSTable file and loads the index block.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let file_size = file.metadata()?.len();

        if file_size < Self::TRAILER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SSTable file too small for trailer metadata",
            ));
        }

        // Read trailer from end of file
        file.seek(SeekFrom::End(-(Self::TRAILER_SIZE as i64)))?;
        let mut trailer_buf = [0u8; 24];
        file.read_exact(&mut trailer_buf)?;

        let index_offset = u64::from_le_bytes(trailer_buf[0..8].try_into().unwrap());
        let _entry_count = u64::from_le_bytes(trailer_buf[8..16].try_into().unwrap());
        let magic = &trailer_buf[16..24];

        if magic != Self::MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid SSTable magic signature",
            ));
        }

        if index_offset > file_size - Self::TRAILER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupted SSTable index offset pointer",
            ));
        }

        // Seek to index block and deserialize
        file.seek(SeekFrom::Start(index_offset))?;
        let mut count_buf = [0u8; 4];
        file.read_exact(&mut count_buf)?;
        let idx_count = u32::from_le_bytes(count_buf) as usize;

        let mut index = Vec::with_capacity(idx_count);
        for _ in 0..idx_count {
            let mut klen_buf = [0u8; 4];
            file.read_exact(&mut klen_buf)?;
            let klen = u32::from_le_bytes(klen_buf) as usize;

            let mut key_buf = vec![0u8; klen];
            file.read_exact(&mut key_buf)?;
            let key = String::from_utf8(key_buf).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

            let mut off_buf = [0u8; 8];
            file.read_exact(&mut off_buf)?;
            let offset = u64::from_le_bytes(off_buf);

            index.push(IndexEntry { key, offset });
        }

        Ok(Self {
            file,
            path,
            index,
            file_size,
        })
    }

    /// Performs binary search on the in-memory index, seeking to the data block offset on hit.
    ///
    /// Returns:
    /// - `Ok(None)`: key not present in SSTable
    /// - `Ok(Some(None))`: key exists as a delete tombstone
    /// - `Ok(Some(Some(value)))`: key exists with value
    pub fn get(&mut self, key: &str) -> io::Result<Option<Option<Vec<u8>>>> {
        match self.index.binary_search_by(|entry| entry.key.as_str().cmp(key)) {
            Ok(idx) => {
                let offset = self.index[idx].offset;
                let entry = self.read_entry_at(offset)?;
                Ok(Some(entry.1))
            }
            Err(_) => Ok(None),
        }
    }

    /// Convenience method returning active value or None if missing / deleted.
    pub fn get_value(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
        match self.get(key)? {
            Some(Some(val)) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    /// Reads single entry at specified data block byte offset.
    fn read_entry_at(&mut self, offset: u64) -> io::Result<(String, Option<Vec<u8>>)> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut tomb_buf = [0u8; 1];
        self.file.read_exact(&mut tomb_buf)?;
        let is_tombstone = tomb_buf[0] != 0;

        let mut klen_buf = [0u8; 4];
        self.file.read_exact(&mut klen_buf)?;
        let klen = u32::from_le_bytes(klen_buf) as usize;

        let mut key_buf = vec![0u8; klen];
        self.file.read_exact(&mut key_buf)?;
        let key = String::from_utf8(key_buf).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, e)
        })?;

        let mut vlen_buf = [0u8; 4];
        self.file.read_exact(&mut vlen_buf)?;
        let vlen = u32::from_le_bytes(vlen_buf) as usize;

        let mut val_buf = vec![0u8; vlen];
        self.file.read_exact(&mut val_buf)?;

        let value = if is_tombstone { None } else { Some(val_buf) };
        Ok((key, value))
    }

    /// Scans sorted entries starting from `start` (inclusive) up to `limit` entries.
    /// If `start` is empty, scans from the beginning.
    pub fn scan(&mut self, start: &str, limit: usize) -> io::Result<Vec<(String, Option<Vec<u8>>)>> {
        if self.index.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let start_idx = if start.is_empty() {
            0
        } else {
            match self.index.binary_search_by(|e| e.key.as_str().cmp(start)) {
                Ok(i) => i,
                Err(i) => i,
            }
        };

        if start_idx >= self.index.len() {
            return Ok(Vec::new());
        }

        let end_idx = (start_idx + limit).min(self.index.len());
        let count = end_idx - start_idx;
        let mut results = Vec::with_capacity(count);

        // Seek once to starting entry and stream sequential records
        self.file.seek(SeekFrom::Start(self.index[start_idx].offset))?;

        for _ in 0..count {
            let mut tomb_buf = [0u8; 1];
            self.file.read_exact(&mut tomb_buf)?;
            let is_tombstone = tomb_buf[0] != 0;

            let mut klen_buf = [0u8; 4];
            self.file.read_exact(&mut klen_buf)?;
            let klen = u32::from_le_bytes(klen_buf) as usize;

            let mut key_buf = vec![0u8; klen];
            self.file.read_exact(&mut key_buf)?;
            let key = String::from_utf8(key_buf).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

            let mut vlen_buf = [0u8; 4];
            self.file.read_exact(&mut vlen_buf)?;
            let vlen = u32::from_le_bytes(vlen_buf) as usize;

            let mut val_buf = vec![0u8; vlen];
            self.file.read_exact(&mut val_buf)?;

            let val = if is_tombstone { None } else { Some(val_buf) };
            results.push((key, val));
        }

        Ok(results)
    }

    /// Reads all entries in this SSTable file.
    pub fn scan_all(&mut self) -> io::Result<Vec<(String, Option<Vec<u8>>)>> {
        self.scan("", self.index.len())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entry_count(&self) -> usize {
        self.index.len()
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn index(&self) -> &[IndexEntry] {
        &self.index
    }
}
