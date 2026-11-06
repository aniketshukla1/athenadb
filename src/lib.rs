//! AthenaDB — the Athena swarm's custom vector store.
//!
//! Dual-mode by design (sharing one core):
//! - **PyO3 native module** (`--features python`): in-process access from Python.
//! - **HTTP API and dashboard** (`--features server`, `src/bin/athenadb.rs`): inspect
//!   a store, or run it as a service. See [`server`].

pub mod core;
pub mod db;
pub mod projection;
#[cfg(feature = "server")]
pub mod server;
pub mod snapshot;
pub mod wal;

#[cfg(feature = "python")]
mod python;
