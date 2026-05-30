//! invariant-coordinator — GLOBAL durable-ownership / coordination layer (WIP).
//!
//! Parked here during the local-scheduler milestone: leases, fencing epochs, and
//! the lease-laden `Job` aggregate. The global layer is not yet fully designed.
//! `invariant-scheduler` (local) does NOT depend on this crate — work flows down.
pub mod domain;
