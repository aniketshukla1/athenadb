//! AthenaDB — the Athena swarm's custom vector store.
//!
//! Dual-mode by design (sharing one core):
//! - **PyO3 native module** (`--features python`) — zero-copy in-process access
//!   for the Python swarm. See [`python`].
//! - **Axum HTTP service** (`--features server`, `src/bin/server.rs`) — for
//!   multi-service deployments. Scaffolded; endpoints land in Phase 5.

pub mod core;
pub mod db;
pub mod snapshot;
pub mod wal;

#[cfg(feature = "python")]
mod python;
