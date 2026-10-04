use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Computes the IEEE 802.3 CRC32 checksum of data.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let table_idx = ((crc ^ (byte as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC_TABLE[table_idx];
    }
    !crc
}

/// Returns current Unix epoch timestamp in nanoseconds.
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WalOpType {
    Delete = 0,
    Put = 1,
}

impl TryFrom<u8> for WalOpType {
    type Error = io::Error;

    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(WalOpType::Delete),
            1 => Ok(WalOpType::Put),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid WalOpType byte: {}", val),
            )),
        }
    }
}

/// A single record in the Write-Ahead Log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub crc: u32,
    pub timestamp: u64,
    pub op_type: WalOpType,
    pub key: String,
    pub value: Option<Vec<u8>>,
}

impl WalRecord {
    pub fn new_put(key: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        let key = key.into();
        let value = Some(value.into());
        let timestamp = current_timestamp();
        let op_type = WalOpType::Put;
        let crc = Self::compute_crc(timestamp, op_type, &key, value.as_deref());
        Self {
            crc,
            timestamp,
            op_type,
            key,
            value,
        }
    }

    pub fn new_delete(key: impl Into<String>) -> Self {
        let key = key.into();
        let value = None;
        let timestamp = current_timestamp();
        let op_type = WalOpType::Delete;
        let crc = Self::compute_crc(timestamp, op_type, &key, None);
        Self {
            crc,
            timestamp,
            op_type,
            key,
            value,
        }
    }

    /// Computes CRC over [timestamp, op_type, key_len, key, val_len, val].
    pub fn compute_crc(
        timestamp: u64,
        op_type: WalOpType,
        key: &str,
        value: Option<&[u8]>,
    ) -> u32 {
        let val_bytes = value.unwrap_or(&[]);
        let mut buf = Vec::with_capacity(8 + 1 + 4 + key.len() + 4 + val_bytes.len());
        buf.extend_from_slice(&timestamp.to_le_bytes());
        buf.push(op_type as u8);
        buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
        buf.extend_from_slice(key.as_bytes());
        buf.extend_from_slice(&(val_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(val_bytes);
        crc32(&buf)
    }

    /// Binary serialization:
    /// [crc: 4B][timestamp: 8B][op_type: 1B][key_len: 4B][key][val_len: 4B][val]
    pub fn serialize(&self) -> Vec<u8> {
        let val_bytes = self.value.as_deref().unwrap_or(&[]);
        let mut buf = Vec::with_capacity(4 + 8 + 1 + 4 + self.key.len() + 4 + val_bytes.len());
        buf.extend_from_slice(&self.crc.to_le_bytes());
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.push(self.op_type as u8);
        buf.extend_from_slice(&(self.key.len() as u32).to_le_bytes());
        buf.extend_from_slice(self.key.as_bytes());
        buf.extend_from_slice(&(val_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(val_bytes);
        buf
    }
}

/// Appends records to the Write-Ahead Log with fsync support.
pub struct WalWriter {
    file: File,
    path: PathBuf,
}

impl WalWriter {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&path)?;
        Ok(Self { file, path })
    }

    pub fn append(&mut self, record: &WalRecord) -> io::Result<()> {
        let bytes = record.serialize();
        self.file.write_all(&bytes)?;
        Ok(())
    }

    pub fn write_put(&mut self, key: &str, value: &[u8]) -> io::Result<WalRecord> {
        let record = WalRecord::new_put(key, value);
        self.append(&record)?;
        Ok(record)
    }

    pub fn write_delete(&mut self, key: &str) -> io::Result<WalRecord> {
        let record = WalRecord::new_delete(key);
        self.append(&record)?;
        Ok(record)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Reader and recovery engine for the Write-Ahead Log.
pub struct WalReader {
    path: PathBuf,
}

impl WalReader {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Ok(Self::new(path))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads all valid records sequentially from WAL.
    /// Trailing corrupted or incomplete writes are safely discarded.
    /// Returns the vector of valid records and the valid file byte offset.
    pub fn read_all_valid<P: AsRef<Path>>(path: P) -> io::Result<(Vec<WalRecord>, u64)> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok((Vec::new(), 0));
        }

        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut records = Vec::new();
        let mut valid_offset = 0u64;
        let mut current_offset = 0u64;

        while current_offset < file_len {
            // 1. Read CRC (4 bytes)
            if current_offset + 4 > file_len {
                break; // Incomplete CRC
            }
            let mut crc_buf = [0u8; 4];
            if file.read_exact(&mut crc_buf).is_err() {
                break;
            }
            let stored_crc = u32::from_le_bytes(crc_buf);
            current_offset += 4;

            // 2. Read timestamp (8B) + op_type (1B) + key_len (4B) = 13 bytes
            if current_offset + 13 > file_len {
                break;
            }
            let mut header_buf = [0u8; 13];
            if file.read_exact(&mut header_buf).is_err() {
                break;
            }
            let timestamp = u64::from_le_bytes(header_buf[0..8].try_into().unwrap());
            let op_type_byte = header_buf[8];
            let op_type = match WalOpType::try_from(op_type_byte) {
                Ok(op) => op,
                Err(_) => break, // Corrupted op_type
            };
            let key_len = u32::from_le_bytes(header_buf[9..13].try_into().unwrap()) as usize;
            current_offset += 13;

            // 3. Read key (key_len bytes)
            if current_offset + (key_len as u64) > file_len {
                break;
            }
            let mut key_buf = vec![0u8; key_len];
            if file.read_exact(&mut key_buf).is_err() {
                break;
            }
            let key = match String::from_utf8(key_buf) {
                Ok(k) => k,
                Err(_) => break, // Corrupted utf-8 key
            };
            current_offset += key_len as u64;

            // 4. Read val_len (4 bytes)
            if current_offset + 4 > file_len {
                break;
            }
            let mut vlen_buf = [0u8; 4];
            if file.read_exact(&mut vlen_buf).is_err() {
                break;
            }
            let val_len = u32::from_le_bytes(vlen_buf) as usize;
            current_offset += 4;

            // 5. Read value (val_len bytes)
            if current_offset + (val_len as u64) > file_len {
                break;
            }
            let mut val_buf = vec![0u8; val_len];
            if file.read_exact(&mut val_buf).is_err() {
                break;
            }
            current_offset += val_len as u64;

            let value = if op_type == WalOpType::Delete {
                None
            } else {
                Some(val_buf)
            };

            // 6. Checksum validation
            let computed_crc = WalRecord::compute_crc(timestamp, op_type, &key, value.as_deref());
            if computed_crc != stored_crc {
                // Checksum mismatch -> corrupted trailing write
                break;
            }

            records.push(WalRecord {
                crc: stored_crc,
                timestamp,
                op_type,
                key,
                value,
            });

            valid_offset = current_offset;
        }

        Ok((records, valid_offset))
    }

    /// Replays valid records from the WAL file.
    pub fn recover<P: AsRef<Path>>(path: P) -> io::Result<Vec<WalRecord>> {
        let (records, _) = Self::read_all_valid(path)?;
        Ok(records)
    }

    /// Replays valid records and truncates any corrupted trailing writes from the file.
    pub fn recover_and_truncate<P: AsRef<Path>>(path: P) -> io::Result<Vec<WalRecord>> {
        let path = path.as_ref();
        let (records, valid_offset) = Self::read_all_valid(path)?;
        if path.exists() {
            let metadata = std::fs::metadata(path)?;
            if metadata.len() > valid_offset {
                let file = OpenOptions::new().write(true).open(path)?;
                file.set_len(valid_offset)?;
                file.sync_all()?;
            }
        }
        Ok(records)
    }
}
