//! Point-in-time snapshots of the vector store.
//!
//! Written atomically (temp file + fsync + rename) so a crash mid-snapshot
//! never corrupts the previous good snapshot. After a successful snapshot the
//! WAL is truncated (see `db::Database::snapshot`).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::core::{DbError, Entry, VectorStore};

pub const SNAPSHOT_FILE: &str = "snapshot.bin";
pub const SNAPSHOT_TMP: &str = "snapshot.bin.tmp";

/// Serialize the store to `snapshot.bin` atomically.
pub fn save(dir: &Path, store: &VectorStore) -> Result<(), DbError> {
    // Borrowing tuple — serializes without cloning the entry vector.
    let payload = (store.entries(), store.dim());
    let bytes = bincode::serialize(&payload).map_err(|e| DbError::Serde(e.to_string()))?;

    let tmp = dir.join(SNAPSHOT_TMP);
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(&bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(SNAPSHOT_FILE))?;
    Ok(())
}

/// Load the snapshot if one exists.
pub fn load(dir: &Path) -> Result<Option<VectorStore>, DbError> {
    let path = dir.join(SNAPSHOT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    let (entries, dim): (Vec<Entry>, Option<usize>) =
        bincode::deserialize(&bytes).map_err(|e| DbError::Serde(e.to_string()))?;
    Ok(Some(VectorStore::from_parts(entries, dim)))
}
