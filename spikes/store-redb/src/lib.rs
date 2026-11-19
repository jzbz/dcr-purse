//! Phase 0 spike for decision D2: a redb schema prototype for dcr-purse's
//! own wallet store, measured against dcrwallet's udb on the same workload.
//!
//! Throwaway code: it answers the questions ADR-0003 asks and is not the
//! Phase 2 store. See README.md for how to rerun everything.

#![forbid(unsafe_code)]

/// The redb release the spike links (Cargo.toml pins it exactly).
pub const REDB_VERSION: &str = "5.0.0";

pub mod bench;
pub mod chain;
pub mod export;
pub mod powerloss;
pub mod store;
pub mod workload;
