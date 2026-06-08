//! Append-only write-ahead log.
//!
//! Every mutation is durably appended here (length-prefixed bincode frames,
//! fsync'd) *before* the in-memory store is touched. On boot the log is
//! replayed on top of the latest snapshot. A torn trailing frame — the
//! signature of a `kill -9` mid-write — is detected and ignored on replay.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::{DbError, Entry};

pub const WAL_FILE: &str = "wal.log";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WalOp {
    Add(Entry),
    Delete(String),
}

pub struct Wal {
    file: File,
}

impl Wal {
    /// Open (creating if needed) the WAL for appending.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(dir.join(WAL_FILE))?;
        Ok(Self { file })
    }

    /// Durably append one operation: write frame, flush, fsync.
    pub fn append(&mut self, op: &WalOp) -> Result<(), DbError> {
        let bytes = bincode::serialize(op).map_err(|e| DbError::Serde(e.to_string()))?;
        let len = bytes.len() as u64;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&bytes)?;
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Read all intact operations from the log. Stops at the first torn or
    /// corrupt frame (treated as end-of-log), so partial writes never abort
    /// recovery.
    pub fn replay(dir: &Path) -> Result<Vec<WalOp>, DbError> {
        let file = match File::open(dir.join(WAL_FILE)) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut reader = BufReader::new(file);
        let mut ops = Vec::new();
        loop {
            let mut lenb = [0u8; 8];
            match reader.read_exact(&mut lenb) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let len = u64::from_le_bytes(lenb) as usize;
            let mut buf = vec![0u8; len];
            match reader.read_exact(&mut buf) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => break, // torn write
                Err(e) => return Err(e.into()),
            }
            match bincode::deserialize::<WalOp>(&buf) {
                Ok(op) => ops.push(op),
                Err(_) => break, // corrupt tail
            }
        }
        Ok(ops)
    }

    /// Truncate the log to empty (called after a snapshot has captured all ops).
    pub fn truncate(&mut self) -> Result<(), DbError> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.sync_all()?;
        Ok(())
    }
}
