//! Cron-based job scheduler.
//!
//! Register jobs against cron expressions in a [`CronRegistry`]; the
//! [`Scheduler`] runner evaluates each schedule and dispatches the job onto the
//! normal queue when due, so scheduled jobs reuse the existing retry, backend
//! and worker machinery.
pub mod registry;
pub mod runner;

pub use registry::CronRegistry;
pub use runner::Scheduler;
