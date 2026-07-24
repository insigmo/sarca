//! Outbound SMTP mailer. Soft-fails when SMTP is not configured.

use lettre::{
    AsyncSmtpTransport,
    AsyncTransport,
    Message,
    Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
};

use crate::{
    config::Config,
    errors::{SarcaError, SarcaResult},
};

pub struct Mailer<'c> {
    config: &'c Config,
}

impl<'c> Mailer<'c> {
    pub fn new(config: &'c Config) -> Self {
        Self {
            config,
        }
    }

    pub fn require_configured(&self) -> SarcaResult<()> {
        if self.config.smtp_configured() { Ok(()) } else { Err(SarcaError::MailNotConfigured) }
    }

    /// Send mail if SMTP is configured. Returns `MailNotConfigured` when host is empty.
    pub async fn send(
        &self,
        to_email: &str,
        subject: &str,
        text: &str,
        html: &str,
    ) -> SarcaResult<()> {
        self.require_configured()?;
        let host = self.config.smtp_host.as_deref().expect("checked above");

        let from: Mailbox = self.config.smtp_from.parse().map_err(|e| {
            tracing::error!("invalid SMTP_FROM: {e}");
            SarcaError::Unknown
        })?;
        let to: Mailbox = to_email.parse().map_err(|e| {
            tracing::error!("invalid recipient email: {e}");
            SarcaError::Unknown
        })?;

        let message = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(text.to_owned()))
                    .singlepart(SinglePart::html(html.to_owned())),
            )
            .map_err(|e| {
                tracing::error!("build email failed: {e}");
                SarcaError::Unknown
            })?;

        let transport = self.build_transport(host)?;
        transport.send(message).await.map_err(|e| {
            tracing::error!("SMTP send failed: {e}");
            SarcaError::Unknown
        })?;
        Ok(())
    }

    /// Best-effort send: log failures, never propagate (for register / forgot).
    pub async fn send_soft(&self, to_email: &str, subject: &str, text: &str, html: &str) {
        if let Err(e) = self.send(to_email, subject, text, html).await {
            if !matches!(e, SarcaError::MailNotConfigured) {
                tracing::warn!("mail soft-fail to {to_email}: {e}");
            }
        }
    }

    fn build_transport(&self, host: &str) -> SarcaResult<AsyncSmtpTransport<Tokio1Executor>> {
        let tls = self.config.smtp_tls.to_ascii_lowercase();
        let mut builder = match tls.as_str() {
            "none" => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                    .port(self.config.smtp_port)
            },
            "tls" => {
                AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                    .map_err(|e| {
                        tracing::error!("SMTP relay setup failed: {e}");
                        SarcaError::Unknown
                    })?
                    .port(self.config.smtp_port)
            },
            // starttls (default)
            _ => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                    .map_err(|e| {
                        tracing::error!("SMTP starttls setup failed: {e}");
                        SarcaError::Unknown
                    })?
                    .port(self.config.smtp_port)
            },
        };

        if let (Some(user), Some(pass)) =
            (self.config.smtp_username.as_deref(), self.config.smtp_password.as_deref())
        {
            builder = builder.credentials(Credentials::new(user.to_owned(), pass.to_owned()));
        }

        Ok(builder.build())
    }
}

pub fn verify_email_body(base_url: &str, token: &str) -> (String, String, String) {
    let link = format!("{}/verify?token={}", base_url.trim_end_matches('/'), token);
    let subject = "Verify your Sarca email".to_owned();
    let text = format!("Verify your email by opening this link:\n\n{link}\n");
    let html = format!(
        "<p>Verify your email by opening this link:</p><p><a href=\"{link}\">{link}</a></p>"
    );
    (subject, text, html)
}

pub fn reset_email_body(base_url: &str, token: &str) -> (String, String, String) {
    let link = format!("{}/reset-password?token={}", base_url.trim_end_matches('/'), token);
    let subject = "Reset your Sarca password".to_owned();
    let text = format!("Reset your password by opening this link:\n\n{link}\n");
    let html = format!(
        "<p>Reset your password by opening this link:</p><p><a href=\"{link}\">{link}</a></p>"
    );
    (subject, text, html)
}
