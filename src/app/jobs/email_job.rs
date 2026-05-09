use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{Job, JobContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Job that sends a transactional email via the configured SMTP client.
///
/// Dispatch via `state.dispatcher`:
/// ```rust,ignore
/// state.dispatcher.dispatch(
///     &EmailJob::new("user@example.com", "Welcome!", "Hello!")
/// ).await?;
/// ```
///
/// Requires `email.enabled = true` in config and valid SMTP credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailJob {
    /// Identifies this struct to the [`JobRegistry`]. Must match `job_type()`.
    #[serde(rename = "type")]
    pub r#type: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub html: Option<String>,
}

impl EmailJob {
    pub fn new(
        to: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            r#type: Self::job_type().to_string(),
            to: to.into(),
            subject: subject.into(),
            body: body.into(),
            html: None,
        }
    }

    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }
}

#[async_trait]
impl Job for EmailJob {
    fn job_type() -> &'static str {
        "email"
    }

    async fn execute(&self, ctx: &JobContext) -> Result<(), QueueError> {
        let client = ctx.email.as_ref().ok_or_else(|| {
            QueueError::Execution(
                "Email client is not configured — set email.enabled = true in config".to_string(),
            )
        })?;

        client
            .send(&self.to, &self.subject, &self.body, self.html.as_deref())
            .await
            .map_err(|e| QueueError::Execution(format!("Email send failed: {e}")))?;

        tracing::info!(
            to = %self.to,
            subject = %self.subject,
            has_html = self.html.is_some(),
            "Email sent successfully"
        );

        Ok(())
    }
}

