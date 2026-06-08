//! PyO3 bindings: `import athenadb` gives the Python swarm a native, durable
//! vector store with near-zero call overhead.
//!
//! Metadata crosses the FFI boundary as JSON strings (Python `json.dumps` /
//! `json.loads`) to keep the binding dependency-free and predictable. The
//! Phase 2 `MemoryManager` wraps this in a friendlier dict-based API.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use crate::core::{DbError, Entry};
use crate::db::Database;

fn to_py(e: DbError) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn parse_meta(s: Option<&String>) -> PyResult<serde_json::Value> {
    match s {
        None => Ok(serde_json::Value::Null),
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| PyValueError::new_err(format!("invalid metadata JSON: {e}"))),
    }
}

#[pyclass]
pub struct AthenaDB {
    inner: Database,
}

#[pymethods]
impl AthenaDB {
    /// Open or create a database at `path`, recovering prior state.
    #[new]
    fn new(path: String) -> PyResult<Self> {
        Database::open(path)
            .map(|inner| Self { inner })
            .map_err(to_py)
    }

    /// Add (or overwrite) vectors. `metadatas` is an optional list of JSON
    /// strings, one per id.
    #[pyo3(signature = (ids, vectors, metadatas=None, namespace=None, embed_model_id=None))]
    fn add(
        &mut self,
        ids: Vec<String>,
        vectors: Vec<Vec<f32>>,
        metadatas: Option<Vec<String>>,
        namespace: Option<String>,
        embed_model_id: Option<String>,
    ) -> PyResult<()> {
        if vectors.len() != ids.len() {
            return Err(PyValueError::new_err("ids and vectors length mismatch"));
        }
        if let Some(m) = &metadatas {
            if m.len() != ids.len() {
                return Err(PyValueError::new_err("ids and metadatas length mismatch"));
            }
        }
        let ns = namespace.unwrap_or_else(|| "default".to_string());
        let model = embed_model_id.unwrap_or_default();

        for (i, id) in ids.into_iter().enumerate() {
            let metadata = parse_meta(metadatas.as_ref().map(|m| &m[i]))?;
            let entry = Entry {
                id,
                vector: vectors[i].clone(),
                metadata,
                namespace: ns.clone(),
                embed_model_id: model.clone(),
                inv_norm: 0.0,
            };
            self.inner.add(entry).map_err(to_py)?;
        }
        Ok(())
    }

    /// Top-`k` cosine search. Returns a list of
    /// `(id, score, namespace, embed_model_id, metadata_json)` tuples.
    #[pyo3(signature = (vector, k=10, namespace=None, embed_model_id=None))]
    fn query(
        &self,
        vector: Vec<f32>,
        k: usize,
        namespace: Option<String>,
        embed_model_id: Option<String>,
    ) -> PyResult<Vec<(String, f32, String, String, String)>> {
        let hits = self
            .inner
            .search(&vector, k, namespace.as_deref(), embed_model_id.as_deref())
            .map_err(to_py)?;
        Ok(hits
            .into_iter()
            .map(|h| {
                let meta = serde_json::to_string(&h.metadata).unwrap_or_else(|_| "null".into());
                (h.id, h.score, h.namespace, h.embed_model_id, meta)
            })
            .collect())
    }

    /// Compatibility shim for `MemoryManager`: same as [`add`], with embeddings
    /// passed positionally.
    #[pyo3(signature = (ids, embeddings, metadatas=None, namespace=None, embed_model_id=None))]
    fn add_documents(
        &mut self,
        ids: Vec<String>,
        embeddings: Vec<Vec<f32>>,
        metadatas: Option<Vec<String>>,
        namespace: Option<String>,
        embed_model_id: Option<String>,
    ) -> PyResult<()> {
        self.add(ids, embeddings, metadatas, namespace, embed_model_id)
    }

    /// Delete by id; returns the number actually removed.
    fn delete(&mut self, ids: Vec<String>) -> PyResult<usize> {
        let mut removed = 0;
        for id in ids {
            if self.inner.delete(&id).map_err(to_py)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Force a snapshot and truncate the WAL.
    fn snapshot(&mut self) -> PyResult<()> {
        self.inner.snapshot().map_err(to_py)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

#[pymodule]
fn athenadb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<AthenaDB>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
