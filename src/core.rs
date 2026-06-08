//! In-memory vector store with SIMD-accelerated, parallel cosine search.
//!
//! Pure compute + data structures — no I/O. Durability lives in `wal`/`snapshot`
//! and is orchestrated by `db::Database`.
//!
//! Foundational invariants baked into [`Entry`]: every vector carries a
//! `namespace` (multi-tenancy as a field, not a migration) and an
//! `embed_model_id` (so similarity is never silently computed across embedding
//! models — search can filter by it).

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wide::f32x8;

fn default_namespace() -> String {
    "default".to_string()
}

/// Serde adapter: persist `serde_json::Value` as a JSON string.
///
/// `Value` deserializes via `deserialize_any`, which bincode (non-self-
/// describing) does not support. Round-tripping through a string keeps the
/// in-memory API as a rich `Value` while staying bincode-compatible.
mod json_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
        let encoded = serde_json::to_string(v).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        let encoded = String::deserialize(d)?;
        serde_json::from_str(&encoded).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub vector: Vec<f32>,
    #[serde(with = "json_string", default)]
    pub metadata: Value,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub embed_model_id: String,
    /// Reciprocal of the L2 norm, precomputed so search is dot-product + 1 mul.
    /// Recomputed on every upsert; the stored value is authoritative.
    #[serde(default)]
    pub inv_norm: f32,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub id: String,
    pub score: f32,
    pub namespace: String,
    pub embed_model_id: String,
    pub metadata: Value,
}

#[derive(Debug)]
pub enum DbError {
    Dimension { expected: usize, got: usize },
    Io(std::io::Error),
    Serde(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Dimension { expected, got } => {
                write!(f, "vector dimension mismatch: expected {expected}, got {got}")
            }
            DbError::Io(e) => write!(f, "io error: {e}"),
            DbError::Serde(e) => write!(f, "serialization error: {e}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len();
    let lanes = n - (n % 8);
    let mut acc = f32x8::splat(0.0);
    let mut i = 0;
    while i < lanes {
        let av: [f32; 8] = a[i..i + 8].try_into().unwrap();
        let bv: [f32; 8] = b[i..i + 8].try_into().unwrap();
        acc += f32x8::from(av) * f32x8::from(bv);
        i += 8;
    }
    let mut sum = acc.reduce_add();
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}

#[inline]
fn inv_norm(v: &[f32]) -> f32 {
    let norm = dot(v, v).sqrt();
    if norm > 0.0 {
        1.0 / norm
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
struct Scored {
    score: f32,
    idx: usize,
}

impl PartialEq for Scored {
    fn eq(&self, o: &Self) -> bool {
        self.score == o.score
    }
}
impl Eq for Scored {}
impl PartialOrd for Scored {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Scored {
    fn cmp(&self, o: &Self) -> Ordering {
        // total_cmp keeps NaN from poisoning the heap ordering.
        self.score.total_cmp(&o.score)
    }
}

/// Maintain a bounded min-heap of the `k` highest scores seen so far.
#[inline]
fn push_topk(heap: &mut BinaryHeap<Reverse<Scored>>, item: Scored, k: usize) {
    if k == 0 {
        return;
    }
    if heap.len() < k {
        heap.push(Reverse(item));
    } else if let Some(Reverse(min)) = heap.peek() {
        if item.score > min.score {
            heap.pop();
            heap.push(Reverse(item));
        }
    }
}

#[derive(Default)]
pub struct VectorStore {
    entries: Vec<Entry>,
    dim: Option<usize>,
    index: HashMap<String, usize>,
}

impl VectorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn dim(&self) -> Option<usize> {
        self.dim
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Reconstruct a store (and its id index) from snapshotted parts.
    pub fn from_parts(entries: Vec<Entry>, dim: Option<usize>) -> Self {
        let mut index = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            index.insert(e.id.clone(), i);
        }
        Self { entries, dim, index }
    }

    /// Cheap pre-check so callers can reject before durably logging.
    pub fn validate_dim(&self, len: usize) -> Result<(), DbError> {
        if let Some(d) = self.dim {
            if d != len {
                return Err(DbError::Dimension { expected: d, got: len });
            }
        }
        Ok(())
    }

    /// Insert or overwrite by id. Sets store dimensionality on first insert and
    /// rejects mismatched dimensions thereafter.
    pub fn upsert(&mut self, mut entry: Entry) -> Result<(), DbError> {
        match self.dim {
            None => self.dim = Some(entry.vector.len()),
            Some(d) if d != entry.vector.len() => {
                return Err(DbError::Dimension { expected: d, got: entry.vector.len() })
            }
            _ => {}
        }
        entry.inv_norm = inv_norm(&entry.vector);
        if entry.namespace.is_empty() {
            entry.namespace = default_namespace();
        }
        if let Some(&pos) = self.index.get(&entry.id) {
            self.entries[pos] = entry;
        } else {
            let pos = self.entries.len();
            self.index.insert(entry.id.clone(), pos);
            self.entries.push(entry);
        }
        Ok(())
    }

    /// Remove by id (swap-remove + index fixup). Returns whether it existed.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(pos) = self.index.remove(id) {
            let last = self.entries.len() - 1;
            self.entries.swap(pos, last);
            self.entries.pop();
            if pos != last {
                let moved_id = self.entries[pos].id.clone();
                self.index.insert(moved_id, pos);
            }
            true
        } else {
            false
        }
    }

    /// Top-`k` cosine search, optionally filtered by namespace and/or embedding
    /// model. Parallel scan + bounded heaps; results sorted by descending score.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        namespace: Option<&str>,
        embed_model_id: Option<&str>,
    ) -> Result<Vec<SearchHit>, DbError> {
        self.validate_dim(query.len())?;
        if k == 0 || self.entries.is_empty() {
            return Ok(Vec::new());
        }

        let qn = inv_norm(query);
        let qunit: Vec<f32> = query.iter().map(|x| x * qn).collect();

        let heap = self
            .entries
            .par_iter()
            .enumerate()
            .filter(|(_, e)| {
                namespace.map_or(true, |ns| e.namespace == ns)
                    && embed_model_id.map_or(true, |m| e.embed_model_id == m)
            })
            .fold(
                BinaryHeap::<Reverse<Scored>>::new,
                |mut h, (i, e)| {
                    let score = dot(&qunit, &e.vector) * e.inv_norm;
                    push_topk(&mut h, Scored { score, idx: i }, k);
                    h
                },
            )
            .reduce(BinaryHeap::new, |mut a, b| {
                for item in b.into_iter() {
                    push_topk(&mut a, item.0, k);
                }
                a
            });

        let mut hits: Vec<Scored> = heap.into_iter().map(|r| r.0).collect();
        hits.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));

