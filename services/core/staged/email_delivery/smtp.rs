use super::{MailError, Result, SmtpOutcome, crypto::read_private};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use std::{net::IpAddr, path::PathBuf, time::Duration};
use zeroize::Zeroizing;

pub enum SmtpTls {
    Starttls,
    Implicit,
    PlaintextLoopbackTest,
}

/// No Debug: credentials and deployment paths are not diagnostics.
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub from: String,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password_file: Option<PathBuf>,
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

pub(crate) fn mailbox(raw: &str) -> Result<Mailbox> {
    // Same bare-mailbox boundary as humangrant/mail.rs, using locked lettre.
    if raw.is_empty()
        || raw.len() > 254
        || !raw.is_ascii()
        || raw
            .bytes()
            .any(|b| b <= 32 || b >= 127 || b"<>(),;:\\\"".contains(&b))
    {
        return Err(MailError::Invalid);
    }
    let address = raw
        .parse::<lettre::Address>()
        .map_err(|_| MailError::Invalid)?;
    if address.user().len() > 64 {
        return Err(MailError::Invalid);
    }
    Ok(Mailbox::new(None, address))
}

/// Only fixed templates and hex proofs; no arbitrary subject/body/password.
pub(crate) struct MailPayload {
    pub recipient: Zeroizing<String>,
    pub purpose: Zeroizing<String>,
    pub code: Zeroizing<String>,
    pub bind_sid: Zeroizing<String>,
}

impl MailPayload {
    pub(crate) fn validate(&self, kind: &str) -> Result<()> {
        mailbox(&self.recipient)?;
        if !matches!(
            self.purpose.as_str(),
            "register" | "bind" | "password_reset"
        ) {
            return Err(MailError::Invalid);
        }
        match kind {
            "challenge" => {
                let length = if *self.purpose == "password_reset" {
                    32
                } else {
                    6
                };
                if self.code.len() != length
                    || !self
                        .code
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(MailError::Invalid);
                }
                if *self.purpose == "bind" {
                    if self.bind_sid.len() != 64
                        || !self.bind_sid.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(MailError::Invalid);
                    }
                } else if !self.bind_sid.is_empty() {
                    return Err(MailError::Invalid);
                }
            }
            "security_notice"
                if self.code.is_empty()
                    && self.bind_sid.is_empty()
                    && *self.purpose != "register" => {}
            _ => return Err(MailError::Invalid),
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::new());
        for field in [&self.recipient, &self.purpose, &self.code, &self.bind_sid] {
            bytes.extend_from_slice(&(field.len() as u16).to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes
    }

    pub(crate) fn decode(mut bytes: &[u8], kind: &str) -> Result<Self> {
        fn take(bytes: &mut &[u8]) -> Result<Zeroizing<String>> {
            let size = bytes.get(..2).ok_or(MailError::Unavailable)?;
            let size = u16::from_be_bytes([size[0], size[1]]) as usize;
            let field = bytes.get(2..2 + size).ok_or(MailError::Unavailable)?;
            let text = std::str::from_utf8(field).map_err(|_| MailError::Unavailable)?;
            let value = Zeroizing::new(text.to_owned());
            *bytes = &bytes[2 + size..];
            Ok(value)
        }
        let payload = Self {
            recipient: take(&mut bytes)?,
            purpose: take(&mut bytes)?,
            code: take(&mut bytes)?,
            bind_sid: take(&mut bytes)?,
        };
        if !bytes.is_empty() {
            return Err(MailError::Unavailable);
        }
        payload.validate(kind)?;
        Ok(payload)
    }
}

impl SmtpMailer {
    /// Constructs only; does not connect or send a readiness message.
    pub fn open(config: &SmtpConfig) -> Result<Self> {
        if config.port == 0
            || config.host.is_empty()
            || config.host.len() > 253
            || config
                .host
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b".-:".contains(&b))
        {
            return Err(MailError::Invalid);
        }
        let from = mailbox(&config.from)?;
        let mut builder = match config.tls {
            SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                .map_err(|_| MailError::Invalid)?,
            SmtpTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|_| MailError::Invalid)?,
            SmtpTls::PlaintextLoopbackTest => {
                if !config
                    .host
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
                    || config.username.is_some()
                    || config.password_file.is_some()
                {
                    return Err(MailError::Invalid);
                }
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
        }
        .port(config.port)
        .timeout(Some(Duration::from_secs(15)));
        match (&config.username, &config.password_file) {
            (None, None) => {}
            (Some(username), Some(path)) if !username.is_empty() => {
                let bytes = read_private(path, 4096)?;
                let secret = std::str::from_utf8(&bytes).map_err(|_| MailError::Invalid)?;
                let secret = secret.trim_end_matches(['\r', '\n']);
                if secret.is_empty() || secret.contains(['\r', '\n', '\0']) {
                    return Err(MailError::Invalid);
                }
                builder =
                    builder.credentials(Credentials::new(username.clone(), secret.to_owned()));
            }
            _ => return Err(MailError::Invalid),
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }

    pub(crate) async fn send(
        &self,
        id: &str,
        kind: &str,
        payload: &MailPayload,
    ) -> Result<SmtpOutcome> {
        payload.validate(kind)?;
        let body = Zeroizing::new(if kind == "challenge" {
            format!(
                "Your LMM {} verification code is: {}\r\nThis code expires with your request. Ignore this message if you did not request it.\r\n",
                payload.purpose.as_str(),
                payload.code.as_str()
            )
        } else {
            format!(
                "Your LMM {} security change completed. If you did not request this change, use your established recovery channel.\r\nNo password or verification code is included in this notice.\r\n",
                payload.purpose.as_str()
            )
        });
        let message = Message::builder()
            .from(self.from.clone())
            .to(mailbox(&payload.recipient)?)
            .message_id(Some(format!("<{id}@email.lmm.invalid>")))
            .subject(if kind == "challenge" {
                "LMM email verification"
            } else {
                "LMM security notice"
            })
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|_| MailError::Invalid)?;
        let bytes = Zeroizing::new(message.formatted());
        // send_raw avoids retaining a second formatted buffer across the await.
        // Lettre 0.11.23's tokio transport returns the DATA result; abort/QUIT
        // errors do not replace it. Never format its error/source or response.
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            self.transport.send_raw(message.envelope(), &bytes),
        )
        .await;
        Ok(match outcome {
            Ok(Ok(response)) if response.code().to_string() == "250" => SmtpOutcome::Accepted,
            Ok(Err(error)) if error.is_transient() => SmtpOutcome::RejectedTemporary,
            Ok(Err(error)) if error.is_permanent() => SmtpOutcome::RejectedPermanent,
            _ => SmtpOutcome::Unknown,
        })
    }
}
