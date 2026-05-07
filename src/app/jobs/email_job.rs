use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{Job, JobContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Job that sends an email.
///
/// In this boilerplate the `execute` method logs the email details.
/// Replace the body with calls to your email provider SDK
/// (e.g. SendGrid, Resend, AWS SES).
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

    async fn execute(&self, _ctx: &JobContext) -> Result<(), QueueError> {
        tracing::info!(
            to = %self.to,
            subject = %self.subject,
            has_html = self.html.is_some(),
            "Sending email (placeholder — wire your email provider here)"
        );

        // TODO: integrate a real email provider, e.g.
        // let client = resend::Client::new(&_ctx.config.email.api_key);
        // client.send(...).await?;

        Ok(())
    }
}
