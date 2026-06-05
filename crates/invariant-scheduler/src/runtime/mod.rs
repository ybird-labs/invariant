//! The execution layer: sibling to the pure [`domain`](crate::domain), this is
//! where scheduling decisions are carried out.

mod admissions;
mod job_runner;
mod slot_supplier;
