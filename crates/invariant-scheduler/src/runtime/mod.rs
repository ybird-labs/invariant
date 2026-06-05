//! The execution layer: sibling to the pure [`domain`](crate::domain), this is
//! where scheduling decisions are carried out.

mod admissions;
mod job_runner;
mod scheduler_core;
mod slot_supplier;
