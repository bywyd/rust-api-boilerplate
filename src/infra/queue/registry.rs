use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{Job, JobContext};
use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// A type-erased async function that deserialises a payload and runs a job.
pub type JobHandlerFn = Arc<
    dyn Fn(Value, Arc<JobContext>) -> BoxFuture<'static, Result<(), QueueError>>
        + Send
        + Sync,
>;

/// Central registry that maps job type strings to their handler functions.
///
/// Register every job type once at startup via [`JobRegistry::register`].
#[derive(Clone, Default)]
pub struct JobRegistry {
    handlers: HashMap<String, JobHandlerFn>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a [`Job`] implementation.
    ///
    /// The concrete type must be `DeserializeOwned` so the registry can
    /// deserialise incoming payloads automatically.
    pub fn register<J>(&mut self)
    where
        J: Job + DeserializeOwned + 'static,
    {
        let handler: JobHandlerFn = Arc::new(|payload, ctx| {
            Box::pin(async move {
                let job: J = serde_json::from_value(payload)?;
                job.execute(&ctx).await
            })
        });
        self.handlers.insert(J::job_type().to_string(), handler);
    }

    /// Look up the handler for a given job type.
    pub fn get(&self, job_type: &str) -> Option<&JobHandlerFn> {
        self.handlers.get(job_type)
    }
}
