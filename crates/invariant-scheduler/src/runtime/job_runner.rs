//! The job-execution port: how a pulled job is run and what it reports.

// TODO: remove once the runtime loop drives a `JobRunner`.
#![allow(dead_code)]

use crate::domain::Job;

/// What a [`JobRunner`] reports when a job finishes executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The job ran to completion successfully.
    Completed,
    /// The job ran but did not succeed.
    Failed,
}

/// Executes a pulled job and reports its [`Outcome`].
///
/// The returned future is `Send` so the scheduler loop can spawn it onto a
/// task set.
pub(crate) trait JobRunner {
    fn run(&self, job: Job) -> impl Future<Output = Outcome> + Send;
}
