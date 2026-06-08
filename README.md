# AthenaDB

A high-performance custom vector database written in Rust. SIMD-accelerated
cosine search, crash-consistent persistence, and a **dual-mode** architecture
that shares one math core:

1. **Native Python module** (PyO3) — `import athenadb` for zero-copy,
   in-process vector search.
2. **HTTP service** (Axum) — a standalone microservice other systems can query.

AthenaDB began as the memory engine for the Athena agent swarm and is now
developed and released as an independent project.

---

## Features

- **SIMD cosine similarity** via `wide::f32x8`, with precomputed reciprocal
  norms so each comparison is a dot product plus one multiply.
- **Parallel search** with `rayon` and bounded per-thread heaps for top-k.
- **Namespaces** on every vector (multi-tenancy as a field, not a migration).
- **Embedding-model tagging** — every vector records its `embed_model_id`, and
  search can filter by it so similarity is never silently computed across
  incompatible embedding spaces.
- **Durability** — an append-only, fsync'd write-ahead log plus atomic
  snapshots. A torn trailing WAL frame (the `kill -9` mid-write signature) is
  detected and ignored on recovery.
- **Dual-mode, one core** — PyO3 module and Axum server are Cargo features over
  the same `core.rs`.

## Performance

1,000,000 vectors at 768 dimensions: mean top-10 query ≈ **40 ms** on an
M-series Mac (`cargo test --release --test bench -- --ignored --nocapture`).

---

## Layout

| Path | Role |
|------|------|
| `src/core.rs` | `VectorStore`, `Entry`, SIMD cosine, parallel top-k |
| `src/wal.rs` | Append-only write-ahead log |
| `src/snapshot.rs` | Atomic point-in-time snapshots |
| `src/db.rs` | `Database` — durable store tying core + WAL + snapshot |
| `src/python.rs` | PyO3 bindings (`--features python`) |
| `src/bin/server.rs` | Axum HTTP service (`--features server`) |

## Build & test

```bash
# Rust core + persistence tests
cargo test

# Large benchmark (≈3 GB RAM, ignored by default)
cargo test --release --test bench -- --ignored --nocapture

# Build the Python module into the active virtualenv
maturin develop --release

# Run the HTTP service scaffold
cargo run --bin server --features server
```

## Python usage

```python
import athenadb

db = athenadb.AthenaDB("./data")
db.add(
    ids=["a", "b"],
    vectors=[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    namespace="default",
    embed_model_id="bge-small",
)
hits = db.query([1.0, 0.0, 0.0], k=10)   # [(id, score, namespace, embed_model_id, metadata_json), ...]

# Enumerate stored entries (no ranking) with namespace filter + pagination.
# Returns (id, namespace, embed_model_id, metadata_json) tuples.
rows = db.scan(namespace="default", limit=100, offset=0)

db.snapshot()
```

## Status & roadmap

Core engine, durability, and enumeration (`scan`) are complete and tested.
Planned enhancements: an ANN index (HNSW/IVF) for sublinear search,
group-commit WAL for higher write throughput, the HTTP endpoints, and a
re-embedding pipeline keyed off `embed_model_id`.

## License

TODO — add a license before public release.