        Ok(hits
            .into_iter()
            .map(|s| {
                let e = &self.entries[s.idx];
                SearchHit {
                    id: e.id.clone(),
                    score: s.score,
                    namespace: e.namespace.clone(),
                    embed_model_id: e.embed_model_id.clone(),
                    metadata: e.metadata.clone(),
                }
            })
            .collect())
    }

    /// Enumerate stored entries (no similarity ranking), optionally filtered by
    /// namespace and/or embedding model, with `offset`/`limit` pagination.
    ///
    /// Iteration order is storage order — stable between calls as long as the
    /// store isn't mutated, which is what pagination needs. (Deletes use
    /// swap-remove, so order is not insertion order across mutations.)
    pub fn scan(
        &self,
        namespace: Option<&str>,
        embed_model_id: Option<&str>,
        limit: Option<usize>,
        offset: usize,
    ) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| {
                namespace.map_or(true, |ns| e.namespace == ns)
                    && embed_model_id.map_or(true, |m| e.embed_model_id == m)
            })
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, v: Vec<f32>, ns: &str) -> Entry {
        Entry {
            id: id.into(),
            vector: v,
            metadata: Value::Null,
            namespace: ns.into(),
            embed_model_id: "m".into(),
            inv_norm: 0.0,
        }
    }

    #[test]
    fn search_orders_by_cosine() {
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0, 0.0], "default")).unwrap();
        s.upsert(e("y", vec![0.9, 0.1, 0.0], "default")).unwrap();
        s.upsert(e("z", vec![0.0, 1.0, 0.0], "default")).unwrap();
        let hits = s.search(&[1.0, 0.0, 0.0], 2, None, None).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "x");
        assert_eq!(hits[1].id, "y");
        assert!(hits[0].score >= hits[1].score);
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dim_enforced() {
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0], "default")).unwrap();
        assert!(s.upsert(e("y", vec![1.0, 0.0, 0.0], "default")).is_err());
        assert!(s.search(&[1.0, 0.0, 0.0], 1, None, None).is_err());
    }

    #[test]
    fn scan_filters_and_paginates() {
        let mut s = VectorStore::new();
        for i in 0..5 {
            s.upsert(e(&format!("a{i}"), vec![1.0, 0.0], "ns-a")).unwrap();
        }
        for i in 0..3 {
            s.upsert(e(&format!("b{i}"), vec![0.0, 1.0], "ns-b")).unwrap();
        }

        // namespace filter
        assert_eq!(s.scan(Some("ns-a"), None, None, 0).len(), 5);
        assert_eq!(s.scan(Some("ns-b"), None, None, 0).len(), 3);
        assert_eq!(s.scan(None, None, None, 0).len(), 8); // all
        assert!(s.scan(Some("missing"), None, None, 0).is_empty());

        // embedding-model filter (helper sets embed_model_id = "m")
        assert_eq!(s.scan(None, Some("m"), None, 0).len(), 8);
        assert!(s.scan(None, Some("other"), None, 0).is_empty());

        // pagination within a namespace
        assert_eq!(s.scan(Some("ns-a"), None, Some(2), 0).len(), 2);
        assert_eq!(s.scan(Some("ns-a"), None, Some(2), 2).len(), 2);
        assert_eq!(s.scan(Some("ns-a"), None, Some(2), 4).len(), 1); // remainder
        assert!(s.scan(Some("ns-a"), None, Some(2), 10).is_empty()); // offset past end

        // entries returned actually match the filter
        assert!(s.scan(Some("ns-a"), None, None, 0).iter().all(|e| e.namespace == "ns-a"));
    }

    #[test]
    fn upsert_overwrites() {
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0], "default")).unwrap();
        s.upsert(e("x", vec![0.0, 1.0], "default")).unwrap();
        assert_eq!(s.len(), 1);
        let hits = s.search(&[0.0, 1.0], 1, None, None).unwrap();
        assert_eq!(hits[0].id, "x");
        assert!(hits[0].score > 0.99);
    }

    #[test]
    fn delete_works() {
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0], "default")).unwrap();
        s.upsert(e("y", vec![0.0, 1.0], "default")).unwrap();
        assert!(s.remove("x"));
        assert_eq!(s.len(), 1);
        assert!(!s.remove("x"));
        let hits = s.search(&[1.0, 0.0], 5, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "y");
    }

    #[test]
    fn namespace_and_model_filter() {
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0], "a")).unwrap();
        s.upsert(e("y", vec![1.0, 0.0], "b")).unwrap();
        let hits = s.search(&[1.0, 0.0], 5, Some("a"), None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "x");
        let hits = s.search(&[1.0, 0.0], 5, None, Some("m")).unwrap();
        assert_eq!(hits.len(), 2);
        let hits = s.search(&[1.0, 0.0], 5, None, Some("other")).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn empty_and_k_zero() {
        let s = VectorStore::new();
        assert!(s.search(&[1.0, 0.0], 5, None, None).unwrap().is_empty());
        let mut s = VectorStore::new();
        s.upsert(e("x", vec![1.0, 0.0], "default")).unwrap();
        assert!(s.search(&[1.0, 0.0], 0, None, None).unwrap().is_empty());
    }

    #[test]
    fn non_multiple_of_eight_dim() {
        // exercises the SIMD remainder path (dim 13)
        let mut s = VectorStore::new();
        let mut a = vec![0.0; 13];
        a[3] = 1.0;
        let mut b = vec![0.0; 13];
        b[7] = 1.0;
        s.upsert(e("a", a.clone(), "default")).unwrap();
        s.upsert(e("b", b, "default")).unwrap();
        let hits = s.search(&a, 1, None, None).unwrap();
        assert_eq!(hits[0].id, "a");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }
}
