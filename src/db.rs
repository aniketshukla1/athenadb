//! Durable database: ties the in-memory [`VectorStore`] to the WAL and
//! snapshots with crash-consistent semantics.
//!
//! Write path: validate -> append to WAL (fsync) -> mutate memory -> maybe
//! snapshot. Boot path: load latest snapshot -> replay WAL tail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::{DbError, Entry, SearchHit, VectorStore};
use crate::snapshot::{self, SNAPSHOT_FILE};
use crate::wal::{Wal, WalOp, WAL_FILE};

pub const DEFAULT_SNAPSHOT_INTERVAL: usize = 10_000;

/// What a store's files look like right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskState {
    pub wal_bytes: u64,
    pub snapshot_bytes: u64,
    /// Seconds since the Unix epoch, UTC.
    pub snapshot_modified_unix: Option<u64>,
}

impl DiskState {
    fn read(dir: &Path) -> Self {
        let wal = std::fs::metadata(dir.join(WAL_FILE)).ok();
        let snapshot = std::fs::metadata(dir.join(SNAPSHOT_FILE)).ok();
        Self {
            wal_bytes: wal.map_or(0, |m| m.len()),
            snapshot_bytes: snapshot.as_ref().map_or(0, |m| m.len()),
            snapshot_modified_unix: snapshot
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        }
    }
}

/// Vectors sharing a namespace and an embedding model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub namespace: String,
    pub embed_model_id: String,
    pub vectors: usize,
}

#[derive(Clone, Debug)]
pub struct Stats {
    pub vectors: usize,
    pub dimensions: Option<usize>,
    /// Vectors with no direction: they score 0 against every query.
    pub zero_norm_vectors: usize,
    /// Largest group first.
    pub groups: Vec<Group>,
    pub disk: DiskState,
    /// Read-only stores only: the files moved on since this copy was loaded.
    pub disk_changed: bool,
    pub ops_since_snapshot: usize,
    pub snapshot_interval: usize,
    pub read_only: bool,
}

pub struct Database {
    store: VectorStore,
    /// `None` when opened read-only.
    wal: Option<Wal>,
    dir: PathBuf,
    ops_since_snapshot: usize,
    snapshot_interval: usize,
    loaded_from: DiskState,
}

