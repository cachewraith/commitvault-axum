//! Outgoing email (sign-in codes and password reset codes).
//!
//! Configured through `SMTP_*`. Without SMTP the mailer is disabled: the
//! second sign-in step is skipped and password reset by email is unavailable.
//! Message bodies contain one-time codes, so they are never logged.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};

use crate::config::{Config, SmtpTls};

const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingEmail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone)]
pub enum Mailer {
    Smtp {
        transport: AsyncSmtpTransport<Tokio1Executor>,
        from: Mailbox,
    },
    /// No SMTP configured: nothing can be sent.
    Disabled,
    /// Keeps messages in memory (integration tests).
    Memory(Arc<Mutex<Vec<OutgoingEmail>>>),
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Smtp { .. } => "Mailer::Smtp",
            Self::Disabled => "Mailer::Disabled",
            Self::Memory(_) => "Mailer::Memory",
        })
    }
}

impl Mailer {
    pub fn from_config(config: &Config) -> anyhow::Result<Self> {
        let Some(smtp) = &config.smtp else {
            return Ok(Self::Disabled);
        };
        let builder = match smtp.tls {
            SmtpTls::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host).context("invalid SMTP_HOST")?
            }
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host).context("invalid SMTP_HOST")?,
            SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host),
        };
        let mut builder = builder.port(smtp.port).timeout(Some(SMTP_TIMEOUT));
        if let (Some(username), Some(password)) = (&smtp.username, &smtp.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.expose().to_owned()));
        }
        let from: Mailbox = smtp.from.parse().context("SMTP_FROM is not a valid sender address")?;
        Ok(Self::Smtp { transport: builder.build(), from })
    }

    /// A mailer that records messages instead of sending them.
    pub fn memory() -> (Self, Arc<Mutex<Vec<OutgoingEmail>>>) {
        let outbox = Arc::new(Mutex::new(Vec::new()));
        (Self::Memory(Arc::clone(&outbox)), outbox)
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub async fn send(&self, email: OutgoingEmail) -> anyhow::Result<()> {
        match self {
            Self::Smtp { transport, from } => {
                let message = Message::builder()
                    .from(from.clone())
                    .to(email.to.parse().context("recipient is not a valid email address")?)
                    .subject(email.subject)
                    .header(ContentType::TEXT_PLAIN)
                    .body(email.body)
                    .context("failed to build the email")?;
                transport.send(message).await.context("SMTP delivery failed")?;
                Ok(())
            }
            Self::Disabled => anyhow::bail!("outgoing email is not configured"),
            Self::Memory(outbox) => {
                outbox.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(email);
                Ok(())
            }
        }
    }
}
