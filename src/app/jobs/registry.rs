use crate::app::jobs::email_job::EmailJob;
use crate::infra::queue::registry::JobRegistry;

/// Build a [`JobRegistry`] with all application-level jobs registered.
///
/// Add new job types here as the application grows.
pub fn build_registry() -> JobRegistry {
    let mut registry = JobRegistry::new();
    registry.register::<EmailJob>();
    registry
}