impl Database {
    /// Open (or create) a database rooted at `dir`, recovering any prior state. This
    /// handle is the store's only writer until it's dropped: anyone else who tries
    /// gets [`DbError::Locked`].
    pub fn open<P: AsRef<Path>>(dir: P) -> Result<Self, DbError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        // The log comes with the lock, so take it first: what's on disk can't be trusted
        // while someone else might be writing it.
        let wal = Wal::open(&dir)?;
        let (store, replayed) = Self::load(&dir)?;
        Ok(Self::assemble(store, Some(wal), dir, replayed))
    }

    /// Open an existing store without touching its files, so it's safe to inspect
    /// one that another process is writing to. This is a copy as of now: a torn log
    /// tail is ignored as usual, and a snapshot landing mid-load can make the copy
    /// lag. Open it again to catch up; [`Stats::disk_changed`] says when to.
    pub fn open_read_only<P: AsRef<Path>>(dir: P) -> Result<Self, DbError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::read_dir(&dir)?;
        let (store, replayed) = Self::load(&dir)?;
        Ok(Self::assemble(store, None, dir, replayed))
    }

    fn load(dir: &Path) -> Result<(VectorStore, usize), DbError> {
        let mut store = snapshot::load(dir)?.unwrap_or_default();
        let ops = Wal::replay(dir)?;
        let replayed = ops.len();
        for op in ops {
            match op {
                WalOp::Add(e) => {
                    store.upsert(e)?;
                }
                WalOp::Delete(id) => {
                    store.remove(&id);
                }
            }
        }
        Ok((store, replayed))
    }

    fn assemble(store: VectorStore, wal: Option<Wal>, dir: PathBuf, replayed: usize) -> Self {
        let loaded_from = DiskState::read(&dir);
        Self {
            store,
            wal,
            dir,
            ops_since_snapshot: replayed,
            snapshot_interval: DEFAULT_SNAPSHOT_INTERVAL,
            loaded_from,
        }
    }

    pub fn with_snapshot_interval(mut self, n: usize) -> Self {
        self.snapshot_interval = n;
        self
    }

    /// Durably add (or overwrite) an entry. Dimension is validated *before* the
    /// WAL write so a bad call never pollutes the log.
    pub fn add(&mut self, entry: Entry) -> Result<(), DbError> {
        self.add_batch(vec![entry])
    }

    /// Durably add (or overwrite) many entries behind one fsync. Every dimension is
    /// checked first, so one bad vector rejects the whole batch before anything is
    /// logged. A crash mid-write keeps a prefix of the batch.
    pub fn add_batch(&mut self, entries: Vec<Entry>) -> Result<(), DbError> {
        let wal = self.wal.as_mut().ok_or(DbError::ReadOnly)?;
        let mut dim = self.store.dim();
        for e in &entries {
            match dim {
                None => dim = Some(e.vector.len()),
                Some(d) if d != e.vector.len() => {
                    return Err(DbError::Dimension { expected: d, got: e.vector.len() })
                }
                _ => {}
            }
        }
        let count = entries.len();
        let ops: Vec<WalOp> = entries.into_iter().map(WalOp::Add).collect();
        wal.append_batch(&ops)?;
        for op in ops {
            if let WalOp::Add(e) = op {
                self.store.upsert(e)?;
            }
        }
        self.after_ops(count)
    }

    /// Durably delete by id; returns whether it existed.
    pub fn delete(&mut self, id: &str) -> Result<bool, DbError> {
        let wal = self.wal.as_mut().ok_or(DbError::ReadOnly)?;
        wal.append(&WalOp::Delete(id.to_string()))?;
        let removed = self.store.remove(id);
        self.after_ops(1)?;
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

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.store.get(id)
    }

    /// Every entry, in storage order.
    pub fn entries(&self) -> &[Entry] {
        self.store.entries()
    }

    pub fn dimensions(&self) -> Option<usize> {
        self.store.dim()
    }

    /// Force a snapshot now and truncate the WAL.
    pub fn snapshot(&mut self) -> Result<(), DbError> {
        let wal = self.wal.as_mut().ok_or(DbError::ReadOnly)?;
        snapshot::save(&self.dir, &self.store)?;
        wal.truncate()?;
        self.ops_since_snapshot = 0;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn is_read_only(&self) -> bool {
        self.wal.is_none()
    }

    /// A summary of the store. Walks every entry, so don't call it per query.
    pub fn stats(&self) -> Stats {
        let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
        let mut zero_norm_vectors = 0;
        for e in self.store.entries() {
            *counts.entry((&e.namespace, &e.embed_model_id)).or_default() += 1;
            if e.inv_norm == 0.0 {
                zero_norm_vectors += 1;
            }
        }
        let mut groups: Vec<Group> = counts
            .into_iter()
            .map(|((namespace, model), vectors)| Group {
                namespace: namespace.to_string(),
                embed_model_id: model.to_string(),
                vectors,
            })
            .collect();
        groups.sort_by(|a, b| {
            b.vectors
                .cmp(&a.vectors)
                .then_with(|| a.namespace.cmp(&b.namespace))
                .then_with(|| a.embed_model_id.cmp(&b.embed_model_id))
        });
        let disk = DiskState::read(&self.dir);
        Stats {
            vectors: self.store.len(),
            dimensions: self.store.dim(),
            zero_norm_vectors,
            groups,
            disk,
            disk_changed: self.is_read_only() && disk != self.loaded_from,
            ops_since_snapshot: self.ops_since_snapshot,
            snapshot_interval: self.snapshot_interval,
            read_only: self.is_read_only(),
        }
    }

    fn after_ops(&mut self, count: usize) -> Result<(), DbError> {
        self.ops_since_snapshot += count;
        if self.snapshot_interval > 0 && self.ops_since_snapshot >= self.snapshot_interval {
            self.snapshot()?;
        }
        Ok(())
    }
}
