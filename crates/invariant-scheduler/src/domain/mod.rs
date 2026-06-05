//! Pure domain model for scheduling.
//!
//! Defines the [`Job`] aggregate and its [`JobStatus`] state machine, the
//! scheduling value objects ([`Priority`], [`Deadline`], [`ReadyAt`]), the
//! [`SchedulerTime`] timeline value, and the [`DomainError`] type shared across
//! fallible operations. The [`JobOrder`] trait is the pluggable seam for ordering
//! ready jobs, with [`Fifo`] and [`ByPriority`] as its built-in implementations.
//! This module has no I/O, concurrency, or external dependencies.

pub mod error;
mod job;
mod job_order;
mod priority;
mod ready_queue;
mod time;

pub use error::DomainError;
pub use job::{AttemptNumber, Job, JobId, JobStatus, TargetRef};
pub use job_order::{ByPriority, Fifo, JobOrder};
pub use priority::{Deadline, Priority, ReadyAt};
pub use ready_queue::{QueuedJob, ReadyQueue};
pub use time::SchedulerTime;
