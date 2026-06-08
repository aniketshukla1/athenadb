//! Durable database: ties the in-memory [`VectorStore`] to the WAL and
//! snapshots with crash-consistent semantics.
//!
//! Write path: validate -> append to WAL (fsync) -> mutate memory -> maybe
//! snapshot. Boot path: load latest snapshot -> replay WAL tail.

use std::path::{Path, PathBuf};

use crate::core::{DbError, Entry, SearchHit, VectorStore};
use crate::snapshot;
use crate::wal::{Wal, WalOp};

pub const DEFAULT_SNAPSHOT_INTERVAL: usize = 10_000;

pub struct Database {
    store: VectorStore,
    wal: Wal,
    dir: PathBuf,
    ops_since_snapshot: usize,
    snapshot_interval: usize,
}

impl Database {
    /// Open (or create) a database rooted at `dir`, recovering any prior state.
    pub fn open<P: AsRef<Path>>(dir: P) -> Result<Self, DbError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let mut store = snapshot::load(&dir)?.unwrap_or_default();
        for op in Wal::replay(&dir)? {
            match op {
                WalOp::Add(e) => {
                    store.upsert(e)?;
                }
                WalOp::Delete(id) => {
                    store.remove(&id);
                }
            }
        }

        let wal = Wal::open(&dir)?;
        Ok(Self {
            store,
            wal,
            dir,
            ops_since_snapshot: 0,
            snapshot_interval: DEFAULT_SNAPSHOT_INTERVAL,
        })
    }

    pub fn with_snapshot_interval(mut self, n: usize) -> Self {
        self.snapshot_interval = n;
        self
    }

    /// Durably add (or overwrite) an entry. Dimension is validated *before* the
    /// WAL write so a bad call never pollutes the log.
    pub fn add(&mut self, entry: Entry) -> Result<(), DbError> {
        self.store.validate_dim(entry.vector.len())?;
        self.wal.append(&WalOp::Add(entry.clone()))?;
        self.store.upsert(entry)?;
        self.after_op()
    }

    /// Durably delete by id; returns whether it existed.
    pub fn delete(&mut self, id: &str) -> Result<bool, DbError> {
        self.wal.append(&WalOp::Delete(id.to_string()))?;
        let removed = self.store.remove(id);
        self.after_op()?;
        Ok(removed)
    }

    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        namespace: Option<&str>,
        embed_model_id: Option<&str>,
    ) -> Result<Vec<SearchHit>, DbError> {
        self.store.search(query, k, namespace, embed_model_id)
    }

    /// Enumerate stored entries (no ranking), filtered + paginated. See
    /// [`VectorStore::scan`].
    pub fn scan(
        &self,
        namespace: Option<&str>,
        embed_model_id: Option<&str>,
        limit: Option<usize>,
        offset: usize,
    ) -> Vec<&Entry> {
        self.store.scan(namespace, embed_model_id, limit, offset)
    }

    /// Force a snapshot now and truncate the WAL.
    pub fn snapshot(&mut self) -> Result<(), DbError> {
        snapshot::save(&self.dir, &self.store)?;
        self.wal.truncate()?;
        self.ops_since_snapshot = 0;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    fn after_op(&mut self) -> Result<(), DbError> {
        self.ops_since_snapshot += 1;
        if self.snapshot_interval > 0 && self.ops_since_snapshot >= self.snapshot_interval {
            self.snapshot()?;
        }
        Ok(())
    }
}
