use crate::infra::config::app_config::EmailConfig;
use anyhow::{Context, Result};
use lettre::{
    message::{header::ContentType, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};

/// SMTP email client backed by [`lettre`].
///
/// Construct once at startup via [`EmailClient::new`] and share as
/// `Option<Arc<EmailClient>>` in `AppState` and `JobContext`.
///
/// The `tls_mode` config controls the connection strategy:
/// - `"starttls"` (default) — SMTP + STARTTLS upgrade, port 587
/// - `"tls"`                — Implicit TLS (SMTPS), port 465
/// - `"none"`               — Plain SMTP, no encryption (dev/localhost only)
#[derive(Clone)]
pub struct EmailClient {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from_address: String,
    from_name: String,
}

impl EmailClient {
    pub fn new(cfg: &EmailConfig) -> Result<Self> {
        let creds = Credentials::new(cfg.smtp_username.clone(), cfg.smtp_password.clone());

        let mailer = match cfg.tls_mode.as_str() {
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)
                .context("Failed to build TLS SMTP transport")?
                .credentials(creds)
                .port(cfg.smtp_port)
                .build(),
            "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
                .credentials(creds)
                .port(cfg.smtp_port)
                .build(),
            _ => {
                // "starttls" or anything unrecognised — use STARTTLS
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
                    .context("Failed to build STARTTLS SMTP transport")?
                    .credentials(creds)
                    .port(cfg.smtp_port)
                    .build()
            }
        };

        Ok(Self {
            mailer,
            from_address: cfg.from_address.clone(),
            from_name: cfg.from_name.clone(),
        })
    }

    /// Send an email with a plain-text body and an optional HTML alternative.
    ///
    /// When `html` is `Some`, the message is sent as `multipart/alternative`
    /// with both the plain-text and HTML parts. Otherwise only plain text is sent.
    pub async fn send(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: Option<&str>,
    ) -> Result<()> {
        let from = format!("{} <{}>", self.from_name, self.from_address)
            .parse()
            .context("Invalid from address")?;
        let to_addr = to.parse().context("Invalid to address")?;

        let email = if let Some(html_body) = html {
            Message::builder()
                .from(from)
                .to(to_addr)
                .subject(subject)
                .multipart(
                    MultiPart::alternative()
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(text.to_string()),
                        )
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(html_body.to_string()),
                        ),
                )
                .context("Failed to build multipart email")?
        } else {
            Message::builder()
                .from(from)
                .to(to_addr)
                .subject(subject)
                .body(text.to_string())
                .context("Failed to build plain-text email")?
        };

        self.mailer
            .send(email)
            .await
            .context("SMTP send failed")?;

        Ok(())
    }
}
