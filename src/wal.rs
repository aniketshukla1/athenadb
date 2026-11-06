//! Append-only write-ahead log.
//!
//! Every mutation is durably appended here (length-prefixed bincode frames,
//! fsync'd) *before* the in-memory store is touched. On boot the log is
//! replayed on top of the latest snapshot. A torn trailing frame — the
//! signature of a `kill -9` mid-write — is detected and ignored on replay.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::core::{DbError, Entry};

pub const WAL_FILE: &str = "wal.log";
pub const LOCK_FILE: &str = "writer.lock";

/// How long to wait out a lock before calling it held. A lock can outlive its holder for
/// a moment: while any of the holder's threads spawns a process, the child has a copy of
/// every open file until it execs. That took 45 ms at worst, with 16 threads spawning.
const LOCK_GRACE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WalOp {
    Add(Entry),
    Delete(String),
}

pub struct Wal {
    file: File,
    /// Holds the store's writer lock for as long as this log can be appended to. The
    /// operating system releases it when the file closes, even if the process is killed.
    _lock: File,
}

impl Wal {
    /// Open (creating if needed) the WAL for appending, as the store's only writer:
    /// frames from two writers would interleave and corrupt the log.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))?;
        let deadline = Instant::now() + LOCK_GRACE;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(DbError::Locked { store: dir.to_path_buf() });
                }
                // Some file systems can't lock. Writing unprotected would risk the store.
                Err(TryLockError::Error(e)) => {
                    let message = format!("couldn't lock {}: {e}", dir.display());
                    return Err(std::io::Error::new(e.kind(), message).into());
                }
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(dir.join(WAL_FILE))?;
        Ok(Self { file, _lock: lock })
    }

    /// Durably append one operation: write frame, flush, fsync.
    pub fn append(&mut self, op: &WalOp) -> Result<(), DbError> {
        self.append_batch(std::slice::from_ref(op))
    }

    /// Durably append several operations behind a single fsync. A crash mid-batch
    /// leaves a prefix of it: replay keeps the intact frames and ignores a torn tail.
    pub fn append_batch(&mut self, ops: &[WalOp]) -> Result<(), DbError> {
        let mut frames = Vec::new();
        for op in ops {
            let bytes = bincode::serialize(op).map_err(|e| DbError::Serde(e.to_string()))?;
            frames.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            frames.extend_from_slice(&bytes);
        }
        self.file.write_all(&frames)?;
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
