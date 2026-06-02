//! Pure domain model for scheduling.
//!
//! Defines the [`Job`] aggregate and its [`JobStatus`] state machine, the
//! scheduling value objects ([`Priority`], [`Deadline`], [`ReadyAt`]), the
//! [`SchedulerTime`] timeline value, and the [`DomainError`] type shared across
//! fallible operations. This module has no I/O, concurrency, or external
//! dependencies.

pub mod error;
mod job;
mod priority;
mod time;

pub use error::DomainError;
pub use job::{AttemptNumber, Job, JobId, JobStatus, TargetRef};
pub use priority::{Deadline, Priority, ReadyAt};
pub use time::SchedulerTime;
